use crate::{
    db::{Db, now},
    error::{ApiError, ApiResult},
};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, Query, State},
    http::{HeaderMap, header::AUTHORIZATION, request::Parts},
    response::Html,
    routing::{get, post},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub fn router(db: Db) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/api/health", get(health))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
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
        .route("/api/companies/{id}/events", post(post_event))
        .route("/api/companies/{id}/ledger", get(get_ledger))
        .route("/api/companies/{id}/transfer", post(transfer))
        .route("/api/trucks/{id}/checkout", post(checkout))
        .route("/api/trucks/{id}/checkin", post(checkin))
        .with_state(db)
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

// ---------------------------------------------------------------- Anmeldung

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

pub struct AuthUser {
    pub id: i64,
    pub name: String,
}

impl FromRequestParts<Db> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, db: &Db) -> Result<Self, Self::Rejection> {
        let token = bearer(&parts.headers).ok_or_else(ApiError::unauthorized)?;
        let conn = db.lock().unwrap();
        conn.query_row(
            "SELECT u.id, u.name FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?1",
            [hash_token(token)],
            |r| Ok(AuthUser { id: r.get(0)?, name: r.get(1)? }),
        )
        .optional()?
        .ok_or_else(ApiError::unauthorized)
    }
}

#[derive(Deserialize)]
struct Credentials {
    name: String,
    password: String,
}

fn issue_token(conn: &Connection, user_id: i64) -> rusqlite::Result<String> {
    let token = random_hex(32);
    conn.execute(
        "INSERT INTO tokens (token_hash, user_id, created_at) VALUES (?1, ?2, ?3)",
        params![hash_token(&token), user_id, now()],
    )?;
    Ok(token)
}

async fn register(State(db): State<Db>, Json(c): Json<Credentials>) -> ApiResult<Json<Value>> {
    let name = c.name.trim();
    if name.len() < 3 || name.len() > 32 {
        return Err(ApiError::bad("Der Name braucht 3 bis 32 Zeichen"));
    }
    if c.password.len() < 8 {
        return Err(ApiError::bad("Das Passwort braucht mindestens 8 Zeichen"));
    }
    let pw_hash = Argon2::default()
        .hash_password(c.password.as_bytes())
        .map_err(|_| ApiError::bad("Das Passwort konnte nicht verarbeitet werden"))?
        .to_string();

    let conn = db.lock().unwrap();
    match conn.execute(
        "INSERT INTO users (name, pw_hash, created_at) VALUES (?1, ?2, ?3)",
        params![name, pw_hash, now()],
    ) {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::conflict("Der Name ist schon vergeben"));
        }
        Err(e) => return Err(e.into()),
    }
    let user_id = conn.last_insert_rowid();
    let token = issue_token(&conn, user_id)?;
    Ok(Json(
        json!({ "user_id": user_id, "name": name, "token": token }),
    ))
}

async fn login(State(db): State<Db>, Json(c): Json<Credentials>) -> ApiResult<Json<Value>> {
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
    let token = issue_token(&conn, user_id)?;
    Ok(Json(
        json!({ "user_id": user_id, "name": name, "token": token }),
    ))
}

/// Abmelden: Der benutzte Token wird ungültig.
async fn logout(
    State(db): State<Db>,
    _user: AuthUser,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let token = bearer(&headers).ok_or_else(ApiError::unauthorized)?;
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM tokens WHERE token_hash = ?1",
        [hash_token(token)],
    )?;
    Ok(Json(json!({ "ok": true })))
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
    wage_percent: Option<i64>,
}

async fn create_company(
    State(db): State<Db>,
    user: AuthUser,
    Json(b): Json<NewCompany>,
) -> ApiResult<Json<Value>> {
    let name = b.name.trim();
    if name.len() < 3 || name.len() > 48 {
        return Err(ApiError::bad("Der Firmenname braucht 3 bis 48 Zeichen"));
    }
    let wage = b.wage_percent.unwrap_or(30);
    if !(0..=100).contains(&wage) {
        return Err(ApiError::bad(
            "Der Lohnanteil muss zwischen 0 und 100 liegen",
        ));
    }
    let invite = random_hex(4).to_uppercase();
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    match tx.execute(
        "INSERT INTO companies (name, invite_code, wage_percent, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![name, invite, wage, now()],
    ) {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(ApiError::conflict("Eine Firma mit dem Namen gibt es schon")),
        Err(e) => return Err(e.into()),
    }
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO memberships (company_id, user_id, role, joined_at) VALUES (?1, ?2, 'chef', ?3)",
        params![id, user.id, now()],
    )?;
    tx.commit()?;
    Ok(Json(
        json!({ "id": id, "name": name, "wage_percent": wage, "invite_code": invite }),
    ))
}

#[derive(Deserialize)]
struct JoinReq {
    invite_code: String,
}

async fn join_company(
    State(db): State<Db>,
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
        params![id, user.id, now()],
    ) {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(ApiError::conflict("Du bist schon in dieser Firma")),
        Err(e) => return Err(e.into()),
    }
    Ok(Json(
        json!({ "company_id": id, "name": name, "role": "fahrer" }),
    ))
}

async fn get_company(
    State(db): State<Db>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    let role = role_in(&conn, id, user.id)?;
    let (name, wage, invite): (String, i64, String) = conn.query_row(
        "SELECT name, wage_percent, invite_code FROM companies WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut stmt = conn.prepare(
        "SELECT u.id, u.name, m.role,
                (SELECT COALESCE(SUM(amount), 0) FROM ledger l
                  WHERE l.company_id = m.company_id AND l.account = 'fahrer' AND l.user_id = u.id)
         FROM memberships m JOIN users u ON u.id = m.user_id
         WHERE m.company_id = ?1 ORDER BY m.role, u.name",
    )?;
    let members = stmt
        .query_map([id], |r| {
            Ok(json!({
                "user_id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?,
                "role": r.get::<_, String>(2)?, "wallet": r.get::<_, i64>(3)?
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = json!({
        "id": id, "name": name, "wage_percent": wage, "your_role": role,
        "balance": company_balance(&conn, id)?, "members": members
    });
    // Den Einladungscode sieht nur der Chef.
    if role == "chef" {
        out["invite_code"] = json!(invite);
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
struct CompanyPatch {
    wage_percent: i64,
}

async fn update_company(
    State(db): State<Db>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(b): Json<CompanyPatch>,
) -> ApiResult<Json<Value>> {
    if !(0..=100).contains(&b.wage_percent) {
        return Err(ApiError::bad(
            "Der Lohnanteil muss zwischen 0 und 100 liegen",
        ));
    }
    let conn = db.lock().unwrap();
    require_chef(&conn, id, user.id)?;
    conn.execute(
        "UPDATE companies SET wage_percent = ?1 WHERE id = ?2",
        [b.wage_percent, id],
    )?;
    Ok(Json(json!({ "id": id, "wage_percent": b.wage_percent })))
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
    let conn = db.lock().unwrap();
    require_chef(&conn, company_id, user.id)?;
    match conn.execute(
        "INSERT INTO trucks (company_id, plate, model, package, odometer_km, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            company_id,
            plate,
            model,
            b.package,
            b.odometer_km,
            state,
            now()
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
    Ok(Json(json!({
        "id": conn.last_insert_rowid(), "plate": plate, "model": model,
        "odometer_km": b.odometer_km, "rev": 1
    })))
}

async fn list_trucks(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let conn = db.lock().unwrap();
    role_in(&conn, company_id, user.id)?;
    let mut stmt = conn.prepare(
        "SELECT t.id, t.plate, t.model, t.odometer_km, t.rev, u.id, u.name, l.taken_at
         FROM trucks t
         LEFT JOIN leases l ON l.truck_id = t.id AND l.returned_at IS NULL
         LEFT JOIN users u ON u.id = l.user_id
         WHERE t.company_id = ?1 ORDER BY t.plate",
    )?;
    let trucks = stmt
        .query_map([company_id], |r| {
            let holder: Option<i64> = r.get(5)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "plate": r.get::<_, String>(1)?, "model": r.get::<_, String>(2)?,
                "odometer_km": r.get::<_, i64>(3)?, "rev": r.get::<_, i64>(4)?,
                "leased_by": holder.map(|id| json!({
                    "user_id": id,
                    "name": r.get::<_, String>(6).unwrap_or_default(),
                    "since": r.get::<_, i64>(7).unwrap_or_default()
                }))
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!({ "trucks": trucks })))
}

/// Lkw ausleihen. Geht nur, wenn er frei ist. Liefert Bauplan und Zustand für den Spielstand.
async fn checkout(
    State(db): State<Db>,
    user: AuthUser,
    Path(truck_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    let (company_id, plate, model, package, odometer, state, rev): (i64, String, String, String, i64, String, i64) = tx
        .query_row(
            "SELECT company_id, plate, model, package, odometer_km, state, rev FROM trucks WHERE id = ?1",
            [truck_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diesen Lkw gibt es nicht"))?;
    role_in(&tx, company_id, user.id)?;
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
    tx.execute(
        "INSERT INTO leases (truck_id, user_id, rev_out, taken_at) VALUES (?1, ?2, ?3, ?4)",
        params![truck_id, user.id, rev, now()],
    )?;
    let lease_id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(Json(json!({
        "lease_id": lease_id,
        "truck": {
            "id": truck_id, "plate": plate, "model": model, "package": package,
            "odometer_km": odometer, "rev": rev,
            "state": serde_json::from_str::<Value>(&state).unwrap_or_else(|_| json!({}))
        }
    })))
}

#[derive(Deserialize)]
struct CheckinReq {
    odometer_km: i64,
    state: Option<Value>,
}

/// Lkw zurückgeben. Der Zustand aus dem Spielstand wird übernommen, die Revision steigt.
async fn checkin(
    State(db): State<Db>,
    user: AuthUser,
    Path(truck_id): Path<i64>,
    Json(b): Json<CheckinReq>,
) -> ApiResult<Json<Value>> {
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    let (company_id, odometer): (i64, i64) = tx
        .query_row(
            "SELECT company_id, odometer_km FROM trucks WHERE id = ?1",
            [truck_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diesen Lkw gibt es nicht"))?;
    let role = role_in(&tx, company_id, user.id)?;
    let (lease_id, holder): (i64, i64) = tx
        .query_row(
            "SELECT id, user_id FROM leases WHERE truck_id = ?1 AND returned_at IS NULL",
            [truck_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::conflict("Der Lkw ist gar nicht verliehen"))?;
    // Zurückgeben darf, wer ihn hat. Der Chef darf ihn zur Not für andere zurücknehmen.
    if holder != user.id && role != "chef" {
        return Err(ApiError::forbidden(
            "Den Lkw hat jemand anderes ausgeliehen",
        ));
    }
    if b.odometer_km < odometer {
        return Err(ApiError::bad(
            "Der Kilometerstand kann nicht kleiner werden",
        ));
    }
    match &b.state {
        Some(s) => tx.execute(
            "UPDATE trucks SET odometer_km = ?1, state = ?2, rev = rev + 1 WHERE id = ?3",
            params![b.odometer_km, s.to_string(), truck_id],
        )?,
        None => tx.execute(
            "UPDATE trucks SET odometer_km = ?1, rev = rev + 1 WHERE id = ?2",
            params![b.odometer_km, truck_id],
        )?,
    };
    tx.execute(
        "UPDATE leases SET returned_at = ?1 WHERE id = ?2",
        params![now(), lease_id],
    )?;
    let rev: i64 = tx.query_row("SELECT rev FROM trucks WHERE id = ?1", [truck_id], |r| {
        r.get(0)
    })?;
    tx.commit()?;
    Ok(Json(
        json!({ "id": truck_id, "odometer_km": b.odometer_km, "rev": rev, "driven_km": b.odometer_km - odometer }),
    ))
}

// ---------------------------------------------------------------- Kasse

#[derive(Deserialize)]
struct EventReq {
    /// Vom Client vergeben und eindeutig. Verhindert doppelte Buchungen.
    event_id: String,
    #[serde(rename = "type")]
    kind: String,
    /// Betrag in Euro, immer positiv. Bei job_delivered ist das der Erlös.
    amount: i64,
    #[serde(default)]
    note: String,
}

/// Ereignis aus dem Spiel buchen. Die Regeln:
/// - job_delivered: Lohnanteil an den Fahrer, der Rest in die Firmenkasse
/// - job_cancelled, fined: zahlt der Fahrer
/// - tollgate, ferry, train: zahlt die Firma
async fn post_event(
    State(db): State<Db>,
    user: AuthUser,
    Path(company_id): Path<i64>,
    Json(b): Json<EventReq>,
) -> ApiResult<Json<Value>> {
    let event_id = b.event_id.trim();
    if event_id.is_empty() || event_id.len() > 100 {
        return Err(ApiError::bad("event_id fehlt oder ist zu lang"));
    }
    if b.amount < 0 {
        return Err(ApiError::bad("Der Betrag darf nicht negativ sein"));
    }
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

    // (Konto, Buchungsart, Betrag mit Vorzeichen)
    let bookings: Vec<(&str, &str, i64)> = match b.kind.as_str() {
        "job_delivered" => {
            let percent: i64 = tx.query_row(
                "SELECT wage_percent FROM companies WHERE id = ?1",
                [company_id],
                |r| r.get(0),
            )?;
            let wage = b.amount * percent / 100;
            vec![
                ("fahrer", "lohn", wage),
                ("firma", "auftrag", b.amount - wage),
            ]
        }
        "job_cancelled" => vec![("fahrer", "auftrag_abgebrochen", -b.amount)],
        "fined" => vec![("fahrer", "strafe", -b.amount)],
        "tollgate" => vec![("firma", "maut", -b.amount)],
        "ferry" => vec![("firma", "faehre", -b.amount)],
        "train" => vec![("firma", "zug", -b.amount)],
        _ => return Err(ApiError::bad("Unbekannter Ereignistyp")),
    };
    for (account, kind, amount) in &bookings {
        tx.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, event_id, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![company_id, account, user.id, kind, amount, event_id, b.note, now()],
        )?;
    }
    let balance = company_balance(&tx, company_id)?;
    let my_wallet = wallet(&tx, company_id, user.id)?;
    tx.commit()?;
    let booked: Vec<Value> = bookings
        .iter()
        .map(
            |(account, kind, amount)| json!({ "account": account, "kind": kind, "amount": amount }),
        )
        .collect();
    Ok(Json(
        json!({ "duplicate": false, "booked": booked, "balance": balance, "wallet": my_wallet }),
    ))
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
            params![company_id, account, b.user_id, amount, b.note, now()],
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
}

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
        "SELECT l.id, l.account, u.name, l.kind, l.amount, l.note, l.created_at
         FROM ledger l LEFT JOIN users u ON u.id = l.user_id
         WHERE l.company_id = ?1 ORDER BY l.id DESC LIMIT ?2",
    )?;
    let entries = stmt
        .query_map([company_id, limit], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "account": r.get::<_, String>(1)?,
                "user": r.get::<_, Option<String>>(2)?, "kind": r.get::<_, String>(3)?,
                "amount": r.get::<_, i64>(4)?, "note": r.get::<_, String>(5)?,
                "created_at": r.get::<_, i64>(6)?
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(
        json!({ "balance": company_balance(&conn, company_id)?, "entries": entries }),
    ))
}

// ---------------------------------------------------------------- Sonstiges

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

/// Das Dashboard ist eine einzelne HTML-Datei und wird fest in die .exe eingebaut.
async fn dashboard() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}
