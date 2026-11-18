use crate::{
    clock::{Clock, format_minute, parse_minute},
    damage::{self, Damage},
    db::{Db, insert_default_wage_levels},
    error::{ApiError, ApiResult},
    levels::{WageLevel, standing, wage_levels},
    payroll,
    presence::{self, HeartbeatReq},
    profiles,
    purchases::{self, Budget},
    reconcile::{client_time, correct_late, kind_label, last_point, plausible_time, window_sum},
    reversal, version,
};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use axum::{
    Json, Router,
    extract::{FromRef, FromRequestParts, Path, Query, State},
    http::{HeaderMap, header::AUTHORIZATION, request::Parts},
    response::Html,
    routing::{get, post, put},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Zustand der Anwendung: Datenbank und Uhr. Handler holen sich per `State<Db>` oder
/// `State<Clock>` nur, was sie brauchen.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub clock: Clock,
}

impl FromRef<AppState> for Db {
    fn from_ref(s: &AppState) -> Db {
        s.db.clone()
    }
}

impl FromRef<AppState> for Clock {
    fn from_ref(s: &AppState) -> Clock {
        s.clock.clone()
    }
}

/// Router mit der echten Uhr.
pub fn router(db: Db) -> Router {
    router_with(db, Clock::system())
}

/// Router mit eigener Uhr, für Tests mit gestellter Zeit.
pub fn router_with(db: Db, clock: Clock) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/api/health", get(health))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/me/password", post(change_password))
        .route("/api/companies", post(create_company))
        .route("/api/companies/join", post(join_company))
        .route(
            "/api/companies/{id}",
            get(get_company).patch(update_company),
        )
        .route(
            "/api/companies/{id}/trucks",
            get(list_trucks).post(add_truck),
        )
        .route("/api/companies/{id}/wage-levels", put(set_wage_levels))
        .route("/api/companies/{id}/events", post(post_event))
        .route("/api/companies/{id}/profile", put(set_profile))
        .route(
            "/api/companies/{id}/rejected-events",
            get(list_rejected_events),
        )
        .route("/api/companies/{id}/heartbeat", post(heartbeat))
        .route("/api/companies/{id}/ledger", get(get_ledger))
        .route("/api/companies/{id}/ledger/reverse", post(reverse_entries))
        .route(
            "/api/companies/{id}/ledger/{entry}/link-truck",
            post(link_truck).delete(unlink_truck),
        )
        .route(
            "/api/companies/{id}/purchase-candidates",
            get(purchase_candidates),
        )
        .route(
            "/api/companies/{id}/budgets",
            get(list_budgets).post(create_budget),
        )
        .route(
            "/api/companies/{id}/budgets/{budget}/close",
            post(close_budget),
        )
        .route("/api/companies/{id}/transfer", post(transfer))
        .route("/api/companies/{id}/payrolls", get(list_payrolls))
        .route("/api/companies/{id}/payrolls/run", post(run_payroll_now))
        .route("/api/companies/{id}/sync", post(sync))
        .route(
            "/api/companies/{id}/sync/{sync_id}/written",
            post(sync_written),
        )
        .route("/api/companies/{id}/unexplained", get(list_unexplained))
        .route(
            "/api/companies/{id}/unexplained/{entry}/close",
            post(close_unexplained),
        )
        .route("/api/trucks/{id}/checkout", post(checkout))
        .route("/api/trucks/{id}/checkin", post(checkin))
        .route("/api/trucks/{id}/lease", get(get_lease))
        .route("/api/trucks/{id}/repair", post(repair))
        .route("/api/trucks/{id}/retire", post(retire))
        .route("/api/trucks/{id}/sell", post(sell))
        .with_state(AppState { db, clock })
}

// ---------------------------------------------------------------- Hilfen

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(n: usize) -> String {
    let bytes: [u8; 32] = rand::random();
    hex(&bytes[..n])
}

/// Tokens liegen nur als Hash in der Datenbank.
fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

/// Rolle des Nutzers in der Firma. 404, wenn es die Firma nicht gibt, 403, wenn er nicht dazugehört.
fn role_in(conn: &Connection, company_id: i64, user_id: i64) -> ApiResult<String> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM companies WHERE id = ?1",
            [company_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(ApiError::not_found("Diese Firma gibt es nicht"));
    }
    conn.query_row(
        "SELECT role FROM memberships WHERE company_id = ?1 AND user_id = ?2",
        [company_id, user_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| ApiError::forbidden("Du gehörst nicht zu dieser Firma"))
}

fn require_chef(conn: &Connection, company_id: i64, user_id: i64) -> ApiResult<()> {
    if role_in(conn, company_id, user_id)? == "chef" {
        Ok(())
    } else {
        Err(ApiError::forbidden("Das darf nur der Chef"))
    }
}

fn company_balance(conn: &Connection, company_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(amount), 0) FROM ledger WHERE company_id = ?1 AND account = 'firma'",
        [company_id],
        |r| r.get(0),
    )
}

fn wallet(conn: &Connection, company_id: i64, user_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(amount), 0) FROM ledger WHERE company_id = ?1 AND account = 'fahrer' AND user_id = ?2",
        [company_id, user_id],
        |r| r.get(0),
    )
}

/// Einstellungen der Firma für Schaden und Werkstatt.
#[derive(Debug, Clone, Copy)]
struct DamageSettings {
    /// Schadensgrenze in Prozent
    limit: i64,
    /// Euro je Prozentpunkt Schaden
    rate: i64,
    /// Haftungsanteil des Fahrers in Prozent
    liability_percent: i64,
}

fn damage_settings(conn: &Connection, company_id: i64) -> rusqlite::Result<DamageSettings> {
    conn.query_row(
        "SELECT damage_limit, repair_rate, liability_percent FROM companies WHERE id = ?1",
        [company_id],
        |r| {
            Ok(DamageSettings {
                limit: r.get(0)?,
                rate: r.get(1)?,
                liability_percent: r.get(2)?,
            })
        },
    )
}

// ---------------------------------------------------------------- Anmeldung

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Nach so vielen Sekunden ohne Benutzung läuft ein Token ab: 30 Tage.
pub const TOKEN_MAX_IDLE: i64 = 30 * 86_400;

/// `last_used` wird höchstens einmal je so vielen Sekunden geschrieben.
const TOKEN_TOUCH_SECS: i64 = 60;

pub struct AuthUser {
    pub id: i64,
    pub name: String,
    /// Hash des benutzten Tokens, zum Abmelden und Passwortwechsel
    pub token_hash: String,
}

impl<S: Send + Sync> FromRequestParts<S> for AuthUser
where
    Db: FromRef<S>,
    Clock: FromRef<S>,
{
    type Rejection = ApiError;

    /// Prüft den Token. War er länger als [`TOKEN_MAX_IDLE`] nicht in Benutzung, wird er
    /// gelöscht und die Anfrage mit 401 abgelehnt. Sonst zählt diese Anfrage als Benutzung.
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let token = bearer(&parts.headers).ok_or_else(ApiError::unauthorized)?;
        let token_hash = hash_token(token);
        let now = Clock::from_ref(state).now();
        let db = Db::from_ref(state);
        let conn = db.lock().unwrap();
        let (id, name, last_used): (i64, String, i64) = conn
            .query_row(
                "SELECT u.id, u.name, COALESCE(t.last_used, t.created_at)
                 FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?1",
                [&token_hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(ApiError::unauthorized)?;
        if now - last_used > TOKEN_MAX_IDLE {
            conn.execute("DELETE FROM tokens WHERE token_hash = ?1", [&token_hash])?;
            return Err(ApiError(
                axum::http::StatusCode::UNAUTHORIZED,
                "Die Anmeldung ist abgelaufen, bitte neu anmelden".into(),
            ));
        }
        if now - last_used >= TOKEN_TOUCH_SECS {
            conn.execute(
                "UPDATE tokens SET last_used = ?1 WHERE token_hash = ?2",
                params![now, token_hash],
            )?;
        }
        Ok(AuthUser {
            id,
            name,
            token_hash,
        })
    }
}

#[derive(Deserialize)]
struct Credentials {
    name: String,
    password: String,
}

fn issue_token(conn: &Connection, user_id: i64, now: i64) -> rusqlite::Result<String> {
    let token = random_hex(32);
    conn.execute(
        "INSERT INTO tokens (token_hash, user_id, created_at, last_used) VALUES (?1, ?2, ?3, ?3)",
        params![hash_token(&token), user_id, now],
    )?;
    Ok(token)
}

fn hash_password(password: &str) -> ApiResult<String> {
    Ok(Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|_| ApiError::bad("Das Passwort konnte nicht verarbeitet werden"))?
        .to_string())
}

/// Versionsprüfung beim Anmelden: Felder `server_version` und `compatible` für die Antwort.
/// Ohne Header `X-SpedHub-Version` (Dashboard, ältere Launcher) gibt es nichts zu vergleichen,
/// dann gilt `compatible: true`. Passt die Version nicht, steht eine Warnung im Log.
fn version_fields(headers: &HeaderMap, name: &str) -> (String, bool) {
    let client = headers
        .get(version::HEADER)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).trim().to_string());
    let compatible = match &client {
        None => true,
        Some(v) => version::compatible(v, version::VERSION),
    };
    if !compatible {
        eprintln!(
            "Warnung: {name} meldet sich mit Version {} an, der Server hat {}. Die Versionen passen nicht zusammen",
            client.as_deref().unwrap_or_default(),
            version::VERSION
        );
    }
    (version::VERSION.to_string(), compatible)
}

async fn register(
    State(db): State<Db>,
    State(clock): State<Clock>,
    headers: HeaderMap,
    Json(c): Json<Credentials>,
) -> ApiResult<Json<Value>> {
    let name = c.name.trim();
    if name.len() < 3 || name.len() > 32 {
        return Err(ApiError::bad("Der Name braucht 3 bis 32 Zeichen"));
    }
    if c.password.len() < 8 {
        return Err(ApiError::bad("Das Passwort braucht mindestens 8 Zeichen"));
    }
    let pw_hash = hash_password(&c.password)?;
    let now = clock.now();
    let conn = db.lock().unwrap();
    match conn.execute(
        "INSERT INTO users (name, pw_hash, created_at) VALUES (?1, ?2, ?3)",
        params![name, pw_hash, now],
    ) {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::conflict("Der Name ist schon vergeben"));
        }
        Err(e) => return Err(e.into()),
    }
    let user_id = conn.last_insert_rowid();
    let token = issue_token(&conn, user_id, now)?;
    let (server_version, compatible) = version_fields(&headers, name);
    Ok(Json(json!({
        "user_id": user_id,
        "name": name,
        "token": token,
        "server_version": server_version,
        "compatible": compatible,
    })))
}

async fn login(
    State(db): State<Db>,
    State(clock): State<Clock>,
    headers: HeaderMap,
    Json(c): Json<Credentials>,
) -> ApiResult<Json<Value>> {
    let wrong = || {
        ApiError(
            axum::http::StatusCode::UNAUTHORIZED,
            "Name oder Passwort stimmt nicht".into(),
        )
    };
    let conn = db.lock().unwrap();
    let (user_id, name, pw_hash): (i64, String, String) = conn
        .query_row(
            "SELECT id, name, pw_hash FROM users WHERE name = ?1",
            [c.name.trim()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(wrong)?;
    Argon2::default()
        .verify_password(c.password.as_bytes(), pw_hash.as_str())
        .map_err(|_| wrong())?;
    let token = issue_token(&conn, user_id, clock.now())?;
    let (server_version, compatible) = version_fields(&headers, &name);
    Ok(Json(json!({
        "user_id": user_id,
        "name": name,
        "token": token,
        "server_version": server_version,
        "compatible": compatible,
    })))
}

/// Abmelden: Der benutzte Token wird ungültig.
async fn logout(State(db): State<Db>, user: AuthUser) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM tokens WHERE token_hash = ?1",
        [&user.token_hash],
    )?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct PasswordReq {
    old_password: String,
    new_password: String,
}

/// Passwort ändern. Alle anderen Tokens des Nutzers werden ungültig, der benutzte bleibt.
/// Ein falsches altes Passwort gibt 403, nicht 401: Die Anmeldung selbst ist ja gültig.
async fn change_password(
    State(db): State<Db>,
    user: AuthUser,
    Json(b): Json<PasswordReq>,
) -> ApiResult<Json<Value>> {
    if b.new_password.len() < 8 {
        return Err(ApiError::bad(
            "Das neue Passwort braucht mindestens 8 Zeichen",
        ));
    }
    let pw_hash: String = {
        let conn = db.lock().unwrap();
        conn.query_row("SELECT pw_hash FROM users WHERE id = ?1", [user.id], |r| {
            r.get(0)
        })?
    };
    // Argon2 dauert, deshalb ohne gesperrte Datenbank
    Argon2::default()
        .verify_password(b.old_password.as_bytes(), pw_hash.as_str())
        .map_err(|_| ApiError::forbidden("Das alte Passwort stimmt nicht"))?;
    let new_hash = hash_password(&b.new_password)?;
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    // Hat sich das Passwort inzwischen geändert, gilt die Prüfung oben nicht mehr
    let changed = tx.execute(
        "UPDATE users SET pw_hash = ?1 WHERE id = ?2 AND pw_hash = ?3",
        params![new_hash, user.id, pw_hash],
    )?;
    if changed == 0 {
        return Err(ApiError::conflict(
            "Das Passwort wurde gerade schon geändert, bitte noch einmal versuchen",
        ));
    }
    let removed = tx.execute(
        "DELETE FROM tokens WHERE user_id = ?1 AND token_hash != ?2",
        params![user.id, user.token_hash],
    )?;
    tx.commit()?;
    Ok(Json(json!({ "ok": true, "other_tokens_removed": removed })))
}

async fn me(State(db): State<Db>, user: AuthUser) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, m.role FROM memberships m JOIN companies c ON c.id = m.company_id
         WHERE m.user_id = ?1 ORDER BY c.name",
    )?;
    let companies = stmt
        .query_map([user.id], |r| {
            Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "role": r.get::<_, String>(2)? }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(
        json!({ "user_id": user.id, "name": user.name, "companies": companies }),
    ))
}

// ---------------------------------------------------------------- Firma

#[derive(Deserialize)]
struct NewCompany {
    name: String,
}

async fn create_company(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Json(b): Json<NewCompany>,
) -> ApiResult<Json<Value>> {
    let name = b.name.trim();
    if name.len() < 3 || name.len() > 48 {
        return Err(ApiError::bad("Der Firmenname braucht 3 bis 48 Zeichen"));
    }
    let invite = random_hex(4).to_uppercase();
    let t = clock.now();
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    match tx.execute(
        "INSERT INTO companies (name, invite_code, created_at, payroll_from) VALUES (?1, ?2, ?3, ?3)",
        params![name, invite, t],
    ) {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(ApiError::conflict("Eine Firma mit dem Namen gibt es schon")),
        Err(e) => return Err(e.into()),
    }
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO memberships (company_id, user_id, role, joined_at) VALUES (?1, ?2, 'chef', ?3)",
        params![id, user.id, t],
    )?;
    insert_default_wage_levels(&tx, id)?;
    tx.commit()?;
    Ok(Json(
        json!({ "id": id, "name": name, "invite_code": invite }),
    ))
}

#[derive(Deserialize)]
struct JoinReq {
    invite_code: String,
}

async fn join_company(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Json(b): Json<JoinReq>,
) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    let (id, name): (i64, String) = conn
        .query_row(
            "SELECT id, name FROM companies WHERE invite_code = ?1",
            [b.invite_code.trim().to_uppercase()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Der Einladungscode passt zu keiner Firma"))?;
    match conn.execute(
        "INSERT INTO memberships (company_id, user_id, role, joined_at) VALUES (?1, ?2, 'fahrer', ?3)",
        params![id, user.id, clock.now()],
    ) {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(ApiError::conflict("Du bist schon in dieser Firma")),
        Err(e) => return Err(e.into()),
    }
    Ok(Json(
        json!({ "company_id": id, "name": name, "role": "fahrer" }),
    ))
}

fn wage_levels_json(levels: &[WageLevel]) -> Value {
    json!(
        levels
            .iter()
            .map(|w| json!({
                "stage": w.position, "min_level": w.min_level, "min_km": w.min_km,
                "cents_per_km": w.cents_per_km
            }))
            .collect::<Vec<_>>()
    )
}

async fn get_company(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let now = clock.now();
    let conn = db.lock().unwrap();
    let role = role_in(&conn, id, user.id)?;
    let (name, invite, factor, flat, minute, advance): (String, String, f64, i64, i64, i64) = conn
        .query_row(
            "SELECT name, invite_code, revenue_factor, daily_flat, payroll_minute, advance
             FROM companies WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?;
    let levels = wage_levels(&conn, id)?;
    let ds = damage_settings(&conn, id)?;
    type Person = (i64, String, String, Option<String>);
    let people: Vec<Person> = conn
        .prepare(
            "SELECT u.id, u.name, m.role, m.profile_name FROM memberships m
             JOIN users u ON u.id = m.user_id
             WHERE m.company_id = ?1 ORDER BY m.role, u.name",
        )?
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let seen = presence::all(&conn, id)?;
    let mut members = Vec::new();
    for (uid, uname, urole, profile_name) in people {
        let s = standing(&conn, &levels, id, uid)?;
        let mut m = json!({
            "user_id": uid, "name": uname, "role": urole,
            "wallet": wallet(&conn, id, uid)?,
            "open_claim": payroll::open_claim(&conn, id, uid)?,
            "xp": s.xp, "game_level": s.game_level, "company_km": s.company_km,
            "stage": s.stage.map(|w| w.position), "cents_per_km": s.cents_per_km(),
            "online": false, "last_seen": Value::Null
        });
        // Das gemeldete Firmenprofil: der Chef sieht alle, ein Fahrer nur seins
        if role == "chef" || uid == user.id {
            m["profile_name"] = json!(profile_name);
        }
        if let Some(p) = seen.get(&uid) {
            m["last_seen"] = json!(p.last_seen);
            if p.online(now) {
                m["online"] = json!(true);
                m["plate"] = json!(p.plate);
                m["near_city"] = p.near_city();
                m["speed_kmh"] = p.speed_kmh();
                m["job"] = p.job();
                // Der eigene geliehene Lkw. Einen fremden Firmen-Lkw sieht nur der Chef.
                m["truck_id"] = json!(p.truck_id.filter(|_| p.own_lease));
                if role == "chef"
                    && !p.own_lease
                    && let Some(tid) = p.truck_id
                {
                    let plate: String =
                        conn.query_row("SELECT plate FROM trucks WHERE id = ?1", [tid], |r| {
                            r.get(0)
                        })?;
                    m["foreign_truck"] = json!({ "id": tid, "plate": plate });
                }
            }
        }
        members.push(m);
    }
    let mut out = json!({
        "id": id, "name": name, "your_role": role,
        "balance": company_balance(&conn, id)?, "members": members,
        "revenue_factor": factor, "daily_flat": flat, "payroll_time": format_minute(minute),
        "advance": advance, "wage_levels": wage_levels_json(&levels),
        "damage_limit": ds.limit, "repair_rate": ds.rate, "liability_percent": ds.liability_percent
    });
    // Einladungscode und ungeklärte Beträge sieht nur der Chef.
    if role == "chef" {
        out["invite_code"] = json!(invite);
        let open: i64 = conn.query_row(
            "SELECT COUNT(*) FROM unexplained WHERE company_id = ?1 AND closed_at IS NULL",
            [id],
            |r| r.get(0),
        )?;
        out["open_unexplained"] = json!(open);
        // Das offene Anschaffungsbudget mit Restbetrag, sonst null
        out["budget"] = purchases::open_budget(&conn, id)?
            .as_ref()
            .map_or(Value::Null, Budget::json);
    }
    Ok(Json(out))
}

/// Einstellungen der Firma. Alles optional, nur Mitgeschicktes ändert sich.
#[derive(Deserialize)]
struct CompanyPatch {
    revenue_factor: Option<f64>,
    daily_flat: Option<i64>,
    /// Uhrzeit der täglichen Abrechnung wie `04:00`, Europe/Berlin
    payroll_time: Option<String>,
    advance: Option<i64>,
    /// Schadensgrenze in Prozent, 0 bis 100
    damage_limit: Option<i64>,
    /// Werkstatt: Euro je Prozentpunkt Schaden
    repair_rate: Option<i64>,
    /// Haftungsanteil des Fahrers in Prozent, 0 bis 100
    liability_percent: Option<i64>,
}

async fn update_company(
    State(db): State<Db>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(b): Json<CompanyPatch>,
) -> ApiResult<Json<Value>> {
    if let Some(f) = b.revenue_factor
        && !(f.is_finite() && f > 0.0 && f <= 10_000.0)
    {
        return Err(ApiError::bad(
            "Der Erlösfaktor muss größer als 0 und höchstens 10000 sein",
        ));
    }
    if let Some(f) = b.daily_flat
        && !(0..=100_000).contains(&f)
    {
        return Err(ApiError::bad(
            "Die Tagespauschale muss zwischen 0 und 100000 Euro liegen",
        ));
    }
    if let Some(a) = b.advance
        && !(0..=10_000_000).contains(&a)
    {
        return Err(ApiError::bad(
            "Der Betriebsvorschuss muss zwischen 0 und 10000000 Euro liegen",
        ));
    }
    if let Some(l) = b.damage_limit
        && !(0..=100).contains(&l)
    {
        return Err(ApiError::bad(
            "Die Schadensgrenze muss zwischen 0 und 100 Prozent liegen",
        ));
    }
    if let Some(r) = b.repair_rate
        && !(0..=1_000_000).contains(&r)
    {
        return Err(ApiError::bad(
            "Der Satz der Werkstatt muss zwischen 0 und 1000000 Euro je Prozentpunkt liegen",
        ));
    }
    if let Some(p) = b.liability_percent
        && !(0..=100).contains(&p)
    {
        return Err(ApiError::bad(
            "Die Haftung muss zwischen 0 und 100 Prozent liegen",
        ));
    }
    let minute =
        match &b.payroll_time {
            None => None,
            Some(t) => Some(parse_minute(t).ok_or_else(|| {
                ApiError::bad("Die Uhrzeit der Abrechnung muss wie 04:00 aussehen")
            })?),
        };
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    require_chef(&tx, id, user.id)?;
    if let Some(f) = b.revenue_factor {
        tx.execute(
            "UPDATE companies SET revenue_factor = ?1 WHERE id = ?2",
            params![f, id],
        )?;
    }
    if let Some(f) = b.daily_flat {
        tx.execute(
            "UPDATE companies SET daily_flat = ?1 WHERE id = ?2",
            [f, id],
        )?;
    }
    if let Some(m) = minute {
        tx.execute(
            "UPDATE companies SET payroll_minute = ?1 WHERE id = ?2",
            [m, id],
        )?;
    }
    if let Some(a) = b.advance {
        tx.execute("UPDATE companies SET advance = ?1 WHERE id = ?2", [a, id])?;
    }
    for (column, value) in [
        ("damage_limit", b.damage_limit),
        ("repair_rate", b.repair_rate),
        ("liability_percent", b.liability_percent),
    ] {
        if let Some(v) = value {
            tx.execute(
                &format!("UPDATE companies SET {column} = ?1 WHERE id = ?2"),
                [v, id],
            )?;
        }
    }
    let ds = damage_settings(&tx, id)?;
    let (factor, flat, minute, advance): (f64, i64, i64, i64) = tx.query_row(
        "SELECT revenue_factor, daily_flat, payroll_minute, advance FROM companies WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    tx.commit()?;
    Ok(Json(json!({
        "id": id, "revenue_factor": factor, "daily_flat": flat,
        "payroll_time": format_minute(minute), "advance": advance,
        "damage_limit": ds.limit, "repair_rate": ds.rate, "liability_percent": ds.liability_percent
    })))
}

#[derive(Deserialize)]
struct LevelReq {
    min_level: i64,
    min_km: i64,
    cents_per_km: i64,
}

#[derive(Deserialize)]
struct LevelsReq {
    levels: Vec<LevelReq>,
}

/// Lohnstufen ersetzen. Die Reihenfolge ist die Rangfolge: Ein Fahrer bekommt die letzte Stufe,
/// deren Bedingungen er erfüllt.
async fn set_wage_levels(
    State(db): State<Db>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(b): Json<LevelsReq>,
) -> ApiResult<Json<Value>> {
    if b.levels.is_empty() || b.levels.len() > 20 {
        return Err(ApiError::bad("Es braucht 1 bis 20 Lohnstufen"));
    }
    for l in &b.levels {
        if l.min_level < 0 || l.min_level > 1000 || l.min_km < 0 || l.min_km > 100_000_000 {
            return Err(ApiError::bad(
                "Level und Kilometer einer Stufe dürfen nicht negativ sein",
            ));
        }
        if !(0..=100_000).contains(&l.cents_per_km) {
            return Err(ApiError::bad(
                "Der Lohn je Kilometer muss zwischen 0 und 100000 Cent liegen",
            ));
        }
    }
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    require_chef(&tx, id, user.id)?;
    tx.execute("DELETE FROM wage_levels WHERE company_id = ?1", [id])?;
    for (i, l) in b.levels.iter().enumerate() {
        tx.execute(
            "INSERT INTO wage_levels (company_id, position, min_level, min_km, cents_per_km)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, i as i64 + 1, l.min_level, l.min_km, l.cents_per_km],
        )?;
    }
    let levels = wage_levels(&tx, id)?;
    tx.commit()?;
    Ok(Json(json!({ "wage_levels": wage_levels_json(&levels) })))
}

// ---------------------------------------------------------------- Fuhrpark

#[derive(Deserialize)]
struct NewTruck {
    plate: String,
    model: String,
    #[serde(default)]
    package: String,
    #[serde(default)]
    odometer_km: i64,
    state: Option<Value>,
}

async fn add_truck(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(b): Json<NewTruck>,
) -> ApiResult<Json<Value>> {
    let plate = b.plate.trim();
    let model = b.model.trim();
    if plate.is_empty() || model.is_empty() {
        return Err(ApiError::bad(
            "Kennzeichen und Modell dürfen nicht leer sein",
        ));
    }
    if b.odometer_km < 0 {
        return Err(ApiError::bad("Der Kilometerstand darf nicht negativ sein"));
    }
    let state = b.state.unwrap_or_else(|| json!({})).to_string();
    let now = clock.now();
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    require_chef(&tx, company_id, user.id)?;
    // Ein ausgemusterter oder verkaufter Lkw gibt sein Kennzeichen ab
    free_retired_plate(&tx, company_id, plate, now)?;
    match tx.execute(
        "INSERT INTO trucks (company_id, plate, model, package, odometer_km, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            company_id,
            plate,
            model,
            b.package,
            b.odometer_km,
            state,
            now
        ],
    ) {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::conflict(
                "Das Kennzeichen gibt es in der Firma schon",
            ));
        }
        Err(e) => return Err(e.into()),
    }
    let id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(Json(json!({
        "id": id, "plate": plate, "model": model,
        "odometer_km": b.odometer_km, "rev": 1
    })))
}

/// Gibt das Kennzeichen `plate` frei, wenn es gerade ein ausgemusterter oder verkaufter Lkw
/// der Firma trägt. Kennzeichen sind je Firma eindeutig, auch über ausgemusterte Lkw. Damit
/// ein neuer Lkw (oder ein Lkw, dem das Spiel bei der Rückgabe ein neues gegeben hat) es
/// bekommen kann, wandert es beim alten Lkw zu den früheren Kennzeichen, und der alte heißt
/// danach `<Kennzeichen> (Lkw <id>)`. Seine Buchungen und Notizen bleiben unverändert. Trägt
/// ein aktiver Lkw das Kennzeichen, passiert nichts, der Aufrufer meldet dann 409.
fn free_retired_plate(
    conn: &Connection,
    company_id: i64,
    plate: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let retired: Option<i64> = conn
        .query_row(
            "SELECT id FROM trucks
             WHERE company_id = ?1 AND plate = ?2 AND retired_at IS NOT NULL",
            params![company_id, plate],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = retired {
        conn.execute(
            "INSERT INTO truck_plates (truck_id, plate, replaced_at) VALUES (?1, ?2, ?3)",
            params![id, plate, now],
        )?;
        conn.execute(
            "UPDATE trucks SET plate = ?1 WHERE id = ?2",
            params![format!("{plate} (Lkw {id})"), id],
        )?;
    }
    Ok(())
}

async fn list_trucks(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let now = clock.now();
    let conn = db.lock().unwrap();
    let role = role_in(&conn, company_id, user.id)?;
    let ds = damage_settings(&conn, company_id)?;
    // Wer gerade online ist und welchen Firmen-Lkw er meldet
    let seen = presence::all(&conn, company_id)?;
    let names: std::collections::HashMap<i64, String> = conn
        .prepare(
            "SELECT u.id, u.name FROM memberships m JOIN users u ON u.id = m.user_id
             WHERE m.company_id = ?1",
        )?
        .query_map([company_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut stmt = conn.prepare(
        "SELECT t.id, t.plate, t.model, t.odometer_km, t.rev, u.id, u.name, l.taken_at,
                t.last_seen_at, t.last_odometer_km, t.state, t.in_workshop,
                t.retired_at, t.retired_reason, t.sale_price
         FROM trucks t
         LEFT JOIN leases l ON l.truck_id = t.id AND l.returned_at IS NULL
         LEFT JOIN users u ON u.id = l.user_id
         WHERE t.company_id = ?1 ORDER BY t.retired_at IS NOT NULL, t.plate",
    )?;
    // Frühere Kennzeichen je Lkw, älteste zuerst
    let mut old: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    let mut plates = conn.prepare(
        "SELECT p.truck_id, p.plate FROM truck_plates p JOIN trucks t ON t.id = p.truck_id
         WHERE t.company_id = ?1 ORDER BY p.replaced_at, p.rowid",
    )?;
    for row in plates.query_map([company_id], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })? {
        let (id, plate) = row?;
        old.entry(id).or_default().push(plate);
    }
    let trucks = stmt
        .query_map([company_id], |r| {
            let holder: Option<i64> = r.get(5)?;
            let id: i64 = r.get(0)?;
            let last_seen_at: Option<i64> = r.get(8)?;
            let retired_at: Option<i64> = r.get(12)?;
            // Schaden aus dem gespeicherten Zustand, also dem bei der letzten Rückgabe
            let d = damage::damage_of(&r.get::<_, String>(10)?);
            // Der Ausleiher ist online und meldet genau diesen Lkw
            let driving = holder
                .and_then(|h| seen.get(&h))
                .filter(|p| p.online(now) && p.own_lease && p.truck_id == Some(id));
            let mut t = json!({
                "id": id, "plate": r.get::<_, String>(1)?, "model": r.get::<_, String>(2)?,
                "old_plates": old.get(&id).cloned().unwrap_or_default(),
                "odometer_km": r.get::<_, i64>(3)?, "rev": r.get::<_, i64>(4)?,
                "leased_by": holder.map(|id| json!({
                    "user_id": id,
                    "name": r.get::<_, String>(6).unwrap_or_default(),
                    "since": r.get::<_, i64>(7).unwrap_or_default()
                })),
                "driving": driving.is_some(),
                "last_known": last_seen_at.map(|at| json!({
                    "at": at, "odometer_km": r.get::<_, Option<i64>>(9).ok().flatten()
                })),
                "damage": d.fixable, "damage_unfixable": d.unfixable,
                "in_workshop": r.get::<_, i64>(11)? != 0,
                "repair_cost": damage::repair_cost(d.fixable, ds.rate),
                // Ausgemustert oder verkauft: nicht mehr ausleihbar, bleibt in der Geschichte
                "retired": match retired_at {
                    Some(at) => json!({
                        "reason": r.get::<_, Option<String>>(13)?,
                        "at": at,
                        "sale_price": r.get::<_, Option<i64>>(14)?
                    }),
                    None => Value::Null,
                }
            });
            if let Some(p) = driving {
                t["speed_kmh"] = p.speed_kmh();
                t["near_city"] = p.near_city();
            }
            // Wer den Lkw fährt, ohne ihn geliehen zu haben, sieht nur der Chef
            if role == "chef" {
                let others: Vec<Value> = seen
                    .values()
                    .filter(|p| p.online(now) && !p.own_lease && p.truck_id == Some(id))
                    .map(|p| {
                        json!({
                            "user_id": p.user_id,
                            "name": names.get(&p.user_id).cloned().unwrap_or_default(),
                            "speed_kmh": p.speed_kmh()
                        })
                    })
                    .collect();
                t["foreign_drivers"] = json!(others);
            }
            Ok(t)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!({ "trucks": trucks })))
}

/// Schaden aus dem gespeicherten Zustand eines Lkw.
fn stored_damage(conn: &Connection, truck_id: i64) -> rusqlite::Result<Damage> {
    let state: String =
        conn.query_row("SELECT state FROM trucks WHERE id = ?1", [truck_id], |r| {
            r.get(0)
        })?;
    Ok(damage::damage_of(&state))
}

/// Bauplan, Zustand und Kennzeichen eines Lkw, wie sie beim Ausleihen herausgehen.
fn truck_package(conn: &Connection, truck_id: i64) -> rusqlite::Result<Value> {
    conn.query_row(
        "SELECT plate, model, package, odometer_km, state, rev FROM trucks WHERE id = ?1",
        [truck_id],
        |r| {
            let state: String = r.get(4)?;
            Ok(json!({
                "id": truck_id, "plate": r.get::<_, String>(0)?, "model": r.get::<_, String>(1)?,
                "package": r.get::<_, String>(2)?, "odometer_km": r.get::<_, i64>(3)?,
                "rev": r.get::<_, i64>(5)?,
                "state": serde_json::from_str::<Value>(&state).unwrap_or_else(|_| json!({}))
            }))
        },
    )
}

/// Lkw ausleihen. Geht nur, wenn er frei ist. Liefert Bauplan und Zustand für den Spielstand.
async fn checkout(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(truck_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    type Row = (i64, i64, String, bool, Option<String>);
    let (company_id, rev, state, in_workshop, retired): Row = tx
        .query_row(
            "SELECT company_id, rev, state, in_workshop, retired_reason FROM trucks WHERE id = ?1",
            [truck_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, i64>(3)? != 0,
                    r.get(4)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diesen Lkw gibt es nicht"))?;
    role_in(&tx, company_id, user.id)?;
    if let Some(reason) = retired {
        return Err(retired_error(&reason, "lässt sich nicht mehr ausleihen"));
    }
    let holder: Option<String> = tx
        .query_row(
            "SELECT u.name FROM leases l JOIN users u ON u.id = l.user_id
             WHERE l.truck_id = ?1 AND l.returned_at IS NULL",
            [truck_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(name) = holder {
        return Err(ApiError::conflict(&format!(
            "Der Lkw ist gerade an {name} verliehen"
        )));
    }
    if in_workshop {
        return Err(ApiError::conflict(
            "Der Lkw steht in der Werkstatt. Ausleihen geht erst, wenn der Chef ihn reparieren lässt",
        ));
    }
    // Schaden beim Ausleihen: Was danach dazukommt, ist neuer Schaden dieser Leihe
    let damage_out = damage::damage_of(&state).fixable;
    tx.execute(
        "INSERT INTO leases (truck_id, user_id, rev_out, taken_at, damage_out)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![truck_id, user.id, rev, clock.now(), damage_out],
    )?;
    let lease_id = tx.last_insert_rowid();
    let truck = truck_package(&tx, truck_id)?;
    tx.commit()?;
    Ok(Json(json!({ "lease_id": lease_id, "truck": truck })))
}

/// Bauplan einer laufenden Leihe noch einmal holen, zum Beispiel auf einem anderen Rechner.
/// Darf der Fahrer, der den Lkw hat, und der Chef der Firma. Alle anderen bekommen 403.
async fn get_lease(
    State(db): State<Db>,
    user: AuthUser,
    Path(truck_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    let company_id: i64 = conn
        .query_row(
            "SELECT company_id FROM trucks WHERE id = ?1",
            [truck_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diesen Lkw gibt es nicht"))?;
    let role = role_in(&conn, company_id, user.id)?;
    let (lease_id, holder, taken_at): (i64, i64, i64) = conn
        .query_row(
            "SELECT id, user_id, taken_at FROM leases WHERE truck_id = ?1 AND returned_at IS NULL",
            [truck_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::conflict("Der Lkw ist gar nicht verliehen"))?;
    if holder != user.id && role != "chef" {
        return Err(ApiError::forbidden(
            "Den Lkw hat jemand anderes ausgeliehen",
        ));
    }
    Ok(Json(json!({
        "lease_id": lease_id, "user_id": holder, "taken_at": taken_at,
        "truck": truck_package(&conn, truck_id)?
    })))
}

#[derive(Deserialize)]
struct CheckinReq {
    /// Fehlt er, nimmt der Chef den Lkw mit dem letzten bekannten Zustand zurück
    #[serde(default)]
    odometer_km: Option<i64>,
    state: Option<Value>,
    /// Aktuelles Kennzeichen im Spielstand, falls das Spiel es ersetzt hat
    #[serde(default)]
    plate: Option<String>,
}

/// Lkw zurückgeben. Der Zustand aus dem Spielstand wird übernommen, die Revision steigt.
/// Mit `plate` bekommt der Lkw ein neues Kennzeichen, das alte kommt in `truck_plates`.
///
/// Ohne `odometer_km` (Fahrer abgestürzt, kein Spielstand) darf nur der Chef zurücknehmen.
/// Dann gilt der letzte bekannte Zustand aus dem Herzschlag. Gibt es keinen, 409. Nach jeder
/// Rückgabe ist der letzte bekannte Zustand geleert, damit er nicht beim nächsten Fahrer gilt.
async fn checkin(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(truck_id): Path<i64>,
    Json(b): Json<CheckinReq>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    type TruckRow = (i64, i64, String, Option<i64>, Option<i64>, Option<String>);
    let (company_id, odometer, current_plate, last_seen_at, last_km, last_state): TruckRow = tx
        .query_row(
            "SELECT company_id, odometer_km, plate, last_seen_at, last_odometer_km, last_state
             FROM trucks WHERE id = ?1",
            [truck_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diesen Lkw gibt es nicht"))?;
    let role = role_in(&tx, company_id, user.id)?;
    let (lease_id, holder, damage_out): (i64, i64, Option<f64>) = tx
        .query_row(
            "SELECT id, user_id, damage_out FROM leases WHERE truck_id = ?1 AND returned_at IS NULL",
            [truck_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::conflict("Der Lkw ist gar nicht verliehen"))?;
    // Leihen von vor der Werkstatt: Der gespeicherte Zustand ist noch der vom Ausleihen
    let damage_out = match damage_out {
        Some(d) => d,
        None => stored_damage(&tx, truck_id)?.fixable,
    };
    // Zurückgeben darf, wer ihn hat. Der Chef darf ihn zur Not für andere zurücknehmen.
    if holder != user.id && role != "chef" {
        return Err(ApiError::forbidden(
            "Den Lkw hat jemand anderes ausgeliehen",
        ));
    }
    // Ohne Kilometerstand: letzter bekannter Zustand aus dem Herzschlag
    let (new_odometer, state, from_last) = match b.odometer_km {
        Some(km) => {
            if km < odometer {
                return Err(ApiError::bad(
                    "Der Kilometerstand kann nicht kleiner werden",
                ));
            }
            (km, b.state.clone(), None)
        }
        None if role != "chef" => {
            return Err(ApiError::bad(
                "Der Kilometerstand fehlt. Ohne ihn kann nur der Chef einen Lkw zurücknehmen",
            ));
        }
        None => {
            let at = last_seen_at.ok_or_else(|| {
                ApiError::conflict(
                    "Für diesen Lkw gibt es keinen letzten bekannten Zustand. Bitte den Kilometerstand angeben",
                )
            })?;
            let state = last_state
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok());
            (last_km.unwrap_or(odometer).max(odometer), state, Some(at))
        }
    };
    let new_plate = match b.plate.as_deref().map(str::trim) {
        None => None,
        Some("") => return Err(ApiError::bad("Das Kennzeichen darf nicht leer sein")),
        Some(p) if p.chars().count() > 32 => {
            return Err(ApiError::bad(
                "Das Kennzeichen darf höchstens 32 Zeichen haben",
            ));
        }
        Some(p) if p == current_plate => None,
        Some(p) => Some(p.to_string()),
    };
    if let Some(p) = &new_plate {
        free_retired_plate(&tx, company_id, p, clock.now())?;
        match tx.execute(
            "UPDATE trucks SET plate = ?1 WHERE id = ?2",
            params![p, truck_id],
        ) {
            Ok(_) => {}
            Err(e) if is_unique_violation(&e) => {
                return Err(ApiError::conflict(&format!(
                    "Das Kennzeichen {p} hat in der Firma schon ein anderer Lkw"
                )));
            }
            Err(e) => return Err(e.into()),
        }
        tx.execute(
            "INSERT INTO truck_plates (truck_id, plate, replaced_at) VALUES (?1, ?2, ?3)",
            params![truck_id, current_plate, clock.now()],
        )?;
    }
    match &state {
        Some(s) => tx.execute(
            "UPDATE trucks SET odometer_km = ?1, state = ?2, rev = rev + 1 WHERE id = ?3",
            params![new_odometer, s.to_string(), truck_id],
        )?,
        None => tx.execute(
            "UPDATE trucks SET odometer_km = ?1, rev = rev + 1 WHERE id = ?2",
            params![new_odometer, truck_id],
        )?,
    };
    // Der letzte bekannte Zustand gehört zu dieser Leihe
    tx.execute(
        "UPDATE trucks SET last_seen_at = NULL, last_odometer_km = NULL, last_state = NULL
         WHERE id = ?1",
        [truck_id],
    )?;
    tx.execute(
        "UPDATE leases SET returned_at = ?1 WHERE id = ?2",
        params![clock.now(), lease_id],
    )?;
    let rev: i64 = tx.query_row("SELECT rev FROM trucks WHERE id = ?1", [truck_id], |r| {
        r.get(0)
    })?;
    let plate = new_plate.unwrap_or(current_plate);
    // Schaden des neuen Zustands: über der Grenze in die Werkstatt, neuer Schaden kostet
    // den Ausleiher seinen Haftungsanteil
    let ds = damage_settings(&tx, company_id)?;
    let d = stored_damage(&tx, truck_id)?;
    let in_workshop = d.fixable > ds.limit as f64;
    tx.execute(
        "UPDATE trucks SET in_workshop = ?1 WHERE id = ?2",
        params![in_workshop, truck_id],
    )?;
    let damage_new = damage::new_damage(d.fixable, damage_out);
    let liability = damage::liability(damage_new, ds.rate, ds.liability_percent);
    if liability > 0 {
        let note = format!(
            "Haftung {plate}, {} Prozentpunkte neuer Schaden",
            damage::percent_text(damage_new)
        );
        for (account, amount) in [("fahrer", -liability), ("firma", liability)] {
            tx.execute(
                "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at)
                 VALUES (?1, ?2, ?3, 'haftung', ?4, ?5, ?6)",
                params![company_id, account, holder, amount, note, clock.now()],
            )?;
        }
    }
    tx.commit()?;
    Ok(Json(json!({
        "id": truck_id, "plate": plate, "odometer_km": new_odometer, "rev": rev,
        "driven_km": new_odometer - odometer,
        // Gesetzt, wenn der letzte bekannte Zustand übernommen wurde: von wann er ist
        "from_last_known": from_last.map(|at| json!({ "at": at })),
        "damage": d.fixable, "damage_unfixable": d.unfixable, "damage_out": damage_out,
        "damage_new": damage_new, "liability": liability, "liability_user_id": holder,
        "damage_limit": ds.limit, "in_workshop": in_workshop
    })))
}

/// Lkw in die Werkstatt: Nur der Chef, nur wenn der Lkw nicht verliehen ist und Schaden hat.
/// Die Kosten (Schaden mal Satz je Prozentpunkt) gehen aus der Firmenkasse, reicht sie nicht,
/// ändert sich nichts. Danach sind alle reparierbaren Verschleißwerte im gespeicherten Zustand
/// 0, der dauerhafte Verschleiß bleibt. Die Revision steigt, weil sich der Zustand geändert
/// hat, und der Lkw ist wieder frei.
async fn repair(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(truck_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    let (company_id, plate, state, retired): (i64, String, String, Option<String>) = tx
        .query_row(
            "SELECT company_id, plate, state, retired_reason FROM trucks WHERE id = ?1",
            [truck_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diesen Lkw gibt es nicht"))?;
    require_chef(&tx, company_id, user.id)?;
    if let Some(reason) = retired {
        return Err(retired_error(&reason, "kommt nicht mehr in die Werkstatt"));
    }
    let holder: Option<String> = tx
        .query_row(
            "SELECT u.name FROM leases l JOIN users u ON u.id = l.user_id
             WHERE l.truck_id = ?1 AND l.returned_at IS NULL",
            [truck_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(name) = holder {
        return Err(ApiError::conflict(&format!(
            "Der Lkw ist gerade an {name} verliehen. Reparieren geht erst nach der Rückgabe"
        )));
    }
    let state: Value = serde_json::from_str(&state).unwrap_or(Value::Null);
    let before = damage::damage(&state);
    if before.fixable <= 0.0 {
        return Err(ApiError::conflict("Am Lkw ist nichts zu reparieren"));
    }
    let ds = damage_settings(&tx, company_id)?;
    let cost = damage::repair_cost(before.fixable, ds.rate);
    let balance = company_balance(&tx, company_id)?;
    if cost > 0 && cost > balance {
        return Err(ApiError::conflict(&format!(
            "In der Kasse ist nicht genug Geld für die Werkstatt: nötig {cost} €, da sind {balance} €"
        )));
    }
    if cost > 0 {
        let note = format!(
            "Werkstatt {plate}, {} %",
            damage::percent_text(before.fixable)
        );
        tx.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at)
             VALUES (?1, 'firma', ?2, 'werkstatt', ?3, ?4, ?5)",
            params![company_id, user.id, -cost, note, clock.now()],
        )?;
    }
    let repaired = damage::repaired(&state);
    tx.execute(
        "UPDATE trucks SET state = ?1, rev = rev + 1, in_workshop = 0 WHERE id = ?2",
        params![repaired.to_string(), truck_id],
    )?;
    let rev: i64 = tx.query_row("SELECT rev FROM trucks WHERE id = ?1", [truck_id], |r| {
        r.get(0)
    })?;
    let after = damage::damage(&repaired);
    let balance = company_balance(&tx, company_id)?;
    tx.commit()?;
    Ok(Json(json!({
        "id": truck_id, "plate": plate, "cost": cost, "damage_before": before.fixable,
        "damage": after.fixable, "damage_unfixable": after.unfixable, "rev": rev,
        "in_workshop": false, "balance": balance
    })))
}

/// 409 für einen ausgemusterten oder verkauften Lkw: "Der Lkw ist verkauft und `what`".
fn retired_error(reason: &str, what: &str) -> ApiError {
    let state = if reason == "verkauft" {
        "verkauft"
    } else {
        "ausgemustert"
    };
    ApiError::conflict(&format!("Der Lkw ist {state} und {what}"))
}

#[derive(Deserialize)]
struct SellReq {
    /// Erlös in Euro, 0 oder mehr. Geht in die Firmenkasse.
    price: i64,
}

/// Lkw ausmustern: Er lässt sich nicht mehr ausleihen, bleibt aber mit seinen Buchungen und
/// alten Kennzeichen in der Fuhrparkliste. Nur der Chef, nur wenn er nicht verliehen ist.
async fn retire(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(truck_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    retire_truck(&mut conn, clock.now(), user.id, truck_id, None).map(Json)
}

/// Lkw verkaufen: wie ausmustern, dazu geht der Erlös als `lkw_verkauf` in die Firmenkasse,
/// in derselben Transaktion. Bei 0 Euro wird nichts gebucht.
async fn sell(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(truck_id): Path<i64>,
    Json(b): Json<SellReq>,
) -> ApiResult<Json<Value>> {
    if !(0..=1_000_000_000).contains(&b.price) {
        return Err(ApiError::bad(
            "Der Erlös muss zwischen 0 und 1000000000 Euro liegen",
        ));
    }
    let mut conn = db.lock().unwrap();
    retire_truck(&mut conn, clock.now(), user.id, truck_id, Some(b.price)).map(Json)
}

/// Ausmustern (`price` leer) oder verkaufen.
fn retire_truck(
    conn: &mut Connection,
    now: i64,
    user_id: i64,
    truck_id: i64,
    price: Option<i64>,
) -> ApiResult<Value> {
    let tx = conn.transaction()?;
    let (company_id, plate, retired): (i64, String, Option<String>) = tx
        .query_row(
            "SELECT company_id, plate, retired_reason FROM trucks WHERE id = ?1",
            [truck_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diesen Lkw gibt es nicht"))?;
    require_chef(&tx, company_id, user_id)?;
    if let Some(reason) = retired {
        return Err(retired_error(&reason, "steht nicht mehr im Fuhrpark"));
    }
    let verb = if price.is_some() {
        "verkaufen"
    } else {
        "ausmustern"
    };
    let holder: Option<String> = tx
        .query_row(
            "SELECT u.name FROM leases l JOIN users u ON u.id = l.user_id
             WHERE l.truck_id = ?1 AND l.returned_at IS NULL",
            [truck_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(name) = holder {
        return Err(ApiError::conflict(&format!(
            "Der Lkw ist gerade an {name} verliehen. Erst zurückgeben, dann {verb}"
        )));
    }
    let reason = if price.is_some() {
        "verkauft"
    } else {
        "ausgemustert"
    };
    tx.execute(
        "UPDATE trucks SET retired_at = ?1, retired_reason = ?2, sale_price = ?3 WHERE id = ?4",
        params![now, reason, price, truck_id],
    )?;
    if let Some(p) = price.filter(|&p| p > 0) {
        tx.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at)
             VALUES (?1, 'firma', ?2, 'lkw_verkauf', ?3, ?4, ?5)",
            params![company_id, user_id, p, purchases::sale_note(&plate), now],
        )?;
    }
    let balance = company_balance(&tx, company_id)?;
    tx.commit()?;
    Ok(json!({
        "id": truck_id, "plate": plate,
        "retired": { "reason": reason, "at": now, "sale_price": price },
        "balance": balance
    }))
}

// ---------------------------------------------------------------- Herzschlag

/// Herzschlag aus dem Spiel, alle 10 Sekunden, solange das Spiel läuft und nicht pausiert ist.
/// Speichert, wer mit welchem Lkw wo fährt, und beim eigenen geliehenen Lkw den letzten
/// bekannten Zustand. Siehe [`crate::presence`].
async fn heartbeat(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(mut b): Json<HeartbeatReq>,
) -> ApiResult<Json<Value>> {
    let profile =
        profiles::clean_name(b.profile.take().as_deref()).map_err(|e| ApiError::bad(&e))?;
    let hb = b.validate().map_err(|e| ApiError::bad(&e))?;
    let now = clock.now();
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    role_in(&tx, company_id, user.id)?;
    // Aus einem anderen Profil als dem gemeldeten Firmenprofil: verwerfen, ohne Fehler. Ein
    // Herzschlag wird nie wiederholt, und das Plugin schreibt den Hinweis ins Log.
    let reported = profiles::reported(&tx, company_id, user.id)?;
    if let Some(m) = profiles::mismatch(reported.as_deref(), profile.as_deref()) {
        return Ok(Json(json!({
            "ok": true, "recorded": false, "note": format!("Herzschlag verworfen. {}", m.message),
            "last_seen": Value::Null, "leased_truck_id": Value::Null
        })));
    }
    let rec = presence::record(&tx, company_id, user.id, &hb, now)?;
    tx.commit()?;
    // Nur der eigene geliehene Lkw kommt zurück. Einen fremden sieht nur der Chef.
    Ok(Json(json!({
        "ok": true, "recorded": true, "last_seen": now,
        "leased_truck_id": rec.truck_id.filter(|_| rec.own_lease)
    })))
}

// ---------------------------------------------------------------- Kasse

#[derive(Deserialize, Serialize)]
struct EventReq {
    /// Vom Client vergeben und eindeutig. Verhindert doppelte Buchungen.
    event_id: String,
    #[serde(rename = "type")]
    kind: String,
    /// Betrag in Euro, immer positiv, so wie das Spiel ihn gemeldet hat. Bei job_delivered ist
    /// das der Erlös.
    amount: i64,
    /// Gefahrene Kilometer, nur bei job_delivered
    #[serde(default)]
    distance_km: Option<f64>,
    #[serde(default)]
    note: String,
    /// Wann das Ereignis im Spiel passiert ist: Systemzeit des Fahrer-PCs, Unix-Sekunden UTC.
    /// Fehlt er oder ist er unplausibel, gilt der Empfangszeitpunkt.
    #[serde(default)]
    occurred_at: Option<i64>,
    /// Fracht, Start- und Zielstadt, nur bei job_delivered. Aus der Job-Konfiguration des SDK.
    #[serde(default)]
    cargo: Option<String>,
    #[serde(default)]
    source_city: Option<String>,
    #[serde(default)]
    target_city: Option<String>,
    /// Name des ETS2-Profils, aus dem das Plugin das Ereignis meldet. Ist für den Fahrer ein
    /// Firmenprofil gemeldet, muss es genau dieses sein.
    #[serde(default)]
    profile: Option<String>,
}

/// Ereignis aus dem Spiel buchen. Die Regeln:
/// - job_delivered: Erlös mal Erlösfaktor in die Firmenkasse, die Kilometer zählen für den Fahrer
/// - job_cancelled, fined: zahlt der Fahrer aus dem Lohnkonto
/// - tollgate, ferry, train: zahlt die Firma
///
/// Jede Buchung merkt sich in `game_amount` den vollen Betrag aus dem Spiel mit Vorzeichen, so
/// wie er den Kontostand im Spiel verändert hat, und in `occurred_at`, wann es laut Client im
/// Spiel passiert ist. Ohne plausiblen Zeitpunkt bleibt `occurred_at` leer, dann zählt das
/// Ereignis im nächsten Abgleich. Daraus rechnet der Geldabgleich.
///
/// Ist das Ereignis laut Client vor einem Abgleich passiert, der es nicht kannte, steckte sein Spielbetrag
/// schon im damaligen Kontostand. Dann wird dieser Abgleich korrigiert (siehe
/// [`correct_late`]), und die Antwort sagt das unter `late`.
///
/// Hat der Fahrer ein Firmenprofil gemeldet und kommt das Ereignis aus einem anderen oder nennt
/// es keins, wird nichts gebucht: Antwort 422, das Ereignis landet in `rejected_events` und im
/// Log des Servers. 422 statt 409, weil das Plugin 422 als endgültig behandelt und das Ereignis
/// beiseitelegt, während es bei 409 den Postausgang immer wieder senden und dahinter stauen
/// würde. Siehe [`crate::profiles`].
async fn post_event(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(b): Json<EventReq>,
) -> ApiResult<Json<Value>> {
    let event_id = b.event_id.trim();
    if event_id.is_empty() || event_id.len() > 100 {
        return Err(ApiError::bad("event_id fehlt oder ist zu lang"));
    }
    if b.amount < 0 || b.amount > 1_000_000_000 {
        return Err(ApiError::bad("Der Betrag darf nicht negativ sein"));
    }
    let profile = profiles::clean_name(b.profile.as_deref()).map_err(|e| ApiError::bad(&e))?;
    let distance = match b.distance_km {
        None => None,
        Some(d) if d.is_finite() && (0.0..=100_000.0).contains(&d) => Some(d.round() as i64),
        Some(_) => {
            return Err(ApiError::bad(
                "Die Strecke muss zwischen 0 und 100000 km liegen",
            ));
        }
    };
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    role_in(&tx, company_id, user.id)?;

    let seen: i64 = tx.query_row(
        "SELECT COUNT(*) FROM ledger WHERE company_id = ?1 AND event_id = ?2",
        params![company_id, event_id],
        |r| r.get(0),
    )?;
    if seen > 0 {
        return Ok(Json(json!({
            "duplicate": true, "booked": [],
            "balance": company_balance(&tx, company_id)?, "wallet": wallet(&tx, company_id, user.id)?
        })));
    }

    // Nur aus dem gemeldeten Firmenprofil
    let reported = profiles::reported(&tx, company_id, user.id)?;
    if let Some(m) = profiles::mismatch(reported.as_deref(), profile.as_deref()) {
        let payload = serde_json::to_value(&b).unwrap_or(Value::Null);
        let reason = format!("Ereignis {}", m.reason);
        profiles::record_rejected(
            &tx,
            &profiles::Rejected {
                company_id,
                user_id: user.id,
                event_id,
                kind: &b.kind,
                amount: b.amount,
                profile: profile.as_deref(),
                reason: &reason,
                payload: &payload,
                now: clock.now(),
            },
        )?;
        tx.commit()?;
        eprintln!(
            "Abgelehnt: Firma {company_id}, {}: {} {} Euro ({event_id}), {reason}",
            user.name, b.kind, b.amount
        );
        return Err(ApiError(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            format!("Ereignis nicht gebucht. {}", m.message),
        ));
    }

    // (Konto, Buchungsart, gebuchter Betrag, Betrag im Spiel), beide mit Vorzeichen
    let a = b.amount;
    let booking: (&str, &str, i64, i64) = match b.kind.as_str() {
        "job_delivered" => {
            let factor: f64 = tx.query_row(
                "SELECT revenue_factor FROM companies WHERE id = ?1",
                [company_id],
                |r| r.get(0),
            )?;
            ("firma", "auftrag", (a as f64 * factor).round() as i64, a)
        }
        "job_cancelled" => ("fahrer", "auftrag_abgebrochen", -a, -a),
        "fined" => ("fahrer", "strafe", -a, -a),
        "tollgate" => ("firma", "maut", -a, -a),
        "ferry" => ("firma", "faehre", -a, -a),
        "train" => ("firma", "zug", -a, -a),
        _ => return Err(ApiError::bad("Unbekannter Ereignistyp")),
    };
    let (account, kind, amount, game_amount) = booking;
    let distance = if kind == "auftrag" { distance } else { None };
    // Fracht und Städte nur beim Auftrag, als Text gekürzt und ohne Steuerzeichen
    let text = |v: &Option<String>| {
        (kind == "auftrag")
            .then(|| presence::clean_text(v.as_deref(), presence::TEXT_MAX))
            .flatten()
    };
    let (cargo, source_city, target_city) =
        (text(&b.cargo), text(&b.source_city), text(&b.target_city));
    let now = clock.now();
    // Nur ein Zeitpunkt vom Client kann einen alten Abgleich korrigieren. Ohne ihn ist das
    // Ereignis nach jedem bisherigen Abgleich angekommen und zählt im nächsten Fenster.
    let reported_at = client_time(b.occurred_at, now);
    let late = match reported_at {
        Some(t) => correct_late(
            &tx,
            company_id,
            user.id,
            t,
            game_amount,
            kind_label(kind),
            now,
        )?,
        None => None,
    };
    tx.execute(
        "INSERT INTO ledger (company_id, account, user_id, kind, amount, event_id, note, created_at,
                             game_amount, distance_km, occurred_at, cargo, source_city, target_city,
                             profile)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            company_id,
            account,
            user.id,
            kind,
            amount,
            event_id,
            b.note,
            now,
            game_amount,
            distance,
            reported_at,
            cargo,
            source_city,
            target_city,
            profile
        ],
    )?;
    let balance = company_balance(&tx, company_id)?;
    let my_wallet = wallet(&tx, company_id, user.id)?;
    tx.commit()?;
    let mut res = json!({
        "duplicate": false,
        "booked": [{
            "account": account, "kind": kind, "amount": amount, "game_amount": game_amount,
            "distance_km": distance
        }],
        "occurred_at": reported_at,
        "balance": balance, "wallet": my_wallet
    });
    if let Some(late) = late {
        res["late"] = late;
    }
    Ok(Json(res))
}

#[derive(Deserialize)]
struct ProfileReq {
    /// Name des Firmenprofils, so wie das Spiel ihn anzeigt. `null` oder leer löscht ihn.
    #[serde(default)]
    profile_name: Option<String>,
}

/// Der Launcher meldet das Firmenprofil des angemeldeten Fahrers. Ab dann bucht der Server nur
/// noch Ereignisse aus diesem Profil, siehe [`crate::profiles`].
async fn set_profile(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(b): Json<ProfileReq>,
) -> ApiResult<Json<Value>> {
    let name = profiles::clean_name(b.profile_name.as_deref()).map_err(|e| ApiError::bad(&e))?;
    let conn = db.lock().unwrap();
    role_in(&conn, company_id, user.id)?;
    profiles::report(&conn, company_id, user.id, name.as_deref())?;
    Ok(Json(json!({ "profile_name": name })))
}

#[derive(Deserialize)]
struct RejectedQuery {
    limit: Option<i64>,
}

/// Ereignisse, die der Server wegen des Profils nicht gebucht hat. Nur der Chef.
async fn list_rejected_events(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Query(q): Query<RejectedQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let conn = db.lock().unwrap();
    require_chef(&conn, company_id, user.id)?;
    let entries = profiles::list_rejected(&conn, company_id, limit)?;
    Ok(Json(json!({ "entries": entries })))
}

#[derive(Deserialize)]
struct TransferReq {
    user_id: i64,
    /// Positiv: aus der Firmenkasse an den Fahrer. Negativ: vom Fahrer zurück in die Kasse.
    amount: i64,
    #[serde(default)]
    note: String,
}

/// Geld zwischen Firmenkasse und Lohnkonto verschieben, zum Beispiel Vorschuss oder Rückzahlung.
/// Die Kasse kann dabei nicht ins Minus gehen, ein Lohnkonto schon (Schulden beim Chef).
async fn transfer(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(b): Json<TransferReq>,
) -> ApiResult<Json<Value>> {
    if b.amount == 0 {
        return Err(ApiError::bad("Der Betrag darf nicht 0 sein"));
    }
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    require_chef(&tx, company_id, user.id)?;
    let is_member: i64 = tx.query_row(
        "SELECT COUNT(*) FROM memberships WHERE company_id = ?1 AND user_id = ?2",
        [company_id, b.user_id],
        |r| r.get(0),
    )?;
    if is_member == 0 {
        return Err(ApiError::not_found("Diese Person ist nicht in der Firma"));
    }
    if b.amount > company_balance(&tx, company_id)? {
        return Err(ApiError::conflict("In der Kasse ist nicht genug Geld"));
    }
    for (account, amount) in [("firma", -b.amount), ("fahrer", b.amount)] {
        tx.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at)
             VALUES (?1, ?2, ?3, 'ueberweisung', ?4, ?5, ?6)",
            params![company_id, account, b.user_id, amount, b.note, clock.now()],
        )?;
    }
    let balance = company_balance(&tx, company_id)?;
    let target_wallet = wallet(&tx, company_id, b.user_id)?;
    tx.commit()?;
    Ok(Json(json!({ "balance": balance, "wallet": target_wallet })))
}

#[derive(Deserialize)]
struct LedgerQuery {
    limit: Option<i64>,
    /// Nur Buchungen mit `created_at` ab hier (Unix-Sekunden, einschließlich)
    from: Option<i64>,
    /// Nur Buchungen mit `created_at` vor diesem Zeitpunkt (Unix-Sekunden, ausschließlich)
    to: Option<i64>,
}

/// Die letzten Buchungen, neueste zuerst. Mit `from` und `to` nur die aus diesem Zeitraum
/// (Empfang beim Server). Jede Buchung sagt, ob und von wem sie storniert ist, ein Storno, was
/// es storniert, und `reversible`, ob sich die Buchung jetzt stornieren lässt.
async fn get_ledger(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Query(q): Query<LedgerQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let conn = db.lock().unwrap();
    role_in(&conn, company_id, user.id)?;
    let mut stmt = conn.prepare(
        "SELECT l.id, l.account, u.name, l.kind, l.amount, l.note, l.created_at, l.user_id,
                l.game_amount, l.distance_km, l.payroll_id, l.occurred_at, l.event_id,
                l.cargo, l.source_city, l.target_city, pl.truck_id, tr.plate, l.profile,
                l.reverses_id, r.id, r.created_at, r.note, ru.name, o.kind, o.created_at,
                bu.name
         FROM ledger l LEFT JOIN users u ON u.id = l.user_id
         LEFT JOIN purchase_links pl ON pl.ledger_id = l.id
         LEFT JOIN trucks tr ON tr.id = pl.truck_id
         LEFT JOIN ledger r ON r.reverses_id = l.id AND r.account = l.account
         LEFT JOIN users ru ON ru.id = r.booked_by
         LEFT JOIN ledger o ON o.id = l.reverses_id
         LEFT JOIN users bu ON bu.id = l.booked_by
         WHERE l.company_id = ?1 AND (?3 IS NULL OR l.created_at >= ?3)
           AND (?4 IS NULL OR l.created_at < ?4)
         ORDER BY l.id DESC LIMIT ?2",
    )?;
    let entries = stmt
        .query_map(params![company_id, limit, q.from, q.to], |r| {
            let kind: String = r.get(3)?;
            let account: String = r.get(1)?;
            let payroll_id: Option<i64> = r.get(10)?;
            let reversed_by: Option<i64> = r.get(20)?;
            let reversible = reversed_by.is_none()
                && reversal::not_reversible(&kind, &account, payroll_id).is_none();
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "account": account,
                "user": r.get::<_, Option<String>>(2)?, "kind": kind,
                "amount": r.get::<_, i64>(4)?, "note": r.get::<_, String>(5)?,
                "created_at": r.get::<_, i64>(6)?, "user_id": r.get::<_, Option<i64>>(7)?,
                "game_amount": r.get::<_, Option<i64>>(8)?,
                "distance_km": r.get::<_, Option<i64>>(9)?,
                "payroll_id": payroll_id,
                "occurred_at": r.get::<_, Option<i64>>(11)?,
                "event_id": r.get::<_, Option<String>>(12)?,
                "cargo": r.get::<_, Option<String>>(13)?,
                "source_city": r.get::<_, Option<String>>(14)?,
                "target_city": r.get::<_, Option<String>>(15)?,
                // Als Lkw-Kauf verknüpft: der Lkw und sein aktuelles Kennzeichen
                "truck_id": r.get::<_, Option<i64>>(16)?,
                "truck_plate": r.get::<_, Option<String>>(17)?,
                // Aus diesem ETS2-Profil hat das Plugin das Ereignis gemeldet
                "profile": r.get::<_, Option<String>>(18)?,
                // Storno: was diese Buchung storniert (Art und Zeitpunkt des Originals) und wer
                "reverses_id": r.get::<_, Option<i64>>(19)?,
                "reverses_kind": r.get::<_, Option<String>>(24)?,
                "reverses_created_at": r.get::<_, Option<i64>>(25)?,
                "booked_by_user": r.get::<_, Option<String>>(26)?,
                // Storniert: von welcher Buchung, wann, warum und von wem
                "reversed_by": reversed_by,
                "reversed_at": r.get::<_, Option<i64>>(21)?,
                "reverse_reason": r.get::<_, Option<String>>(22)?,
                "reversed_by_user": r.get::<_, Option<String>>(23)?,
                "reversible": reversible
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(
        json!({ "balance": company_balance(&conn, company_id)?, "entries": entries }),
    ))
}

#[derive(Deserialize)]
struct ReverseReq {
    #[serde(default)]
    entry_ids: Vec<i64>,
    #[serde(default)]
    reason: Option<String>,
}

/// Buchungen stornieren, siehe [`crate::reversal`]. Nur der Chef. Alles oder nichts in einer
/// Transaktion. Die Antwort enthält die neuen Storno-Buchungen, den Kassenstand und je
/// betroffenem Fahrer Lohnkonto und Wirkung auf den Geldabgleich.
async fn reverse_entries(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(b): Json<ReverseReq>,
) -> ApiResult<Json<Value>> {
    let reason = reversal::clean_reason(b.reason.as_deref()).map_err(|e| ApiError::bad(&e))?;
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    require_chef(&tx, company_id, user.id)?;
    let (entries, drivers) =
        reversal::reverse(&tx, company_id, user.id, &b.entry_ids, &reason, clock.now())?;
    let mut list = Vec::new();
    for (uid, mut d) in drivers {
        let name: Option<String> = tx
            .query_row("SELECT name FROM users WHERE id = ?1", [uid], |r| r.get(0))
            .optional()?;
        d["user"] = json!(name);
        d["wallet"] = json!(wallet(&tx, company_id, uid)?);
        list.push(d);
    }
    let balance = company_balance(&tx, company_id)?;
    tx.commit()?;
    Ok(Json(json!({
        "entries": entries, "balance": balance, "drivers": list
    })))
}

#[derive(Deserialize)]
struct LinkReq {
    truck_id: i64,
}

/// Eine Betriebskosten-Buchung als Kauf eines Lkw verknüpfen. Nur der Chef. Die Buchung bleibt
/// unverändert, im Kassenbuch steht dann "Lkw-Kauf <Kennzeichen>".
async fn link_truck(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path((company_id, entry)): Path<(i64, i64)>,
    Json(b): Json<LinkReq>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    require_chef(&tx, company_id, user.id)?;
    let (kind, account): (String, String) = tx
        .query_row(
            "SELECT kind, account FROM ledger WHERE id = ?1 AND company_id = ?2",
            [entry, company_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diese Buchung gibt es nicht"))?;
    if kind != "betriebskosten" || account != "firma" {
        return Err(ApiError::bad(
            "Als Lkw-Kauf lässt sich nur eine Betriebskosten-Buchung der Firmenkasse verknüpfen",
        ));
    }
    let reversed: bool = tx
        .prepare("SELECT 1 FROM ledger WHERE reverses_id = ?1")?
        .exists([entry])?;
    if reversed {
        return Err(ApiError::conflict(
            "Die Buchung ist storniert und lässt sich nicht mehr als Lkw-Kauf verknüpfen",
        ));
    }
    let linked: Option<String> = tx
        .query_row(
            "SELECT t.plate FROM purchase_links p JOIN trucks t ON t.id = p.truck_id
             WHERE p.ledger_id = ?1",
            [entry],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(plate) = linked {
        return Err(ApiError::conflict(&format!(
            "Die Buchung ist schon als Kauf von {plate} verknüpft"
        )));
    }
    let plate: String = tx
        .query_row(
            "SELECT plate FROM trucks WHERE id = ?1 AND company_id = ?2",
            [b.truck_id, company_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diesen Lkw gibt es in der Firma nicht"))?;
    tx.execute(
        "INSERT INTO purchase_links (ledger_id, truck_id, linked_by, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![entry, b.truck_id, user.id, clock.now()],
    )?;
    tx.commit()?;
    Ok(Json(json!({
        "ledger_id": entry, "truck_id": b.truck_id, "truck_plate": plate
    })))
}

/// Verknüpfung lösen, etwa wenn der Chef den falschen Lkw gewählt hat. Die Buchung steht
/// danach wieder als Betriebskosten im Kassenbuch.
async fn unlink_truck(
    State(db): State<Db>,
    user: AuthUser,
    Path((company_id, entry)): Path<(i64, i64)>,
) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    require_chef(&conn, company_id, user.id)?;
    let removed = conn.execute(
        "DELETE FROM purchase_links WHERE ledger_id = ?1
           AND ledger_id IN (SELECT id FROM ledger WHERE company_id = ?2)",
        [entry, company_id],
    )?;
    if removed == 0 {
        return Err(ApiError::not_found(
            "Die Buchung ist mit keinem Lkw verknüpft",
        ));
    }
    Ok(Json(json!({ "ledger_id": entry, "unlinked": true })))
}

#[derive(Deserialize)]
struct CandidateQuery {
    limit: Option<i64>,
}

/// Betriebskosten-Buchungen des Chefs, die noch mit keinem Lkw verknüpft und nicht storniert
/// sind, neueste zuerst.
/// Daraus wählt der Chef nach "Lkw übernehmen" den Kauf.
async fn purchase_candidates(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Query(q): Query<CandidateQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(10).clamp(1, 100);
    let conn = db.lock().unwrap();
    require_chef(&conn, company_id, user.id)?;
    let entries: Vec<Value> = conn
        .prepare(
            "SELECT l.id, l.created_at, l.amount, l.note FROM ledger l
             WHERE l.company_id = ?1 AND l.account = 'firma' AND l.kind = 'betriebskosten'
               AND l.user_id = ?2
               AND NOT EXISTS (SELECT 1 FROM purchase_links p WHERE p.ledger_id = l.id)
               AND NOT EXISTS (SELECT 1 FROM ledger s WHERE s.reverses_id = l.id)
             ORDER BY l.id DESC LIMIT ?3",
        )?
        .query_map(params![company_id, user.id, limit], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "created_at": r.get::<_, i64>(1)?,
                "amount": r.get::<_, i64>(2)?, "note": r.get::<_, String>(3)?
            }))
        })?
        .collect::<Result<_, _>>()?;
    Ok(Json(json!({ "entries": entries })))
}

// ---------------------------------------------------------------- Anschaffungsbudget

#[derive(Deserialize)]
struct BudgetReq {
    amount: i64,
    #[serde(default)]
    note: Option<String>,
}

/// Anschaffungsbudget freigeben, siehe [`crate::purchases`]. Nur der Chef. Höchstens ein offenes
/// Budget je Firma. Die Kasse muss den Betrag decken, gebucht wird aber nichts.
async fn create_budget(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(b): Json<BudgetReq>,
) -> ApiResult<Json<Value>> {
    if !(1..=1_000_000_000).contains(&b.amount) {
        return Err(ApiError::bad(
            "Der Betrag muss zwischen 1 und 1000000000 Euro liegen",
        ));
    }
    let note = presence::clean_text(b.note.as_deref(), 80).unwrap_or_default();
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    require_chef(&tx, company_id, user.id)?;
    // Gäbe es mehrere offene Budgets, müsste die Kasse auch deren Reste decken. Es gibt aber
    // höchstens eins, also reicht der Kassenstand.
    if purchases::open_budget(&tx, company_id)?.is_some() {
        return Err(ApiError::conflict(
            "Es gibt schon ein offenes Anschaffungsbudget. Schließ es zuerst, dann lässt sich ein neues freigeben",
        ));
    }
    let balance = company_balance(&tx, company_id)?;
    if b.amount > balance {
        return Err(ApiError::conflict(&format!(
            "In der Kasse ist nicht genug Geld für das Budget: nötig {} €, da sind {balance} €",
            b.amount
        )));
    }
    tx.execute(
        "INSERT INTO purchase_budgets (company_id, user_id, amount, note, created_at, ledger_from,
                                       sync_from)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            company_id,
            user.id,
            b.amount,
            note,
            clock.now(),
            purchases::max_ledger_id(&tx)?,
            purchases::max_sync_id(&tx)?
        ],
    )?;
    let id = tx.last_insert_rowid();
    let budget = purchases::budget(&tx, company_id, id)?
        .ok_or_else(|| ApiError::not_found("Dieses Budget gibt es nicht"))?;
    tx.commit()?;
    Ok(Json(json!({ "budget": budget.json(), "balance": balance })))
}

/// Alle Anschaffungsbudgets der Firma mit Restbetrag, das offene unter `open`. Nur der Chef.
async fn list_budgets(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    require_chef(&conn, company_id, user.id)?;
    let all = purchases::budgets(&conn, company_id)?;
    let open = all
        .iter()
        .find(|b| b.closed_at.is_none())
        .map_or(Value::Null, Budget::json);
    Ok(Json(json!({
        "open": open,
        "budgets": all.iter().map(Budget::json).collect::<Vec<_>>()
    })))
}

/// Budget schließen. Gebucht wird nichts. Beim nächsten Abgleich kommt der Rest nicht mehr in
/// den Spielstand. Die Antwort sagt unter `rest`, wie viel damit entfällt.
async fn close_budget(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path((company_id, budget_id)): Path<(i64, i64)>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    require_chef(&tx, company_id, user.id)?;
    let budget = purchases::budget(&tx, company_id, budget_id)?
        .ok_or_else(|| ApiError::not_found("Dieses Budget gibt es nicht"))?;
    if budget.closed_at.is_some() {
        return Err(ApiError::conflict("Das Budget ist schon geschlossen"));
    }
    tx.execute(
        "UPDATE purchase_budgets SET closed_at = ?1, closed_by = ?2, ledger_to = ?3 WHERE id = ?4",
        params![
            clock.now(),
            user.id,
            purchases::max_ledger_id(&tx)?,
            budget_id
        ],
    )?;
    let budget = purchases::budget(&tx, company_id, budget_id)?
        .ok_or_else(|| ApiError::not_found("Dieses Budget gibt es nicht"))?;
    tx.commit()?;
    Ok(Json(json!({ "budget": budget.json() })))
}

// ---------------------------------------------------------------- Lohnabrechnung

async fn list_payrolls(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Query(q): Query<LedgerQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(30).clamp(1, 500);
    let conn = db.lock().unwrap();
    role_in(&conn, company_id, user.id)?;
    let ids: Vec<i64> = conn
        .prepare(
            "SELECT id FROM payroll_runs WHERE company_id = ?1
               AND (?3 IS NULL OR created_at >= ?3) AND (?4 IS NULL OR created_at < ?4)
             ORDER BY id DESC LIMIT ?2",
        )?
        .query_map(params![company_id, limit, q.from, q.to], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let runs = ids
        .into_iter()
        .map(|id| payroll::run_json(&conn, id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!({ "payrolls": runs })))
}

/// Abrechnung sofort auslösen (Testbereich im Dashboard). Nur der Chef.
async fn run_payroll_now(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    require_chef(&conn, company_id, user.id)?;
    let ids = payroll::run_now(&mut conn, company_id, clock.now())?;
    let runs = ids
        .into_iter()
        .map(|id| payroll::run_json(&conn, id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!({
        "payrolls": runs, "balance": company_balance(&conn, company_id)?
    })))
}

// ---------------------------------------------------------------- Geldabgleich

#[derive(Deserialize)]
struct SyncReq {
    /// Kontostand im neuesten Spielstand
    ingame_money: i64,
    /// Erfahrungspunkte aus `info.sii`
    #[serde(default)]
    xp: Option<i64>,
    /// Änderungszeit von `game.sii` des gelesenen Spielstands, Unix-Sekunden. Der Kontostand
    /// darin enthält die Ereignisse bis hier. Fehlt sie, gilt der Empfangszeitpunkt.
    #[serde(default)]
    saved_at: Option<i64>,
}

/// Geldabgleich eines Fahrers. Vergleicht den Kontostand im Spiel mit dem, was die gemeldeten
/// Ereignisse seit dem letzten Abgleich erklären:
///
/// erwartet = Stand nach dem letzten Abgleich + Summe der Spielbeträge im Fenster
///
/// Das Fenster reicht von der Grundlage des letzten Abgleichs bis zum Zeitpunkt des gelesenen
/// Spielstands (`saved_at`), siehe [`crate::reconcile`].
///
/// Fehlt Geld, bucht die Firma die Differenz als Betriebskosten (Tanken, Reparaturen und was
/// das Spiel sonst abzieht). Ist mehr da, wird nichts gutgeschrieben, sondern ein ungeklärter
/// Betrag für den Chef angelegt. Der erste Abgleich bucht nichts.
///
/// Zurück kommt, was der Launcher in den Spielstand schreiben soll: Lohnkonto plus
/// Betriebsvorschuss, beim Chef plus der Rest eines offenen Anschaffungsbudgets (siehe
/// [`crate::purchases`]), nie unter 0. Hat er es geschrieben, meldet er das über `.../written`.
async fn sync(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(b): Json<SyncReq>,
) -> ApiResult<Json<Value>> {
    if b.ingame_money.abs() > 1_000_000_000_000 {
        return Err(ApiError::bad("Der Kontostand ist nicht plausibel"));
    }
    if let Some(xp) = b.xp
        && !(0..=1_000_000_000).contains(&xp)
    {
        return Err(ApiError::bad(
            "Die Erfahrungspunkte dürfen nicht negativ sein",
        ));
    }
    let t = clock.now();
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    role_in(&tx, company_id, user.id)?;
    let last = last_point(&tx, company_id, user.id)?;
    // Nie vor der Grundlage des vorigen Abgleichs: Kommen Abgleiche mit verschiedenen Uhren
    // (Launcher und Dashboard), überlappten sonst die Fenster und Ereignisse zählten doppelt.
    let mut cutoff = plausible_time(b.saved_at, t);
    if let Some(last) = &last {
        cutoff = cutoff.max(last.base());
    }

    let mut booked = Vec::new();
    let mut expected = None;
    let mut diff = None;
    let mut costs = 0;
    let mut surplus = 0;
    let mut events = 0;
    if let Some(last) = &last {
        let (sum, n) = window_sum(
            &tx,
            company_id,
            user.id,
            last.base(),
            cutoff,
            last.last_ledger_id,
        )?;
        events = n;
        let exp = last.baseline() + sum;
        expected = Some(exp);
        let d = b.ingame_money - exp;
        diff = Some(d);
        if d < 0 {
            costs = -d;
        } else {
            surplus = d;
        }
    }

    // Erst der Abgleichpunkt, damit die Betriebskosten ihn in `sync_id` tragen. Was geschrieben
    // werden soll und die größte Buchung kommen danach dazu.
    tx.execute(
        "INSERT INTO sync_points (company_id, user_id, ingame_money, xp, expected, write_money,
                                  last_ledger_id, created_at, cutoff, diff, costs_booked,
                                  unexplained_attributed)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, ?6, ?7, ?8, ?9, ?10)",
        params![
            company_id,
            user.id,
            b.ingame_money,
            b.xp,
            expected,
            t,
            cutoff,
            diff,
            costs,
            surplus
        ],
    )?;
    let sync_id = tx.last_insert_rowid();
    if let (Some(exp), true) = (expected, costs > 0) {
        let note = format!("Abgleich: erwartet {exp} €, im Spiel {} €", b.ingame_money);
        tx.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at,
                                 sync_id)
             VALUES (?1, 'firma', ?2, 'betriebskosten', ?3, ?4, ?5, ?6)",
            params![company_id, user.id, -costs, note, t, sync_id],
        )?;
        booked.push(json!({ "account": "firma", "kind": "betriebskosten", "amount": -costs }));
    }

    let advance: i64 = tx.query_row(
        "SELECT advance FROM companies WHERE id = ?1",
        [company_id],
        |r| r.get(0),
    )?;
    let my_wallet = wallet(&tx, company_id, user.id)?;
    // Anschaffungsbudget des Chefs: der Rest nach den gerade gebuchten Betriebskosten
    let budget = purchases::open_rest_for(&tx, company_id, user.id)?;
    let write_money = (my_wallet + advance + budget).max(0);
    let last_ledger_id: i64 =
        tx.query_row("SELECT COALESCE(MAX(id), 0) FROM ledger", [], |r| r.get(0))?;
    tx.execute(
        "UPDATE sync_points SET write_money = ?1, last_ledger_id = ?2 WHERE id = ?3",
        params![write_money, last_ledger_id, sync_id],
    )?;
    let mut unexplained = Value::Null;
    if surplus > 0 {
        tx.execute(
            "INSERT INTO unexplained (company_id, user_id, amount, sync_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![company_id, user.id, surplus, sync_id, t],
        )?;
        unexplained = json!({ "id": tx.last_insert_rowid(), "amount": surplus });
    }
    let levels = wage_levels(&tx, company_id)?;
    let s = standing(&tx, &levels, company_id, user.id)?;
    tx.commit()?;
    Ok(Json(json!({
        "sync_id": sync_id, "first": last.is_none(), "ingame_money": b.ingame_money,
        "expected": expected, "events": events, "booked": booked, "unexplained": unexplained,
        "saved_at": cutoff,
        "wallet": my_wallet, "advance": advance, "budget": budget, "write_money": write_money,
        "xp": s.xp, "game_level": s.game_level, "stage": s.stage.map(|w| w.position)
    })))
}

#[derive(Deserialize, Default)]
struct WrittenReq {
    /// Wann der Launcher den neuen Spielstand geschrieben hat, Unix-Sekunden. Fehlt er, gilt
    /// der Empfangszeitpunkt.
    #[serde(default)]
    written_at: Option<i64>,
}

/// Der Launcher hat den Betrag aus dem Abgleich in den Spielstand geschrieben. Ab jetzt ist
/// das die Grundlage für den nächsten Abgleich, und dessen Fenster beginnt bei `written_at`.
async fn sync_written(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path((company_id, sync_id)): Path<(i64, i64)>,
    body: Option<Json<WrittenReq>>,
) -> ApiResult<Json<Value>> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let conn = db.lock().unwrap();
    role_in(&conn, company_id, user.id)?;
    let latest: Option<i64> = conn.query_row(
        "SELECT MAX(id) FROM sync_points WHERE company_id = ?1 AND user_id = ?2",
        [company_id, user.id],
        |r| r.get(0),
    )?;
    let owner: Option<i64> = conn
        .query_row(
            "SELECT user_id FROM sync_points WHERE id = ?1 AND company_id = ?2",
            [sync_id, company_id],
            |r| r.get(0),
        )
        .optional()?;
    match owner {
        None => return Err(ApiError::not_found("Diesen Abgleich gibt es nicht")),
        Some(u) if u != user.id => {
            return Err(ApiError::forbidden("Das ist nicht dein Abgleich"));
        }
        _ => {}
    }
    if latest != Some(sync_id) {
        return Err(ApiError::conflict("Es gibt schon einen neueren Abgleich"));
    }
    // Nie vor dem gelesenen Spielstand: Sonst zählten Ereignisse dazwischen doppelt.
    let cutoff: i64 = conn.query_row(
        "SELECT COALESCE(cutoff, created_at) FROM sync_points WHERE id = ?1",
        [sync_id],
        |r| r.get(0),
    )?;
    let written_at = plausible_time(req.written_at, clock.now()).max(cutoff);
    conn.execute(
        "UPDATE sync_points SET written = 1, written_at = ?1 WHERE id = ?2",
        params![written_at, sync_id],
    )?;
    Ok(Json(
        json!({ "sync_id": sync_id, "written": true, "written_at": written_at }),
    ))
}

#[derive(Deserialize)]
struct UnexplainedQuery {
    #[serde(default)]
    all: bool,
}

/// Ungeklärte Beträge aus dem Abgleich. Nur der Chef. Ohne `?all=true` nur die offenen.
async fn list_unexplained(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Query(q): Query<UnexplainedQuery>,
) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    require_chef(&conn, company_id, user.id)?;
    let entries: Vec<Value> = conn
        .prepare(
            "SELECT x.id, x.user_id, u.name, x.amount, x.created_at, x.closed_at, c.name, x.close_note,
                    s.expected, s.ingame_money
             FROM unexplained x
             JOIN users u ON u.id = x.user_id
             JOIN sync_points s ON s.id = x.sync_id
             LEFT JOIN users c ON c.id = x.closed_by
             WHERE x.company_id = ?1 AND (?2 OR x.closed_at IS NULL)
             ORDER BY x.id DESC LIMIT 200",
        )?
        .query_map(params![company_id, q.all], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "user_id": r.get::<_, i64>(1)?,
                "user": r.get::<_, String>(2)?, "amount": r.get::<_, i64>(3)?,
                "created_at": r.get::<_, i64>(4)?, "closed_at": r.get::<_, Option<i64>>(5)?,
                "closed_by": r.get::<_, Option<String>>(6)?, "note": r.get::<_, String>(7)?,
                "expected": r.get::<_, Option<i64>>(8)?, "ingame_money": r.get::<_, i64>(9)?
            }))
        })?
        .collect::<Result<_, _>>()?;
    Ok(Json(json!({ "entries": entries })))
}

#[derive(Deserialize, Default)]
struct CloseReq {
    #[serde(default)]
    note: String,
}

/// Ungeklärten Betrag schließen. Gebucht wird dabei nichts.
async fn close_unexplained(
    State(db): State<Db>,
    State(clock): State<Clock>,
    user: AuthUser,
    Path((company_id, entry)): Path<(i64, i64)>,
    body: Option<Json<CloseReq>>,
) -> ApiResult<Json<Value>> {
    let note = body.map(|Json(b)| b.note).unwrap_or_default();
    let conn = db.lock().unwrap();
    require_chef(&conn, company_id, user.id)?;
    let closed: Option<Option<i64>> = conn
        .query_row(
            "SELECT closed_at FROM unexplained WHERE id = ?1 AND company_id = ?2",
            [entry, company_id],
            |r| r.get(0),
        )
        .optional()?;
    match closed {
        None => return Err(ApiError::not_found("Diesen Eintrag gibt es nicht")),
        Some(Some(_)) => return Err(ApiError::conflict("Der Eintrag ist schon geschlossen")),
        Some(None) => {}
    }
    conn.execute(
        "UPDATE unexplained SET closed_at = ?1, closed_by = ?2, close_note = ?3 WHERE id = ?4",
        params![clock.now(), user.id, note.trim(), entry],
    )?;
    Ok(Json(json!({ "id": entry, "closed": true })))
}

// ---------------------------------------------------------------- Sonstiges

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "version": version::VERSION }))
}

/// Das Dashboard ist eine einzelne HTML-Datei und wird fest in die .exe eingebaut.
async fn dashboard() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}
