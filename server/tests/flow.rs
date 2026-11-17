//! Spielt den ganzen Ablauf einmal durch: zwei Nutzer, eine Firma, ein Lkw, Buchungen.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn app() -> Router {
    spedhub_server::api::router(spedhub_server::db::open(":memory:").unwrap())
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header("content-type", "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn register(app: &Router, name: &str) -> String {
    let (s, v) = call(
        app,
        "POST",
        "/api/register",
        None,
        Some(json!({ "name": name, "password": "geheim1234" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn ganzer_ablauf() {
    let app = app();
    let chef = register(&app, "Chef").await;
    let fahrer = register(&app, "Fahrer").await;
    let fremder = register(&app, "Fremder").await;

    // Firma gründen, Fahrer tritt per Einladungscode bei
    let (s, firma) = call(
        &app,
        "POST",
        "/api/companies",
        Some(&chef),
        Some(json!({ "name": "Nordfracht Logistik" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{firma}");
    let id = firma["id"].as_i64().unwrap();
    let code = firma["invite_code"].as_str().unwrap();
    let (s, _) = call(
        &app,
        "POST",
        "/api/companies/join",
        Some(&fahrer),
        Some(json!({ "invite_code": code })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(
        &app,
        "POST",
        "/api/companies/join",
        Some(&fahrer),
        Some(json!({ "invite_code": code })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Nur der Chef darf Lkw anlegen
    let truck = json!({ "plate": "SPED 001", "model": "scania.s_2016", "package": "vehicle : x {}", "odometer_km": 3865 });
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/trucks"),
        Some(&fahrer),
        Some(truck.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, t) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/trucks"),
        Some(&chef),
        Some(truck.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    let truck_id = t["id"].as_i64().unwrap();
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/trucks"),
        Some(&chef),
        Some(truck),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Ausleihen: der Fahrer bekommt den Bauplan, der Chef danach nicht mehr
    let (s, out) = call(
        &app,
        "POST",
        &format!("/api/trucks/{truck_id}/checkout"),
        Some(&fahrer),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["truck"]["package"], "vehicle : x {}");
    assert_eq!(out["truck"]["rev"], 1);
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/trucks/{truck_id}/checkout"),
        Some(&chef),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("Fahrer"));
    let (_, list) = call(
        &app,
        "GET",
        &format!("/api/companies/{id}/trucks"),
        Some(&chef),
        None,
    )
    .await;
    assert_eq!(list["trucks"][0]["leased_by"]["name"], "Fahrer");

    // Auftrag: der ganze Erlös in die Kasse. Doppelt geschickt wird nur einmal gebucht.
    let job = json!({ "event_id": "job-1", "type": "job_delivered", "amount": 1000, "distance_km": 523.4 });
    let (s, r) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/events"),
        Some(&fahrer),
        Some(job.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["wallet"].as_i64(), r["balance"].as_i64()),
        (Some(0), Some(1000))
    );
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "auftrag", "amount": 1000, "game_amount": 1000, "distance_km": 523 }])
    );
    let (_, r) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/events"),
        Some(&fahrer),
        Some(job),
    )
    .await;
    assert_eq!(r["duplicate"], true);
    assert_eq!(
        (r["wallet"].as_i64(), r["balance"].as_i64()),
        (Some(0), Some(1000))
    );

    // Strafe zahlt der Fahrer, Maut die Firma
    let fine = json!({ "event_id": "fine-1", "type": "fined", "amount": 100, "note": "speeding" });
    let (_, r) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/events"),
        Some(&fahrer),
        Some(fine),
    )
    .await;
    assert_eq!(r["wallet"], -100);
    let toll = json!({ "event_id": "toll-1", "type": "tollgate", "amount": 50 });
    let (_, r) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/events"),
        Some(&fahrer),
        Some(toll),
    )
    .await;
    assert_eq!(r["balance"], 950);
    let bad = json!({ "event_id": "x-1", "type": "geschenk", "amount": 5 });
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/events"),
        Some(&fahrer),
        Some(bad),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Rückgabe: Kilometer rauf, Revision rauf, Lkw wieder frei
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/trucks/{truck_id}/checkin"),
        Some(&fahrer),
        Some(json!({ "odometer_km": 100 })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let back =
        json!({ "odometer_km": 4200, "state": { "chassis_wear": 0.12, "fuel_relative": 0.5 } });
    let (s, r) = call(
        &app,
        "POST",
        &format!("/api/trucks/{truck_id}/checkin"),
        Some(&fahrer),
        Some(back),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["rev"].as_i64(), r["driven_km"].as_i64()),
        (Some(2), Some(335))
    );
    let (s, out) = call(
        &app,
        "POST",
        &format!("/api/trucks/{truck_id}/checkout"),
        Some(&chef),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(out["truck"]["state"]["chassis_wear"], 0.12);
    assert_eq!(out["truck"]["odometer_km"], 4200);

    // Übersicht und Journal
    let (_, c) = call(
        &app,
        "GET",
        &format!("/api/companies/{id}"),
        Some(&chef),
        None,
    )
    .await;
    assert_eq!(c["balance"], 950);
    assert_eq!(c["members"].as_array().unwrap().len(), 2);
    assert!(
        c.get("wage_percent").is_none(),
        "den Lohnanteil gibt es nicht mehr"
    );
    assert!(c["invite_code"].is_string());
    let (_, c) = call(
        &app,
        "GET",
        &format!("/api/companies/{id}"),
        Some(&fahrer),
        None,
    )
    .await;
    assert!(
        c["invite_code"].is_null(),
        "der Fahrer sieht den Code nicht"
    );
    let (_, l) = call(
        &app,
        "GET",
        &format!("/api/companies/{id}/ledger"),
        Some(&chef),
        None,
    )
    .await;
    assert_eq!(l["entries"].as_array().unwrap().len(), 3);

    // Überweisung: nur der Chef, nicht mehr als in der Kasse ist. Ein Lohnkonto darf ins Minus.
    let (_, ich) = call(&app, "GET", "/api/me", Some(&fahrer), None).await;
    let fahrer_id = ich["user_id"].as_i64().unwrap();
    let geld = |amount: i64| json!({ "user_id": fahrer_id, "amount": amount, "note": "Vorschuss" });
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/transfer"),
        Some(&fahrer),
        Some(geld(100)),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/transfer"),
        Some(&chef),
        Some(geld(951)),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, r) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/transfer"),
        Some(&chef),
        Some(geld(150)),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["balance"].as_i64(), r["wallet"].as_i64()),
        (Some(800), Some(50))
    );
    let (_, r) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/transfer"),
        Some(&chef),
        Some(geld(-400)),
    )
    .await;
    assert_eq!(
        (r["balance"].as_i64(), r["wallet"].as_i64()),
        (Some(1200), Some(-350))
    );
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/transfer"),
        Some(&chef),
        Some(json!({ "user_id": 999, "amount": 1 })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Fremde und Unangemeldete kommen nicht rein
    let (s, _) = call(
        &app,
        "GET",
        &format!("/api/companies/{id}"),
        Some(&fremder),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(
        &app,
        "GET",
        &format!("/api/companies/{id}"),
        Some("falsch"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = call(&app, "GET", &format!("/api/companies/{id}"), None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = call(&app, "GET", "/api/companies/999", Some(&chef), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn anmeldung() {
    let app = app();
    register(&app, "Alex").await;
    let (s, _) = call(
        &app,
        "POST",
        "/api/register",
        None,
        Some(json!({ "name": "alex", "password": "geheim1234" })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "Namen sind unabhängig von Groß- und Kleinschreibung eindeutig"
    );
    let (s, _) = call(
        &app,
        "POST",
        "/api/register",
        None,
        Some(json!({ "name": "Kurz", "password": "kurz" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({ "name": "Alex", "password": "falsch123" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = call(
        &app,
        "POST",
        "/api/login",
        None,
        Some(json!({ "name": "Alex", "password": "geheim1234" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, me) = call(
        &app,
        "GET",
        "/api/me",
        Some(v["token"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["name"], "Alex");

    // Nach dem Abmelden gilt der Token nicht mehr
    let token = v["token"].as_str().unwrap();
    let (s, _) = call(&app, "POST", "/api/logout", Some(token), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, "GET", "/api/me", Some(token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// Anmelden mit Header `X-SpedHub-Version`, wie der Launcher es tut.
async fn login_with_version(
    app: &Router,
    path: &str,
    name: &str,
    version: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(v) = version {
        req = req.header("x-spedhub-version", v);
    }
    let body = json!({ "name": name, "password": "geheim1234" }).to_string();
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn versionen_beim_anmelden_vergleichen() {
    let app = app();
    let own = env!("CARGO_PKG_VERSION");
    let mut parts = own.split('.');
    let (major, minor): (u64, u64) = (
        parts.next().unwrap().parse().unwrap(),
        parts.next().unwrap().parse().unwrap(),
    );
    let same_minor = format!("{major}.{minor}.99");
    let next_minor = format!("{major}.{}.0", minor + 1);

    // Health nennt die Version
    let (s, v) = call(&app, "GET", "/api/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["version"], own);

    // Konto anlegen mit passender Version, auch mit v davor
    let (s, v) = login_with_version(&app, "/api/register", "Alex", Some(&format!("v{own}"))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["server_version"], own);
    assert_eq!(v["compatible"], true);
    assert!(v["token"].is_string());

    // Gleiche Haupt- und Nebenversion passt, eine andere Nebenversion nicht
    let (s, v) = login_with_version(&app, "/api/login", "Alex", Some(&same_minor)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["compatible"], true);
    let (s, v) = login_with_version(&app, "/api/login", "Alex", Some(&next_minor)).await;
    assert_eq!(s, StatusCode::OK, "Anmelden geht trotzdem: {v}");
    assert_eq!(v["compatible"], false);
    assert_eq!(v["server_version"], own);
    let (_, v) = login_with_version(&app, "/api/login", "Alex", Some("0.1.0")).await;
    assert_eq!(v["compatible"], own.starts_with("0.1."));
    let (_, v) = login_with_version(&app, "/api/login", "Alex", Some("Unsinn")).await;
    assert_eq!(v["compatible"], false);

    // Ohne Header (Dashboard, älterer Launcher) gibt es nichts zu vergleichen
    let (_, v) = login_with_version(&app, "/api/login", "Alex", None).await;
    assert_eq!(v["compatible"], true);
    assert_eq!(v["server_version"], own);
    let (_, v) = login_with_version(&app, "/api/register", "Berta", Some(&next_minor)).await;
    assert_eq!(v["compatible"], false);
}

#[tokio::test]
async fn dashboard_wird_ausgeliefert() {
    let app = app();
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Kassenbuch"));
    assert!(html.contains(r#"betriebskosten_korrektur: "Betriebskosten, Korrektur""#));
    assert!(html.contains("Herzschlag senden") && html.contains("Passwort ändern"));
}

#[tokio::test]
async fn kennzeichen_wechselt_bei_der_rueckgabe() {
    let app = app();
    let chef = register(&app, "Chef").await;
    let fahrer = register(&app, "Fahrer").await;
    let (_, firma) = call(
        &app,
        "POST",
        "/api/companies",
        Some(&chef),
        Some(json!({ "name": "Nordfracht" })),
    )
    .await;
    let id = firma["id"].as_i64().unwrap();
    call(
        &app,
        "POST",
        "/api/companies/join",
        Some(&fahrer),
        Some(json!({ "invite_code": firma["invite_code"] })),
    )
    .await;
    let mut ids = Vec::new();
    for plate in ["SPED 001", "SPED 002"] {
        let (s, t) = call(
            &app,
            "POST",
            &format!("/api/companies/{id}/trucks"),
            Some(&chef),
            Some(json!({ "plate": plate, "model": "scania.s_2016", "odometer_km": 1000 })),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{t}");
        ids.push(t["id"].as_i64().unwrap());
    }
    let truck = ids[0];
    let checkout = |token: String| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/api/trucks/{truck}/checkout"),
                Some(&token),
                None,
            )
            .await
        }
    };
    let checkin = |token: String, body: Value| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/api/trucks/{truck}/checkin"),
                Some(&token),
                Some(body),
            )
            .await
        }
    };
    let list = |token: String| {
        let app = app.clone();
        async move {
            let (_, l) = call(
                &app,
                "GET",
                &format!("/api/companies/{id}/trucks"),
                Some(&token),
                None,
            )
            .await;
            l["trucks"].as_array().unwrap().clone()
        }
    };

    assert_eq!(checkout(fahrer.clone()).await.0, StatusCode::OK);

    // Kennzeichen, das ein anderer Lkw der Firma hat: abgelehnt, und die ganze Rückgabe mit
    let (s, v) = checkin(
        fahrer.clone(),
        json!({ "odometer_km": 1200, "plate": "SPED 002" }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(
        v["error"],
        "Das Kennzeichen SPED 002 hat in der Firma schon ein anderer Lkw"
    );
    let (s, _) = checkin(
        fahrer.clone(),
        json!({ "odometer_km": 1200, "plate": "  " }),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let l = list(chef.clone()).await;
    assert_eq!(l[0]["leased_by"]["name"], "Fahrer", "noch verliehen");
    assert_eq!(l[0]["odometer_km"], 1000);

    // Neues Kennzeichen vom Spiel: übernommen, das alte bleibt in der Liste
    let (s, r) = checkin(
        fahrer.clone(),
        json!({ "odometer_km": 1200, "plate": " EF XU 577 " }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["plate"], "EF XU 577");
    let l = list(fahrer.clone()).await;
    let t = l.iter().find(|t| t["id"] == truck).unwrap();
    assert_eq!(t["plate"], "EF XU 577");
    assert_eq!(t["old_plates"], json!(["SPED 001"]));
    assert_eq!(t["odometer_km"], 1200);
    let other = l.iter().find(|t| t["id"] == ids[1]).unwrap();
    assert_eq!(other["old_plates"], json!([]));

    // Eindeutig ist nur das aktuelle Kennzeichen: SPED 001 ist wieder frei
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/trucks"),
        Some(&chef),
        Some(json!({ "plate": "SPED 001", "model": "volvo.fh_2024" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Gleiches Kennzeichen oder keins: nichts ändert sich an der Liste
    checkout(fahrer.clone()).await;
    let (_, r) = checkin(
        fahrer.clone(),
        json!({ "odometer_km": 1300, "plate": "EF XU 577" }),
    )
    .await;
    assert_eq!(r["plate"], "EF XU 577");
    checkout(fahrer.clone()).await;
    checkin(fahrer.clone(), json!({ "odometer_km": 1400 })).await;
    // Zweiter Wechsel: beide alten Kennzeichen, ältestes zuerst
    checkout(fahrer.clone()).await;
    checkin(
        fahrer.clone(),
        json!({ "odometer_km": 1500, "plate": "EF AB 123" }),
    )
    .await;
    let l = list(chef.clone()).await;
    let t = l.iter().find(|t| t["id"] == truck).unwrap();
    assert_eq!(t["plate"], "EF AB 123");
    assert_eq!(t["old_plates"], json!(["SPED 001", "EF XU 577"]));
    // Das neue Kennzeichen kommt auch beim Ausleihen mit
    let (_, out) = checkout(chef.clone()).await;
    assert_eq!(out["truck"]["plate"], "EF AB 123");
}

// ------------------------------------------------------------ Lohnmodell und Geldabgleich

use spedhub_server::{clock::Clock, clock::berlin, db::Db};

/// Eine Firma mit Chef und Fahrer und gestellter Uhr. Die Uhr steht auf Montag, 16.11.2026,
/// 10:00 in Berlin.
struct Firma {
    app: Router,
    clock: Clock,
    db: Db,
    id: i64,
    chef: String,
    fahrer: String,
    chef_id: i64,
    fahrer_id: i64,
}

impl Firma {
    async fn new() -> Firma {
        Firma::with_db(spedhub_server::db::open(":memory:").unwrap()).await
    }

    async fn with_db(db: Db) -> Firma {
        let clock = Clock::fixed(berlin(2026, 11, 16, 10, 0));
        let app = spedhub_server::api::router_with(db.clone(), clock.clone());
        let chef = register(&app, "Chefin").await;
        let fahrer = register(&app, "Fahrer").await;
        let (_, firma) = call(
            &app,
            "POST",
            "/api/companies",
            Some(&chef),
            Some(json!({ "name": "Nordfracht" })),
        )
        .await;
        let id = firma["id"].as_i64().unwrap();
        call(
            &app,
            "POST",
            "/api/companies/join",
            Some(&fahrer),
            Some(json!({ "invite_code": firma["invite_code"] })),
        )
        .await;
        let (_, a) = call(&app, "GET", "/api/me", Some(&chef), None).await;
        let (_, b) = call(&app, "GET", "/api/me", Some(&fahrer), None).await;
        Firma {
            app,
            clock,
            db,
            id,
            chef,
            fahrer,
            chef_id: a["user_id"].as_i64().unwrap(),
            fahrer_id: b["user_id"].as_i64().unwrap(),
        }
    }

    async fn get(&self, token: &str, path: &str) -> (StatusCode, Value) {
        call(&self.app, "GET", path, Some(token), None).await
    }

    async fn post(&self, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
        call(&self.app, "POST", path, Some(token), Some(body)).await
    }

    async fn company(&self, token: &str) -> Value {
        let (s, c) = self
            .get(token, &format!("/api/companies/{}", self.id))
            .await;
        assert_eq!(s, StatusCode::OK, "{c}");
        c
    }

    async fn member(&self, user_id: i64) -> Value {
        let c = self.company(&self.chef).await;
        c["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["user_id"] == user_id)
            .unwrap()
            .clone()
    }

    async fn balance(&self) -> i64 {
        self.company(&self.chef).await["balance"].as_i64().unwrap()
    }

    async fn wallet(&self, user_id: i64) -> i64 {
        self.member(user_id).await["wallet"].as_i64().unwrap()
    }

    async fn event(
        &self,
        token: &str,
        event_id: &str,
        kind: &str,
        amount: i64,
        km: Option<f64>,
    ) -> Value {
        self.event_at(token, event_id, kind, amount, km, None).await
    }

    /// Ereignis mit Zeitpunkt im Spiel, wie es das Plugin schickt
    async fn event_at(
        &self,
        token: &str,
        event_id: &str,
        kind: &str,
        amount: i64,
        km: Option<f64>,
        occurred_at: Option<i64>,
    ) -> Value {
        let mut body = json!({ "event_id": event_id, "type": kind, "amount": amount });
        if let Some(km) = km {
            body["distance_km"] = json!(km);
        }
        if let Some(t) = occurred_at {
            body["occurred_at"] = json!(t);
        }
        let (s, r) = self
            .post(token, &format!("/api/companies/{}/events", self.id), body)
            .await;
        assert_eq!(s, StatusCode::OK, "{r}");
        r
    }

    async fn patch(&self, token: &str, body: Value) -> (StatusCode, Value) {
        call(
            &self.app,
            "PATCH",
            &format!("/api/companies/{}", self.id),
            Some(token),
            Some(body),
        )
        .await
    }

    /// Was der Hintergrund-Task des Servers jede Minute tut
    fn run_due(&self) -> usize {
        let mut conn = self.db.lock().unwrap();
        spedhub_server::payroll::run_due_all(&mut conn, self.clock.now()).unwrap()
    }

    async fn payrolls(&self) -> Vec<Value> {
        let (s, p) = self
            .get(&self.chef, &format!("/api/companies/{}/payrolls", self.id))
            .await;
        assert_eq!(s, StatusCode::OK, "{p}");
        p["payrolls"].as_array().unwrap().clone()
    }

    async fn sync(&self, token: &str, money: i64, xp: Option<i64>) -> Value {
        self.sync_at(token, money, xp, None).await
    }

    /// Abgleich mit Änderungszeit des gelesenen Spielstands, wie ihn der Launcher schickt
    async fn sync_at(
        &self,
        token: &str,
        money: i64,
        xp: Option<i64>,
        saved_at: Option<i64>,
    ) -> Value {
        let mut body = json!({ "ingame_money": money });
        if let Some(xp) = xp {
            body["xp"] = json!(xp);
        }
        if let Some(t) = saved_at {
            body["saved_at"] = json!(t);
        }
        let (s, r) = self
            .post(token, &format!("/api/companies/{}/sync", self.id), body)
            .await;
        assert_eq!(s, StatusCode::OK, "{r}");
        r
    }

    async fn written(&self, token: &str, sync: &Value) -> (StatusCode, Value) {
        self.written_at(token, sync, None).await
    }

    /// Bestätigung mit dem Zeitpunkt, zu dem der Launcher geschrieben hat
    async fn written_at(&self, token: &str, sync: &Value, at: Option<i64>) -> (StatusCode, Value) {
        let body = match at {
            Some(t) => json!({ "written_at": t }),
            None => json!({}),
        };
        self.post(
            token,
            &format!(
                "/api/companies/{}/sync/{}/written",
                self.id, sync["sync_id"]
            ),
            body,
        )
        .await
    }

    async fn ledger(&self) -> Vec<Value> {
        let (s, l) = self
            .get(
                &self.chef,
                &format!("/api/companies/{}/ledger?limit=500", self.id),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{l}");
        l["entries"].as_array().unwrap().clone()
    }

    /// Summe aller Buchungen der Arten `betriebskosten` und `betriebskosten_korrektur`
    async fn costs(&self) -> i64 {
        self.ledger()
            .await
            .iter()
            .filter(|e| e["kind"] == "betriebskosten" || e["kind"] == "betriebskosten_korrektur")
            .map(|e| e["amount"].as_i64().unwrap())
            .sum()
    }

    /// Alle ungeklärten Beträge, auch die geschlossenen, neueste zuerst
    async fn unexplained(&self) -> Vec<Value> {
        let (s, u) = self
            .get(
                &self.chef,
                &format!("/api/companies/{}/unexplained?all=true", self.id),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{u}");
        u["entries"].as_array().unwrap().clone()
    }

    fn at(&self, d: i64, h: i64, mi: i64) {
        self.clock.set(berlin(2026, 11, d, h, mi));
    }
}

fn line(run: &Value, user_id: i64) -> &Value {
    run["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["user_id"] == user_id)
        .unwrap_or_else(|| panic!("keine Zeile für {user_id} in {run}"))
}

#[tokio::test]
async fn bauplan_einer_laufenden_leihe_abrufen() {
    let f = Firma::new().await;
    let fremder = register(&f.app, "Fremder").await;
    let kollege = register(&f.app, "Kollege").await;
    let code = f.company(&f.chef).await["invite_code"].clone();
    f.post(
        &kollege,
        "/api/companies/join",
        json!({ "invite_code": code }),
    )
    .await;
    let (_, t) = f
        .post(
            &f.chef,
            &format!("/api/companies/{}/trucks", f.id),
            json!({ "plate": "SPED 001", "model": "scania.s_2016", "package": "vehicle : x {}",
                    "odometer_km": 3865, "state": { "engine_wear": 0.1 } }),
        )
        .await;
    let truck = t["id"].as_i64().unwrap();
    let path = format!("/api/trucks/{truck}/lease");

    // Nicht verliehen: nichts abzurufen
    let (s, _) = f.get(&f.fahrer, &path).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = f.get(&f.fahrer, "/api/trucks/999/lease").await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (_, out) = f
        .post(
            &f.fahrer,
            &format!("/api/trucks/{truck}/checkout"),
            json!({}),
        )
        .await;

    // Der Fahrer bekommt dasselbe wie beim Ausleihen
    let (s, l) = f.get(&f.fahrer, &path).await;
    assert_eq!(s, StatusCode::OK, "{l}");
    assert_eq!(l["truck"], out["truck"]);
    assert_eq!(l["lease_id"], out["lease_id"]);
    assert_eq!(l["user_id"], f.fahrer_id);
    assert_eq!(l["truck"]["package"], "vehicle : x {}");
    assert_eq!(l["truck"]["plate"], "SPED 001");
    assert_eq!(l["truck"]["state"]["engine_wear"], 0.1);

    // Der Chef darf auch, der Kollege und der Fremde nicht
    let (s, l) = f.get(&f.chef, &path).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(l["truck"]["package"], "vehicle : x {}");
    let (s, _) = f.get(&kollege, &path).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = f.get(&fremder, &path).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Nach der Rückgabe ist die Leihe vorbei
    f.post(
        &f.fahrer,
        &format!("/api/trucks/{truck}/checkin"),
        json!({ "odometer_km": 4000 }),
    )
    .await;
    let (s, _) = f.get(&f.fahrer, &path).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn auftrag_geht_ganz_in_die_kasse() {
    let f = Firma::new().await;
    let c = f.company(&f.chef).await;
    assert_eq!(c["revenue_factor"], 1.0);
    assert_eq!(c["daily_flat"], 50);
    assert_eq!(c["payroll_time"], "04:00");
    assert_eq!(c["advance"], 2000);
    assert_eq!(c["wage_levels"].as_array().unwrap().len(), 5);
    assert_eq!(c["wage_levels"][0]["cents_per_km"], 40);
    assert_eq!(c["wage_levels"][4]["cents_per_km"], 80);

    // Faktor 1: der gemeldete Erlös
    let r = f
        .event(&f.fahrer, "job-1", "job_delivered", 1234, Some(400.0))
        .await;
    assert_eq!(r["booked"][0]["amount"], 1234);
    assert_eq!(
        (r["balance"].as_i64(), r["wallet"].as_i64()),
        (Some(1234), Some(0))
    );

    // Faktor 100 für die Wirtschafts-Mod mit 1 Prozent Auszahlung
    let (s, _) = f.patch(&f.fahrer, json!({ "revenue_factor": 100 })).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    for bad in [json!(0), json!(-1), json!(20000)] {
        let (s, _) = f.patch(&f.chef, json!({ "revenue_factor": bad })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
    let (s, r) = f.patch(&f.chef, json!({ "revenue_factor": 100 })).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["revenue_factor"], 100.0);
    let r = f
        .event(&f.fahrer, "job-2", "job_delivered", 12, Some(380.0))
        .await;
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "auftrag", "amount": 1200, "game_amount": 12, "distance_km": 380 }])
    );
    assert_eq!(r["balance"], 2434);
    assert_eq!(r["wallet"], 0);

    // Strafe und Abbruch zahlt weiter der Fahrer, ohne Faktor
    let r = f.event(&f.fahrer, "fine-1", "fined", 30, None).await;
    assert_eq!(r["booked"][0]["game_amount"], -30);
    assert_eq!(r["wallet"], -30);
    let r = f.event(&f.fahrer, "tol-1", "tollgate", 8, Some(5.0)).await;
    assert_eq!(
        r["booked"][0]["distance_km"],
        Value::Null,
        "Kilometer zählen nur bei Aufträgen"
    );
    assert_eq!(r["balance"], 2426);

    // Firmenkilometer des Fahrers
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 780);

    // Ungültige Strecke
    let (s, _) = f
        .post(
            &f.fahrer,
            &format!("/api/companies/{}/events", f.id),
            json!({ "event_id": "job-x", "type": "job_delivered", "amount": 1, "distance_km": -5 }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Übrige Einstellungen
    let (s, r) = f
        .patch(
            &f.chef,
            json!({ "daily_flat": 70, "payroll_time": "5:30", "advance": 3000 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["daily_flat"].as_i64(), r["advance"].as_i64()),
        (Some(70), Some(3000))
    );
    assert_eq!(r["payroll_time"], "05:30");
    assert_eq!(r["revenue_factor"], 100.0, "nicht Mitgeschicktes bleibt");
    for bad in [
        json!({ "payroll_time": "25:00" }),
        json!({ "daily_flat": -1 }),
        json!({ "advance": -5 }),
    ] {
        let (s, _) = f.patch(&f.chef, bad).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn lohnstufe_wechselt_mit_kilometern_und_erfahrung() {
    let f = Firma::new().await;
    let stage = |m: &Value| (m["stage"].as_i64(), m["cents_per_km"].as_i64());

    // Neu: Stufe 1, ohne Abgleich Level 0
    let m = f.member(f.fahrer_id).await;
    assert_eq!(stage(&m), (Some(1), Some(40)));
    assert_eq!(
        (m["xp"].clone(), m["game_level"].as_i64()),
        (Value::Null, Some(0))
    );

    // 6000 km reichen für Stufe 2 nicht, solange das Level fehlt
    f.event(&f.fahrer, "job-1", "job_delivered", 100, Some(6000.0))
        .await;
    let m = f.member(f.fahrer_id).await;
    assert_eq!(m["company_km"], 6000);
    assert_eq!(stage(&m), (Some(1), Some(40)));

    // Abgleich mit 3300 Erfahrungspunkten: Level 5, jetzt Stufe 2
    let r = f.sync(&f.fahrer, 0, Some(3300)).await;
    assert_eq!(
        (r["game_level"].as_i64(), r["stage"].as_i64()),
        (Some(5), Some(2))
    );
    let m = f.member(f.fahrer_id).await;
    assert_eq!(
        (m["xp"].as_i64(), m["game_level"].as_i64()),
        (Some(3300), Some(5))
    );
    assert_eq!(stage(&m), (Some(2), Some(50)));

    // Level 10 (11100 Punkte), aber noch keine 20000 km: bleibt Stufe 2
    f.sync(&f.fahrer, 0, Some(11_100)).await;
    let m = f.member(f.fahrer_id).await;
    assert_eq!(m["game_level"], 10);
    assert_eq!(stage(&m), (Some(2), Some(50)));
    // Ein Abgleich ohne Erfahrung behält die letzte
    f.sync(&f.fahrer, 0, None).await;
    assert_eq!(f.member(f.fahrer_id).await["xp"], 11_100);

    // Mit den Kilometern dazu: Stufe 3
    f.event(&f.fahrer, "job-2", "job_delivered", 100, Some(14_000.0))
        .await;
    assert_eq!(stage(&f.member(f.fahrer_id).await), (Some(3), Some(60)));

    // Der Chef setzt eigene Stufen, der Fahrer darf das nicht
    let path = format!("/api/companies/{}/wage-levels", f.id);
    let levels = json!({ "levels": [
        { "min_level": 0, "min_km": 0, "cents_per_km": 30 },
        { "min_level": 10, "min_km": 0, "cents_per_km": 90 }
    ]});
    let (s, _) = call(&f.app, "PUT", &path, Some(&f.fahrer), Some(levels.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, r) = call(&f.app, "PUT", &path, Some(&f.chef), Some(levels)).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        r["wage_levels"][1],
        json!({ "stage": 2, "min_level": 10, "min_km": 0, "cents_per_km": 90 })
    );
    assert_eq!(stage(&f.member(f.fahrer_id).await), (Some(2), Some(90)));
    // Erfahrung zählt nur für den eigenen Abgleich: der Chef ist noch Level 0
    assert_eq!(stage(&f.member(f.chef_id).await), (Some(1), Some(30)));

    for bad in [
        json!({ "levels": [] }),
        json!({ "levels": [{ "min_level": -1, "min_km": 0, "cents_per_km": 30 }] }),
        json!({ "levels": [{ "min_level": 0, "min_km": 0, "cents_per_km": -3 }] }),
    ] {
        let (s, _) = call(&f.app, "PUT", &path, Some(&f.chef), Some(bad)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn tagesabrechnung_mit_genug_geld() {
    let f = Firma::new().await;
    // Montag: zwei Aufträge, zusammen 623 km
    f.event(&f.fahrer, "job-1", "job_delivered", 2000, Some(523.0))
        .await;
    f.at(16, 22, 30);
    f.event(&f.fahrer, "job-2", "job_delivered", 500, Some(100.0))
        .await;
    // Kurz nach Mitternacht zählt schon für Dienstag
    f.at(17, 0, 30);
    f.event(&f.fahrer, "job-3", "job_delivered", 300, Some(50.0))
        .await;
    assert_eq!(f.balance().await, 2800);

    // Vor 4:00 passiert nichts
    f.at(17, 3, 59);
    assert_eq!(f.run_due(), 0);
    assert!(f.payrolls().await.is_empty());

    // Um 4:00: Montag wird abgerechnet. 623 km mal 40 Cent = 249,20 €, dazu 50 € Pauschale
    f.at(17, 4, 0);
    assert_eq!(f.run_due(), 1);
    let runs = f.payrolls().await;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        (runs[0]["day"].as_str(), runs[0]["kind"].as_str()),
        (Some("2026-11-16"), Some("taeglich"))
    );
    let l = line(&runs[0], f.fahrer_id);
    assert_eq!(
        (
            l["km"].as_i64(),
            l["cents_per_km"].as_i64(),
            l["flat_days"].as_i64(),
            l["flat"].as_i64()
        ),
        (Some(623), Some(40), Some(1), Some(50))
    );
    assert_eq!(
        (
            l["wage"].as_i64(),
            l["paid"].as_i64(),
            l["claim_after"].as_i64()
        ),
        (Some(299), Some(299), Some(0))
    );
    assert_eq!(
        runs[0]["lines"].as_array().unwrap().len(),
        1,
        "die Chefin ist nicht gefahren"
    );
    assert_eq!(f.balance().await, 2800 - 299);
    assert_eq!(f.wallet(f.fahrer_id).await, 299);

    // Im Kassenbuch: eine Buchung für die Firma, eine aufs Lohnkonto, beide mit der Abrechnung
    let (_, l) = f
        .get(&f.chef, &format!("/api/companies/{}/ledger", f.id))
        .await;
    let lohn: Vec<&Value> = l["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "lohn")
        .collect();
    assert_eq!(lohn.len(), 2);
    assert!(lohn.iter().all(|e| e["payroll_id"] == runs[0]["id"]));
    assert_eq!(
        lohn.iter()
            .map(|e| e["amount"].as_i64().unwrap())
            .sum::<i64>(),
        0
    );

    // Derselbe Tag läuft nicht zweimal, auch nicht später am Tag
    assert_eq!(f.run_due(), 0);
    f.at(17, 23, 0);
    assert_eq!(f.run_due(), 0);
    assert_eq!(f.payrolls().await.len(), 1);

    // Am Mittwoch um 4:00 kommt der Dienstag: 50 km = 20 €, plus Pauschale
    f.at(18, 4, 0);
    assert_eq!(f.run_due(), 1);
    let runs = f.payrolls().await;
    assert_eq!(runs[0]["day"], "2026-11-17");
    assert_eq!(line(&runs[0], f.fahrer_id)["paid"], 70);
    assert_eq!(f.wallet(f.fahrer_id).await, 369);
}

#[tokio::test]
async fn tagesabrechnung_mit_zu_wenig_geld_und_offener_forderung() {
    let f = Firma::new().await;
    // Fahrer: 1000 km = 400 € + 50 €; Chefin: 500 km = 200 € + 50 €. In der Kasse nur 150 €.
    f.event(&f.fahrer, "job-1", "job_delivered", 100, Some(1000.0))
        .await;
    f.event(&f.chef, "job-2", "job_delivered", 50, Some(500.0))
        .await;
    f.at(17, 4, 0);
    assert_eq!(f.run_due(), 1);
    let runs = f.payrolls().await;
    // Jeder bekommt denselben Anteil: 150 von 700
    let a = line(&runs[0], f.fahrer_id);
    let b = line(&runs[0], f.chef_id);
    assert_eq!(
        (
            a["wage"].as_i64(),
            a["paid"].as_i64(),
            a["claim_after"].as_i64()
        ),
        (Some(450), Some(96), Some(354))
    );
    assert_eq!(
        (
            b["wage"].as_i64(),
            b["paid"].as_i64(),
            b["claim_after"].as_i64()
        ),
        (Some(250), Some(53), Some(197))
    );
    assert_eq!(f.balance().await, 1);
    let m = f.member(f.fahrer_id).await;
    assert_eq!(
        (m["wallet"].as_i64(), m["open_claim"].as_i64()),
        (Some(96), Some(354))
    );

    // Dienstag: großer Auftrag, die Forderungen werden zuerst bedient
    f.at(17, 12, 0);
    f.event(&f.fahrer, "job-3", "job_delivered", 2000, Some(100.0))
        .await;
    f.at(18, 4, 0);
    assert_eq!(f.run_due(), 1);
    let runs = f.payrolls().await;
    let a = line(&runs[0], f.fahrer_id);
    assert_eq!(
        (
            a["claim_before"].as_i64(),
            a["wage"].as_i64(),
            a["paid"].as_i64(),
            a["claim_after"].as_i64()
        ),
        (Some(354), Some(90), Some(444), Some(0))
    );
    let b = line(&runs[0], f.chef_id);
    assert_eq!(
        (
            b["claim_before"].as_i64(),
            b["wage"].as_i64(),
            b["paid"].as_i64(),
            b["claim_after"].as_i64()
        ),
        (Some(197), Some(0), Some(197), Some(0))
    );
    assert_eq!(f.balance().await, 1 + 2000 - 444 - 197);
    let m = f.member(f.fahrer_id).await;
    assert_eq!(
        (m["wallet"].as_i64(), m["open_claim"].as_i64()),
        (Some(540), Some(0))
    );

    // Wenn die Kasse leer ist, reicht es auch für die Forderung nur anteilig
    f.at(18, 12, 0);
    f.event(&f.fahrer, "job-4", "job_delivered", 0, Some(250.0))
        .await;
    f.post(
        &f.chef,
        &format!("/api/companies/{}/transfer", f.id),
        json!({ "user_id": f.chef_id, "amount": f.balance().await }),
    )
    .await;
    assert_eq!(f.balance().await, 0);
    f.at(19, 4, 0);
    f.run_due();
    let runs = f.payrolls().await;
    let a = line(&runs[0], f.fahrer_id);
    assert_eq!(
        (
            a["wage"].as_i64(),
            a["paid"].as_i64(),
            a["claim_after"].as_i64()
        ),
        (Some(150), Some(0), Some(150))
    );
}

#[tokio::test]
async fn verpasste_tage_werden_nachgeholt_auch_nach_neustart() {
    let path = std::env::temp_dir().join(format!("spedhub-flow-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let f = Firma::with_db(spedhub_server::db::open(path.to_str().unwrap()).unwrap()).await;
    f.event(&f.fahrer, "job-1", "job_delivered", 1000, Some(100.0))
        .await;
    f.at(17, 9, 0);
    f.event(&f.fahrer, "job-2", "job_delivered", 1000, Some(200.0))
        .await;

    // Der Server war bis Donnerstag 5:00 aus: Montag, Dienstag und Mittwoch kommen der Reihe nach
    f.at(19, 5, 0);
    assert_eq!(f.run_due(), 3);
    let mut runs = f.payrolls().await;
    runs.reverse();
    let days: Vec<&str> = runs.iter().map(|r| r["day"].as_str().unwrap()).collect();
    assert_eq!(days, ["2026-11-16", "2026-11-17", "2026-11-18"]);
    assert_eq!(line(&runs[0], f.fahrer_id)["paid"], 40 + 50);
    assert_eq!(line(&runs[1], f.fahrer_id)["paid"], 80 + 50);
    assert!(
        runs[2]["lines"].as_array().unwrap().is_empty(),
        "Mittwoch ohne Aufträge"
    );
    assert_eq!(f.wallet(f.fahrer_id).await, 220);

    // Neustart: dieselbe Datenbank neu öffnen, nichts läuft doppelt
    drop(f.db.lock().unwrap());
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    let mut conn = db.lock().unwrap();
    assert_eq!(
        spedhub_server::payroll::run_due_all(&mut conn, berlin(2026, 11, 19, 6, 0)).unwrap(),
        0
    );
    // Am Freitag um 4:00 genau der Donnerstag
    assert_eq!(
        spedhub_server::payroll::run_due_all(&mut conn, berlin(2026, 11, 20, 4, 0)).unwrap(),
        1
    );
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM payroll_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 4);
    drop(conn);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn abrechnung_von_hand_und_danach_taeglich() {
    let f = Firma::new().await;
    f.patch(&f.chef, json!({ "payroll_time": "04:00" })).await;
    f.event(&f.fahrer, "job-1", "job_delivered", 1000, Some(100.0))
        .await;
    let path = format!("/api/companies/{}/payrolls/run", f.id);
    let (s, _) = f.post(&f.fahrer, &path, json!({})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Sofort: der Auftrag von heute mit Pauschale
    f.at(16, 11, 0);
    let (s, r) = f.post(&f.chef, &path, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let runs = r["payrolls"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["kind"], "hand");
    assert_eq!(line(&runs[0], f.fahrer_id)["paid"], 90);

    // Später am Tag noch ein Auftrag. Die tägliche Abrechnung zahlt nur ihn, ohne zweite Pauschale.
    f.at(16, 15, 0);
    f.event(&f.fahrer, "job-2", "job_delivered", 1000, Some(200.0))
        .await;
    f.at(17, 4, 0);
    assert_eq!(f.run_due(), 1);
    let runs = f.payrolls().await;
    let l = line(&runs[0], f.fahrer_id);
    assert_eq!(
        (
            l["km"].as_i64(),
            l["flat_days"].as_i64(),
            l["paid"].as_i64()
        ),
        (Some(200), Some(0), Some(80))
    );
    assert_eq!(f.wallet(f.fahrer_id).await, 170);
}

#[tokio::test]
async fn geldabgleich() {
    let f = Firma::new().await;
    let fremder = register(&f.app, "Fremder").await;
    let (s, _) = f
        .post(
            &fremder,
            &format!("/api/companies/{}/sync", f.id),
            json!({ "ingame_money": 5 }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Erster Abgleich: bucht nichts, der Launcher schreibt Lohnkonto (0) plus Vorschuss (2000)
    let r = f.sync(&f.fahrer, 48_500, Some(0)).await;
    assert_eq!(r["first"], true);
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    assert_eq!(
        (
            r["wallet"].as_i64(),
            r["advance"].as_i64(),
            r["write_money"].as_i64()
        ),
        (Some(0), Some(2000), Some(2000))
    );
    let (s, _) = f.written(&f.fahrer, &r).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(f.balance().await, 0);

    // Gleich noch einmal: alles passt, nichts wird gebucht
    let r = f.sync(&f.fahrer, 2000, None).await;
    assert_eq!(
        (r["first"].as_bool(), r["expected"].as_i64()),
        (Some(false), Some(2000))
    );
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    f.written(&f.fahrer, &r).await;

    // Im Spiel: Auftrag +1500, Strafe -100, Maut -30, Abbruch -70, und 200 € fürs Tanken.
    // Die Uhr steht dabei still: Ohne Zeitpunkt zählt die Reihenfolge des Empfangs.
    f.event(&f.fahrer, "job-1", "job_delivered", 1500, Some(300.0))
        .await;
    f.event(&f.fahrer, "fine-1", "fined", 100, None).await;
    f.event(&f.fahrer, "toll-1", "tollgate", 30, None).await;
    f.event(&f.fahrer, "cancel-1", "job_cancelled", 70, None)
        .await;
    // Ereignisse der Chefin zählen nicht für den Fahrer
    f.event(&f.chef, "job-c", "job_delivered", 999, Some(10.0))
        .await;
    let r = f
        .sync(&f.fahrer, 2000 + 1500 - 100 - 30 - 70 - 200, None)
        .await;
    assert_eq!(
        (r["expected"].as_i64(), r["events"].as_i64()),
        (Some(3300), Some(4))
    );
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -200 }])
    );
    // Strafe und Abbruch zahlt der Fahrer: 2000 - 170
    assert_eq!(
        (r["wallet"].as_i64(), r["write_money"].as_i64()),
        (Some(-170), Some(1830))
    );
    assert_eq!(f.balance().await, 1500 + 999 - 30 - 200);
    f.written(&f.fahrer, &r).await;

    // Mehr Geld als erklärt: nichts gutschreiben, ungeklärt für die Chefin
    let r = f.sync(&f.fahrer, 1830 + 500, None).await;
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"]["amount"], 500);
    assert_eq!(r["write_money"], 1830);
    assert_eq!(f.balance().await, 2269);
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 1);
    assert!(f.company(&f.fahrer).await.get("open_unexplained").is_none());
    let list = format!("/api/companies/{}/unexplained", f.id);
    let (s, _) = f.get(&f.fahrer, &list).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, u) = f.get(&f.chef, &list).await;
    let e = &u["entries"][0];
    assert_eq!(
        (
            e["user"].as_str(),
            e["amount"].as_i64(),
            e["expected"].as_i64(),
            e["ingame_money"].as_i64()
        ),
        (Some("Fahrer"), Some(500), Some(1830), Some(2330))
    );
    let close = format!("{list}/{}/close", e["id"]);
    let (s, _) = f.post(&f.fahrer, &close, json!({})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = f
        .post(&f.chef, &close, json!({ "note": "Geschenk im Spiel" }))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = f.post(&f.chef, &close, json!({})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, u) = f.get(&f.chef, &list).await;
    assert!(u["entries"].as_array().unwrap().is_empty());
    let (_, u) = f.get(&f.chef, &format!("{list}?all=true")).await;
    assert_eq!(u["entries"][0]["closed_by"], "Chefin");
    assert_eq!(u["entries"][0]["note"], "Geschenk im Spiel");
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 0);

    // Dieser Abgleich wurde nicht geschrieben (Spiel lief): der nächste rechnet vom gelesenen
    // Stand 2330 weiter, nicht von 1830
    let r = f.sync(&f.fahrer, 2330, None).await;
    assert_eq!(r["expected"], 2330);
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);

    // Bestätigen geht nur für den eigenen und neuesten Abgleich
    let alt = json!({ "sync_id": r["sync_id"].as_i64().unwrap() - 1 });
    let (s, _) = f.written(&f.fahrer, &alt).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = f.written(&f.chef, &r).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = f.written(&f.fahrer, &json!({ "sync_id": 999 })).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Lohnkonto im Minus und kleiner Vorschuss: geschrieben wird nie weniger als 0
    f.patch(&f.chef, json!({ "advance": 100 })).await;
    let r = f.sync(&f.fahrer, 2330, None).await;
    assert_eq!(r["write_money"], 0);
}

#[tokio::test]
async fn alte_datenbank_mit_lohnanteil_wird_uebernommen() {
    // Eine Datenbank aus der Zeit des Lohnanteils: Auftrag mit 30 % Lohn, 70 % Kasse
    let path = std::env::temp_dir().join(format!("spedhub-alt-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                pw_hash TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE companies (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                invite_code TEXT NOT NULL UNIQUE,
                wage_percent INTEGER NOT NULL DEFAULT 30 CHECK (wage_percent BETWEEN 0 AND 100),
                created_at INTEGER NOT NULL);
             CREATE TABLE memberships (company_id INTEGER NOT NULL, user_id INTEGER NOT NULL,
                role TEXT NOT NULL, joined_at INTEGER NOT NULL, PRIMARY KEY (company_id, user_id));
             CREATE TABLE ledger (id INTEGER PRIMARY KEY, company_id INTEGER NOT NULL,
                account TEXT NOT NULL CHECK (account IN ('firma', 'fahrer')), user_id INTEGER,
                kind TEXT NOT NULL, amount INTEGER NOT NULL, event_id TEXT,
                note TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL);
             INSERT INTO users VALUES (1, 'Alt', 'x', 1000);
             INSERT INTO companies VALUES (1, 'Altfracht', 'ABCD1234', 30, 1000);
             INSERT INTO memberships VALUES (1, 1, 'chef', 1000);
             INSERT INTO ledger VALUES (1, 1, 'fahrer', 1, 'lohn', 300, 'job-1', '', 2000);
             INSERT INTO ledger VALUES (2, 1, 'firma', 1, 'auftrag', 700, 'job-1', '', 2000);",
        )
        .unwrap();
    }
    let before = berlin(2026, 11, 16, 10, 0);
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    let conn = db.lock().unwrap();
    // Die alten Buchungen sind unverändert, die neuen Spalten leer
    type Row = (String, String, i64, Option<i64>, Option<i64>);
    let rows: Vec<Row> = conn
        .prepare("SELECT account, kind, amount, game_amount, payroll_id FROM ledger ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("fahrer".into(), "lohn".into(), 300, None, None),
            ("firma".into(), "auftrag".into(), 700, None, None)
        ]
    );
    // Die Firma bekommt die Standardstufen und wird erst ab jetzt abgerechnet
    let (levels, from): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM wage_levels WHERE company_id = 1), payroll_from FROM companies",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(levels, 5);
    assert!(
        from > before - 86_400 * 365 * 10,
        "payroll_from ist die Zeit der Migration"
    );
    // Ein zweites Öffnen ändert nichts mehr
    drop(conn);
    drop(db);
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    let conn = db.lock().unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM wage_levels", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 5);
    drop(conn);
    let _ = std::fs::remove_file(&path);
}

// ------------------------------------------------------------ Spät gemeldete Ereignisse

/// Montag, 16.11.2026, 10:00 in Berlin plus `m` Minuten. Alle Zeitpunkte in diesen Tests
/// stehen für die Uhr des Fahrer-PCs, die Serveruhr wird passend mitgestellt.
fn t(m: i64) -> i64 {
    berlin(2026, 11, 16, 10, 0) + m * 60
}

fn n(v: &Value) -> i64 {
    v.as_i64().unwrap_or_else(|| panic!("keine Zahl: {v}"))
}

/// Erster Abgleich um 10:00: Spielstand von 9:59 mit 2000 €, der Launcher schreibt um 10:00
/// Lohnkonto (0) plus Vorschuss (2000) und bestätigt.
async fn first_sync(f: &Firma) {
    f.clock.set(t(0));
    let r = f.sync_at(&f.fahrer, 2000, None, Some(t(-1))).await;
    assert_eq!(r["first"], true);
    assert_eq!(n(&r["saved_at"]), t(-1));
    let (s, w) = f.written_at(&f.fahrer, &r, Some(t(0))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(n(&w["written_at"]), t(0));
}

/// Abgleich um `m` + 1 Minuten mit einem Spielstand von Minute `m`, bestätigt.
async fn sync_written_at(f: &Firma, money: i64, m: i64) -> Value {
    f.clock.set(t(m + 1));
    let r = f.sync_at(&f.fahrer, money, None, Some(t(m))).await;
    let (s, w) = f.written_at(&f.fahrer, &r, Some(t(m + 1))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    r
}

#[tokio::test]
async fn spaet_gemeldeter_auftrag_mit_erloesfaktor_1() {
    let f = Firma::new().await;
    first_sync(&f).await;

    // Im Spiel um 10:30 ein Auftrag über 1000 €, gespeichert um 11:00. Das Plugin meldet ihn
    // erst nach dem Abgleich, etwa weil der Server kurz weg war.
    let r = sync_written_at(&f, 3000, 60).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"]["amount"], 1000);
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 1);

    f.clock.set(t(70));
    let e = f
        .event_at(
            &f.fahrer,
            "job-1",
            "job_delivered",
            1000,
            Some(300.0),
            Some(t(30)),
        )
        .await;
    // Gebucht wird wie immer, dazu die Korrektur des Abgleichs
    assert_eq!(e["booked"][0]["amount"], 1000);
    assert_eq!(n(&e["occurred_at"]), t(30));
    let late = &e["late"];
    assert_eq!(late["sync_id"], r["sync_id"]);
    assert_eq!((n(&late["diff_before"]), n(&late["diff"])), (1000, 0));
    assert_eq!(late["booked"], json!([]));
    assert_eq!(late["unexplained"]["closed"], true);
    let u = f.unexplained().await;
    assert_eq!(u.len(), 1);
    assert!(u[0]["closed_at"].is_i64());
    assert_eq!(u[0]["closed_by"], Value::Null);
    assert_eq!(u[0]["note"], "Durch spät gemeldete Ereignisse erklärt");
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 0);
    // Der Zeitpunkt steht im Kassenbuch
    let job = f
        .ledger()
        .await
        .into_iter()
        .find(|e| e["event_id"] == "job-1")
        .unwrap();
    assert_eq!(n(&job["occurred_at"]), t(30));
    assert_eq!(n(&job["created_at"]), t(70));

    // Noch einmal geschickt: nichts passiert, auch keine zweite Korrektur
    let e = f
        .event_at(
            &f.fahrer,
            "job-1",
            "job_delivered",
            1000,
            Some(300.0),
            Some(t(30)),
        )
        .await;
    assert_eq!(e["duplicate"], true);
    assert!(e.get("late").is_none());

    // Nächster Abgleich ohne Fahrt: Der Auftrag zählt nicht noch einmal
    let r = sync_written_at(&f, 2000, 119).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);

    // Die Firma hat den ganzen Erlös, Betriebskosten gab es keine
    assert_eq!(f.balance().await, 1000);
    assert_eq!(f.costs().await, 0);
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 300);
}

#[tokio::test]
async fn spaet_gemeldete_auftraege_mit_erloesfaktor_100() {
    let f = Firma::new().await;
    let (s, _) = f.patch(&f.chef, json!({ "revenue_factor": 100 })).await;
    assert_eq!(s, StatusCode::OK);
    first_sync(&f).await;

    // Die Mod zahlt im Spiel 1 Prozent: 10 € statt 1000 €
    let r = sync_written_at(&f, 2010, 60).await;
    assert_eq!(r["unexplained"]["amount"], 10);
    f.clock.set(t(65));
    let e = f
        .event_at(&f.fahrer, "job-1", "job_delivered", 10, None, Some(t(30)))
        .await;
    assert_eq!(e["booked"][0]["amount"], 1000);
    assert_eq!(e["booked"][0]["game_amount"], 10);
    assert_eq!(n(&e["late"]["diff"]), 0);
    assert_eq!(e["late"]["booked"], json!([]));
    assert_eq!(e["late"]["unexplained"]["closed"], true);

    // Zweites Fenster: wieder ein Auftrag über 10 € und 200 € fürs Tanken. Der Abgleich bucht
    // erst 190 € Betriebskosten, die Korrektur rechnet mit 10 €, nicht mit dem Kassenerlös.
    let r = sync_written_at(&f, 2000 + 10 - 200, 120).await;
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -190 }])
    );
    f.clock.set(t(125));
    let e = f
        .event_at(&f.fahrer, "job-2", "job_delivered", 10, None, Some(t(90)))
        .await;
    assert_eq!(e["booked"][0]["amount"], 1000);
    assert_eq!(
        (n(&e["late"]["diff_before"]), n(&e["late"]["diff"])),
        (-190, -200)
    );
    assert_eq!(
        e["late"]["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -10 }])
    );
    assert_eq!(e["late"]["unexplained"], Value::Null);

    assert_eq!(f.balance().await, 1000 + 1000 - 200);
    assert_eq!(f.costs().await, -200, "nur das Tanken");
    assert!(
        f.unexplained()
            .await
            .iter()
            .all(|u| u["closed_at"].is_i64())
    );
}

#[tokio::test]
async fn spaeter_auftrag_deckt_tanken_auf() {
    // Fall a: Tanken 200 €, Auftrag 1000 € kommt spät. Beim Abgleich sieht es nach 800 €
    // zu viel aus, nach der Korrektur sind es 200 € Betriebskosten.
    let f = Firma::new().await;
    first_sync(&f).await;
    let r = sync_written_at(&f, 2000 + 1000 - 200, 60).await;
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"]["amount"], 800);
    assert_eq!(f.costs().await, 0);

    f.clock.set(t(70));
    let e = f
        .event_at(&f.fahrer, "job-1", "job_delivered", 1000, None, Some(t(20)))
        .await;
    let late = &e["late"];
    assert_eq!((n(&late["diff_before"]), n(&late["diff"])), (800, -200));
    assert_eq!(
        late["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -200 }])
    );
    assert_eq!(late["unexplained"]["closed"], true);
    let kosten = f
        .ledger()
        .await
        .into_iter()
        .find(|e| e["kind"] == "betriebskosten")
        .unwrap();
    assert_eq!(
        kosten["note"],
        "Spät gemeldet: Auftrag, Abgleich vom 16.11.2026 11:01"
    );
    assert_eq!(kosten["user_id"], f.fahrer_id);
    assert_eq!(f.balance().await, 800);
    assert_eq!(f.costs().await, -200);
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 0);
}

#[tokio::test]
async fn mehrere_spaete_ereignisse_fuer_dasselbe_fenster() {
    // Fall d: Im Spiel zwei Aufträge (600 € und 400 €), Maut 50 €, Strafe 100 € und 200 €
    // fürs Tanken. Gemeldet wird alles erst nach dem Abgleich, nacheinander.
    let f = Firma::new().await;
    first_sync(&f).await;
    let r = sync_written_at(&f, 2000 + 600 + 400 - 50 - 100 - 200, 60).await;
    assert_eq!(r["unexplained"]["amount"], 650);
    let entry = r["unexplained"]["id"].clone();

    let late = |id: &'static str, kind: &'static str, amount: i64, m: i64| {
        let f = &f;
        async move {
            f.event_at(&f.fahrer, id, kind, amount, None, Some(t(m)))
                .await["late"]
                .clone()
        }
    };
    f.clock.set(t(70));
    // Auftrag 600: ungeklärt sinkt auf 50
    let l = late("job-1", "job_delivered", 600, 10).await;
    assert_eq!(n(&l["diff"]), 50);
    assert_eq!(l["booked"], json!([]));
    assert_eq!(
        l["unexplained"],
        json!({ "id": entry, "amount": 50, "closed": false })
    );
    // Maut 50: der Fahrer hatte noch weniger, ungeklärt steigt auf 100
    let l = late("toll-1", "tollgate", 50, 15).await;
    assert_eq!(n(&l["diff"]), 100);
    assert_eq!(l["unexplained"]["amount"], 100);
    let u = f.unexplained().await;
    assert_eq!((u.len(), n(&u[0]["amount"])), (1, 100));
    assert!(u[0]["closed_at"].is_null());
    // Auftrag 400: jetzt fehlen 300, Betriebskosten, ungeklärt ist erledigt
    let l = late("job-2", "job_delivered", 400, 20).await;
    assert_eq!(n(&l["diff"]), -300);
    assert_eq!(
        l["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -300 }])
    );
    assert_eq!(l["unexplained"]["closed"], true);
    // Strafe 100: die zahlt der Fahrer, also 100 € Betriebskosten zurück
    let l = late("fine-1", "fined", 100, 25).await;
    assert_eq!(n(&l["diff"]), -200);
    assert_eq!(
        l["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten_korrektur", "amount": 100 }])
    );
    assert_eq!(l["unexplained"], Value::Null);
    // Ein spätes Ereignis der Chefin ändert am Fenster des Fahrers nichts
    f.clock.set(t(75));
    let e = f
        .event_at(&f.chef, "toll-c", "tollgate", 5, None, Some(t(30)))
        .await;
    assert!(e.get("late").is_none());

    assert_eq!(f.costs().await, -200, "nur das Tanken");
    assert_eq!(f.balance().await, 600 + 400 - 50 - 5 - 300 + 100);
    assert_eq!(f.wallet(f.fahrer_id).await, -100);
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 0);
    assert_eq!(f.unexplained().await.len(), 1, "kein neuer Eintrag");

    // Der nächste Abgleich ohne Fahrt sieht keines der Ereignisse noch einmal
    let r = sync_written_at(&f, 2000, 120).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    // Geschrieben wird jetzt das Lohnkonto nach der Strafe plus Vorschuss
    assert_eq!(r["write_money"], 1900);
    assert_eq!(f.costs().await, -200);
}

#[tokio::test]
async fn spaete_strafe_gibt_betriebskosten_zurueck() {
    // Fall c: Strafe 100 € und Tanken 200 €. Ohne die Strafe sieht der Abgleich 300 €
    // Betriebskosten. Kommt sie spät, gibt es 100 € zurück, die Strafe trägt der Fahrer.
    let f = Firma::new().await;
    first_sync(&f).await;
    let r = sync_written_at(&f, 2000 - 100 - 200, 60).await;
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -300 }])
    );
    assert_eq!(f.balance().await, -300);

    f.clock.set(t(80));
    let e = f
        .event_at(&f.fahrer, "fine-1", "fined", 100, None, Some(t(45)))
        .await;
    assert_eq!(
        e["booked"],
        json!([{ "account": "fahrer", "kind": "strafe", "amount": -100, "game_amount": -100, "distance_km": null }])
    );
    assert_eq!(
        e["late"]["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten_korrektur", "amount": 100 }])
    );
    let korrektur = f
        .ledger()
        .await
        .into_iter()
        .find(|e| e["kind"] == "betriebskosten_korrektur")
        .unwrap();
    assert_eq!(
        korrektur["note"],
        "Spät gemeldet: Strafe, Abgleich vom 16.11.2026 11:01"
    );
    assert_eq!(korrektur["account"], "firma");
    assert_eq!(korrektur["user_id"], f.fahrer_id);
    assert_eq!(f.costs().await, -200);
    assert_eq!(f.balance().await, -200);
    assert_eq!(f.wallet(f.fahrer_id).await, -100);
    assert!(f.unexplained().await.is_empty());
}

#[tokio::test]
async fn ereignis_ohne_zeitpunkt_gilt_beim_empfang() {
    let f = Firma::new().await;
    first_sync(&f).await;

    // Älteres Plugin, Dashboard: ohne Zeitpunkt zählt der Empfang beim Server. Im Kassenbuch
    // bleibt occurred_at leer, created_at ist der Empfang.
    f.clock.set(t(10));
    let e = f.event(&f.fahrer, "toll-1", "tollgate", 30, None).await;
    assert_eq!(e["occurred_at"], Value::Null);
    assert!(e.get("late").is_none());
    // Unplausible Zeitpunkte werden nicht abgelehnt, sondern behandelt, als fehlten sie.
    // 1000 wäre sonst ein Zeitpunkt vor dem ersten Abgleich.
    f.clock.set(t(11));
    let e = f
        .event_at(&f.fahrer, "toll-2", "tollgate", 20, None, Some(1000))
        .await;
    assert_eq!(e["occurred_at"], Value::Null);
    f.clock.set(t(12));
    let e = f
        .event_at(
            &f.fahrer,
            "toll-3",
            "tollgate",
            10,
            None,
            Some(t(12) + 2 * 86_400),
        )
        .await;
    assert_eq!(e["occurred_at"], Value::Null);
    let l = f.ledger().await;
    let entry = |id: &str| l.iter().find(|e| e["event_id"] == id).unwrap().clone();
    for (id, m) in [("toll-1", 10), ("toll-2", 11), ("toll-3", 12)] {
        let e = entry(id);
        assert_eq!(e["occurred_at"], Value::Null, "{id}");
        assert_eq!(n(&e["created_at"]), t(m), "{id}");
    }

    // Abgleich ohne saved_at (Dashboard): Spielstand gilt als jetzt gelesen, alle drei zählen
    f.clock.set(t(20));
    let r = f.sync(&f.fahrer, 2000 - 60 - 200, None).await;
    assert_eq!(n(&r["saved_at"]), t(20));
    assert_eq!((n(&r["expected"]), n(&r["events"])), (1940, 3));
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -200 }])
    );
    // Bestätigung ohne written_at: es gilt der Empfang
    f.clock.set(t(21));
    let (_, w) = f.written(&f.fahrer, &r).await;
    assert_eq!(n(&w["written_at"]), t(21));

    // Danach ohne Zeitpunkt: landet im nächsten Fenster wie bisher
    f.clock.set(t(30));
    let e = f.event(&f.fahrer, "toll-4", "tollgate", 40, None).await;
    assert!(e.get("late").is_none());
    f.clock.set(t(40));
    let r = f.sync(&f.fahrer, 2000 - 40, None).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (1960, 1));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    assert_eq!(f.costs().await, -200);
}

#[tokio::test]
async fn ereignis_zwischen_gelesenem_und_geschriebenem_spielstand() {
    let f = Firma::new().await;
    first_sync(&f).await;
    // Gelesen wird der Spielstand von 11:00, geschrieben um 11:05
    f.clock.set(t(66));
    let r = f.sync_at(&f.fahrer, 2000, None, Some(t(60))).await;
    let (_, w) = f.written_at(&f.fahrer, &r, Some(t(65))).await;
    assert_eq!(n(&w["written_at"]), t(65));

    // Ein Ereignis mit Zeitpunkt dazwischen (da war das Spiel zu, also eine falsch gehende
    // Uhr): keine Korrektur, und es zählt auch im nächsten Fenster nicht
    f.clock.set(t(70));
    let e = f
        .event_at(&f.fahrer, "toll-1", "tollgate", 30, None, Some(t(62)))
        .await;
    assert!(e.get("late").is_none());
    assert_eq!(e["booked"][0]["amount"], -30);
    let r = sync_written_at(&f, 2000, 120).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);

    // Eine Bestätigung mit einem Zeitpunkt vor dem gelesenen Spielstand wird auf diesen
    // angehoben, sonst zählten Ereignisse dazwischen doppelt
    f.clock.set(t(181));
    let r = f.sync_at(&f.fahrer, 2000, None, Some(t(180))).await;
    let (_, w) = f.written_at(&f.fahrer, &r, Some(t(170))).await;
    assert_eq!(n(&w["written_at"]), t(180));

    // Unbestätigter Abgleich: das nächste Fenster beginnt direkt beim gelesenen Spielstand
    f.clock.set(t(241));
    let r = f.sync_at(&f.fahrer, 2000, None, Some(t(240))).await;
    assert_eq!(r["booked"], json!([]));
    f.clock.set(t(250));
    let e = f
        .event_at(&f.fahrer, "toll-2", "tollgate", 25, None, Some(t(242)))
        .await;
    assert!(e.get("late").is_none());
    let r = f.sync_at(&f.fahrer, 2000 - 25, None, Some(t(260))).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (1975, 1));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(f.costs().await, 0);
}

#[tokio::test]
async fn ereignis_vor_dem_ersten_abgleich() {
    let f = Firma::new().await;
    first_sync(&f).await;
    // Passiert um 9:30, vor dem Spielstand des ersten Abgleichs, gemeldet um 10:10: Der
    // Betrag steckt im Anfangsstand. Gebucht wird normal, korrigiert wird nichts.
    f.clock.set(t(10));
    let e = f
        .event_at(&f.fahrer, "job-1", "job_delivered", 500, None, Some(t(-30)))
        .await;
    assert!(e.get("late").is_none());
    assert_eq!(f.balance().await, 500);
    // Auch der nächste Abgleich zählt es nicht
    let r = sync_written_at(&f, 2000, 60).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    // Ebenso nach dem zweiten Abgleich: vor dem ersten bleibt vor dem ersten
    f.clock.set(t(70));
    let e = f
        .event_at(&f.fahrer, "toll-1", "tollgate", 10, None, Some(t(-20)))
        .await;
    assert!(e.get("late").is_none());
    assert_eq!(f.costs().await, 0);
    assert!(f.unexplained().await.is_empty());
}

#[tokio::test]
async fn geschlossener_ungeklaerter_eintrag_bleibt_geschlossen() {
    let f = Firma::new().await;
    first_sync(&f).await;
    // Ein Auftrag über 1000 € kommt spät, im Spielstand sind nur 800 € mehr. Die Chefin
    // schließt die 800 € vorher.
    let r = sync_written_at(&f, 2000 + 1000 - 200, 60).await;
    let list = format!("/api/companies/{}/unexplained", f.id);
    let (s, _) = f
        .post(
            &f.chef,
            &format!("{list}/{}/close", r["unexplained"]["id"]),
            json!({ "note": "Geschenk im Spiel" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    f.clock.set(t(70));
    let e = f
        .event_at(&f.fahrer, "job-1", "job_delivered", 1000, None, Some(t(20)))
        .await;
    // Die Betriebskosten werden nachgebucht, ungeklärt bleibt, wie die Chefin es hinterließ
    assert_eq!(
        e["late"]["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -200 }])
    );
    assert_eq!(e["late"]["unexplained"], Value::Null);
    let u = f.unexplained().await;
    assert_eq!(u.len(), 1);
    assert_eq!(
        (
            u[0]["amount"].as_i64(),
            u[0]["closed_by"].as_str(),
            u[0]["note"].as_str()
        ),
        (Some(800), Some("Chefin"), Some("Geschenk im Spiel"))
    );

    // Eine späte Maut von 900 € steckte auch schon im Spielstand. Dann fehlt nichts, sondern
    // es sind 700 € zu viel da. Die Chefin hat 800 € hingenommen, das deckt es.
    f.clock.set(t(75));
    let e = f
        .event_at(&f.fahrer, "toll-1", "tollgate", 900, None, Some(t(25)))
        .await;
    assert_eq!(n(&e["late"]["diff"]), 700);
    assert_eq!(
        e["late"]["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten_korrektur", "amount": 200 }])
    );
    assert_eq!(e["late"]["unexplained"], Value::Null);
    assert_eq!(f.unexplained().await.len(), 1);
    // Noch eine späte Maut: 1000 € zu viel, 200 € mehr als hingenommen, neuer offener Eintrag
    let e = f
        .event_at(&f.fahrer, "toll-2", "tollgate", 300, None, Some(t(26)))
        .await;
    assert_eq!(n(&e["late"]["diff"]), 1000);
    assert_eq!(e["late"]["booked"], json!([]));
    assert_eq!(e["late"]["unexplained"]["amount"], 200);
    assert_eq!(e["late"]["unexplained"]["closed"], false);
    let u = f.unexplained().await;
    assert_eq!(u.len(), 2);
    assert!(u[0]["closed_at"].is_null());
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 1);
    // Im Spiel ausgegeben wurde hier nichts außer dem Gemeldeten
    assert_eq!(f.costs().await, 0);
}

#[tokio::test]
async fn alte_abgleichpunkte_bekommen_ein_fenster() {
    // Abgleichpunkte aus der Zeit vor den Fenstern: ohne cutoff, diff und Aufteilung
    let path = std::env::temp_dir().join(format!("spedhub-fenster-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let f = Firma::with_db(spedhub_server::db::open(path.to_str().unwrap()).unwrap()).await;
    first_sync(&f).await;
    let r = sync_written_at(&f, 2000 + 1000 - 200, 60).await;
    assert_eq!(r["unexplained"]["amount"], 800);
    f.db.lock()
        .unwrap()
        .execute_batch(
            "UPDATE sync_points SET cutoff = NULL, written_at = NULL, diff = NULL,
                                    costs_booked = 0, unexplained_attributed = 0",
        )
        .unwrap();

    // Neustart des Servers: die Spalten werden aus dem befüllt, was damals galt
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    type Row = (i64, i64, Option<i64>, i64, i64);
    let rows: Vec<Row> = db
        .lock()
        .unwrap()
        .prepare(
            "SELECT cutoff, created_at, diff, costs_booked, unexplained_attributed
             FROM sync_points ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![(t(0), t(0), None, 0, 0), (t(61), t(61), Some(800), 0, 800)]
    );
    let f = Firma {
        app: spedhub_server::api::router_with(db.clone(), f.clock.clone()),
        db,
        ..f
    };

    // Ein spätes Ereignis korrigiert auch so einen Abgleich
    f.clock.set(t(70));
    let e = f
        .event_at(&f.fahrer, "job-1", "job_delivered", 1000, None, Some(t(30)))
        .await;
    assert_eq!(n(&e["late"]["diff"]), -200);
    assert_eq!(e["late"]["unexplained"]["closed"], true);
    assert_eq!(f.costs().await, -200);
    drop(f);
    for end in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{end}", path.display()));
    }
}

#[tokio::test]
async fn ereignis_ohne_zeitpunkt_in_derselben_sekunde_wie_der_abgleich() {
    let f = Firma::new().await;
    first_sync(&f).await;
    // Abgleich wie aus dem Dashboard, ohne saved_at und ohne written_at: cutoff und Grundlage
    // sind der Empfang um 10:10
    f.clock.set(t(10));
    let r = f.sync(&f.fahrer, 2000, None).await;
    assert_eq!(n(&r["saved_at"]), t(10));
    let (_, w) = f.written(&f.fahrer, &r).await;
    assert_eq!(n(&w["written_at"]), t(10));

    // In derselben Sekunde ein Auftrag und eine Maut ohne Zeitpunkt. Sie kamen nach dem
    // Abgleich an, also korrigieren sie ihn nicht, sondern zählen im nächsten.
    let e = f
        .event(&f.fahrer, "job-1", "job_delivered", 1000, Some(100.0))
        .await;
    assert!(e.get("late").is_none(), "{e}");
    let e = f.event(&f.fahrer, "toll-1", "tollgate", 30, None).await;
    assert!(e.get("late").is_none(), "{e}");
    assert_eq!(f.costs().await, 0);

    // Auch der nächste Abgleich in derselben Sekunde sieht beide
    let r = f.sync(&f.fahrer, 2000 + 1000 - 30, None).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2970, 2));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    f.written(&f.fahrer, &r).await;

    // Und danach zählen sie nicht noch einmal
    f.clock.set(t(20));
    let r = f.sync(&f.fahrer, 2000, None).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    assert_eq!(f.costs().await, 0, "im Spiel wurde nichts ausgegeben");
    assert_eq!(f.balance().await, 1000 - 30);
    assert!(f.unexplained().await.is_empty());
}

/// Die Uhr des Fahrer-PCs geht um `offset` Minuten anders als die des Servers. Launcher und
/// Plugin melden Zeitpunkte in PC-Zeit, dazu kommen Ereignisse ohne Zeitpunkt (älteres
/// Plugin, alte Zeilen im Postausgang, Dashboard) und ein Abgleich wie aus dem Dashboard.
async fn fremde_uhr(offset: i64) {
    let f = Firma::new().await;
    // Serverzeit zur PC-Minute m
    let server = |m: i64| f.clock.set(t(m - offset));
    let sync = |money: i64, m: i64| {
        let f = &f;
        async move {
            server(m + 1);
            let r = f.sync_at(&f.fahrer, money, None, Some(t(m))).await;
            let (s, w) = f.written_at(&f.fahrer, &r, Some(t(m + 1))).await;
            assert_eq!(s, StatusCode::OK, "{w}");
            r
        }
    };
    let r = sync(2000, 0).await;
    assert_eq!(r["first"], true);

    // Fenster 1: Tanken 200 €, eine Maut mit Zeitpunkt, alles pünktlich
    server(30);
    let e = f
        .event_at(&f.fahrer, "toll-1", "tollgate", 50, None, Some(t(30)))
        .await;
    assert!(e.get("late").is_none(), "{e}");
    let r = sync(2000 - 50 - 200, 60).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (1950, 1));
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -200 }])
    );

    // Gleich nach dem Abgleich, noch vor dem Spielstart, kommen ein Auftrag und eine Maut ohne
    // Zeitpunkt an, etwa aus dem Postausgang eines älteren Plugins. Sie gehören ins nächste
    // Fenster, egal wie die Uhren stehen.
    server(62);
    let e = f
        .event(&f.fahrer, "job-1", "job_delivered", 1000, Some(100.0))
        .await;
    assert!(e.get("late").is_none(), "{e}");
    let e = f.event(&f.fahrer, "toll-2", "tollgate", 30, None).await;
    assert!(e.get("late").is_none(), "{e}");
    assert_eq!(f.costs().await, -200);

    // Fenster 2: dazu ein Auftrag mit Zeitpunkt
    server(90);
    let e = f
        .event_at(&f.fahrer, "job-2", "job_delivered", 500, None, Some(t(90)))
        .await;
    assert!(e.get("late").is_none(), "{e}");
    let r = sync(2000 + 1000 - 30 + 500, 120).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (3470, 3));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);

    // Ein Abgleich wie aus dem Dashboard mit der Serverzeit, dazwischen ein Ereignis ohne
    // Zeitpunkt
    server(150);
    let e = f.event(&f.fahrer, "toll-3", "tollgate", 20, None).await;
    assert!(e.get("late").is_none(), "{e}");
    server(151);
    let r = f.sync(&f.fahrer, 2000 - 20, None).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (1980, 1));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    f.written(&f.fahrer, &r).await;

    // Danach wieder der Launcher, im Spiel ist nichts passiert
    let r = sync(2000, 200).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);

    assert_eq!(f.costs().await, -200, "nur das Tanken");
    assert_eq!(f.balance().await, -50 - 200 + 1000 - 30 + 500 - 20);
    assert!(f.unexplained().await.is_empty());
}

#[tokio::test]
async fn pc_uhr_geht_vor() {
    fremde_uhr(30).await;
}

#[tokio::test]
async fn pc_uhr_geht_nach() {
    fremde_uhr(-30).await;
}

#[tokio::test]
async fn fenster_ueberlappen_nicht() {
    // Die PC-Uhr geht 10 Minuten vor. Abgleiche kommen vom Launcher (PC-Zeit) und vom
    // Dashboard (Serverzeit).
    let f = Firma::new().await;
    first_sync(&f).await;
    // Auftrag 1000 € um 10:50 PC-Zeit, sofort gemeldet
    f.clock.set(t(40));
    f.event_at(&f.fahrer, "job-1", "job_delivered", 1000, None, Some(t(50)))
        .await;
    // Abgleich 2 über den Launcher: Spielstand von 11:00, geschrieben um 11:01 PC-Zeit
    f.clock.set(t(51));
    let r = f.sync_at(&f.fahrer, 3000, None, Some(t(60))).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (3000, 1));
    let (_, w) = f.written_at(&f.fahrer, &r, Some(t(61))).await;
    assert_eq!(n(&w["written_at"]), t(61));

    // Abgleich 3 aus dem Dashboard um 10:52 Serverzeit, also vor der Grundlage von Abgleich 2.
    // Sein cutoff wird auf diese Grundlage angehoben, sein Fenster ist leer.
    f.clock.set(t(52));
    let r = f.sync(&f.fahrer, 2000, None).await;
    assert_eq!(n(&r["saved_at"]), t(61));
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    let (_, w) = f.written(&f.fahrer, &r).await;
    assert_eq!(n(&w["written_at"]), t(61));

    // Abgleich 4 über den Launcher, im Spiel ist nichts passiert: Der Auftrag zählt nicht
    // noch einmal
    f.clock.set(t(60));
    let r = f.sync_at(&f.fahrer, 2000, None, Some(t(70))).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    f.written_at(&f.fahrer, &r, Some(t(71))).await;

    // Ein Launcher-Spielstand, der älter ist als der zuletzt geschriebene: auch hier keine
    // Überlappung, der Auftrag zählt nicht ein drittes Mal
    f.clock.set(t(61));
    let r = f.sync_at(&f.fahrer, 2000, None, Some(t(55))).await;
    assert_eq!(n(&r["saved_at"]), t(71));
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    // Ein spätes Ereignis mit Zeitpunkt in Fenster 2 korrigiert nur Abgleich 2. Die Maut
    // steckte schon in den 3000 €, also waren 40 € mehr da als erklärt.
    f.clock.set(t(62));
    let e = f
        .event_at(&f.fahrer, "toll-1", "tollgate", 40, None, Some(t(55)))
        .await;
    assert_eq!(e["late"]["sync_id"], json!(2));
    assert_eq!(e["late"]["booked"], json!([]));
    assert_eq!(e["late"]["unexplained"]["amount"], 40);

    assert_eq!(f.costs().await, 0, "im Spiel wurde nichts ausgegeben");
    assert_eq!(f.balance().await, 1000 - 40);
    assert_eq!(f.unexplained().await.len(), 1);
}

// ------------------------------------------------------------ Herzschlag, Online, Anmeldung

impl Firma {
    /// Lkw anlegen (als Chef), liefert die ID
    async fn truck(&self, plate: &str, km: i64, state: Value) -> i64 {
        let (s, t) = self
            .post(
                &self.chef,
                &format!("/api/companies/{}/trucks", self.id),
                json!({ "plate": plate, "model": "scania.s_2016", "odometer_km": km, "state": state }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{t}");
        n(&t["id"])
    }

    async fn checkout(&self, token: &str, truck: i64) {
        let (s, v) = self
            .post(token, &format!("/api/trucks/{truck}/checkout"), json!({}))
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }

    async fn checkin(&self, token: &str, truck: i64, body: Value) -> (StatusCode, Value) {
        self.post(token, &format!("/api/trucks/{truck}/checkin"), body)
            .await
    }

    async fn beat(&self, token: &str, body: Value) -> (StatusCode, Value) {
        self.post(
            token,
            &format!("/api/companies/{}/heartbeat", self.id),
            body,
        )
        .await
    }

    /// Ein Mitglied, so wie `token` die Firma sieht
    async fn member_as(&self, token: &str, user_id: i64) -> Value {
        let c = self.company(token).await;
        c["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["user_id"] == user_id)
            .unwrap()
            .clone()
    }

    /// Ein Lkw aus der Fuhrparkliste, so wie `token` sie sieht
    async fn truck_as(&self, token: &str, truck: i64) -> Value {
        let (s, l) = self
            .get(token, &format!("/api/companies/{}/trucks", self.id))
            .await;
        assert_eq!(s, StatusCode::OK, "{l}");
        l["trucks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == truck)
            .unwrap()
            .clone()
    }

    /// Zustand eines Lkw in der Datenbank: Kilometerstand, Zustand, letzter bekannter Zustand
    fn truck_row(&self, truck: i64) -> (i64, Value, Option<i64>, Option<i64>, Option<Value>) {
        let conn = self.db.lock().unwrap();
        conn.query_row(
            "SELECT odometer_km, state, last_seen_at, last_odometer_km, last_state FROM trucks WHERE id = ?1",
            [truck],
            |r| {
                let state: String = r.get(1)?;
                let last: Option<String> = r.get(4)?;
                Ok((
                    r.get(0)?,
                    serde_json::from_str(&state).unwrap(),
                    r.get(2)?,
                    r.get(3)?,
                    last.map(|l| serde_json::from_str(&l).unwrap()),
                ))
            },
        )
        .unwrap()
    }
}

/// Ein Zustand, wie ihn der Launcher aus dem Spielstand liest (Feldnamen von TruckState)
fn full_state(km: i64) -> Value {
    json!({
        "odometer": km, "odometer_float_part": 0.0, "integrity_odometer": 100,
        "integrity_odometer_float_part": 0.0, "fuel_relative": 0.8,
        "engine_wear": 0.01, "transmission_wear": 0.02, "cabin_wear": 0.03, "chassis_wear": 0.04,
        "engine_wear_unfixable": 0.0, "transmission_wear_unfixable": 0.0,
        "cabin_wear_unfixable": 0.0, "chassis_wear_unfixable": 0.0,
        "wheels_wear": [0.0, 0.0, 0.0], "wheels_wear_unfixable": [0.0, 0.0, 0.0]
    })
}

#[tokio::test]
async fn online_nach_herzschlag_und_offline_nach_31_sekunden() {
    let f = Firma::new().await;
    // Noch nie gemeldet
    let m = f.member(f.fahrer_id).await;
    assert_eq!(m["online"], false);
    assert!(m["last_seen"].is_null());

    let start = f.clock.now();
    let (s, r) = f
        .beat(
            &f.fahrer,
            json!({
                "plate": "EF XU 577", "odometer_km": 12345.6, "speed_kmh": 85.4,
                "near_city": "Berlin", "fuel_relative": 0.5,
                "job": { "cargo": "Holz", "source_city": "Berlin", "target_city": "Hamburg" },
                "paused": false
            }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r["leased_truck_id"].is_null(), "kein Firmen-Lkw");

    // Fahrer und Chef sehen den Fahrer online, mit Lkw, Stadt und Auftrag
    for token in [&f.chef, &f.fahrer] {
        let m = f.member_as(token, f.fahrer_id).await;
        assert_eq!(m["online"], true, "{m}");
        assert_eq!(n(&m["last_seen"]), start);
        assert_eq!(m["plate"], "EF XU 577");
        assert_eq!(m["near_city"], "Berlin");
        assert_eq!(m["speed_kmh"], 85.4);
        assert_eq!(
            m["job"],
            json!({ "cargo": "Holz", "source_city": "Berlin", "target_city": "Hamburg" })
        );
        assert!(m["truck_id"].is_null());
    }
    // Der Chef hat sich nie gemeldet
    assert_eq!(f.member(f.chef_id).await["online"], false);

    // 30 Sekunden später noch online, nach 31 Sekunden offline, nur noch mit last_seen
    f.clock.set(start + 30);
    assert_eq!(f.member(f.fahrer_id).await["online"], true);
    f.clock.set(start + 31);
    let m = f.member(f.fahrer_id).await;
    assert_eq!(m["online"], false);
    assert_eq!(n(&m["last_seen"]), start);
    assert!(m.get("plate").is_none() && m.get("job").is_none(), "{m}");

    // Ein neuer Herzschlag ohne Auftrag: wieder online, Auftrag leer, Rest überschrieben
    let (s, _) = f
        .beat(&f.fahrer, json!({ "plate": "EF XU 577", "job": null }))
        .await;
    assert_eq!(s, StatusCode::OK);
    let m = f.member(f.fahrer_id).await;
    assert_eq!(m["online"], true);
    assert_eq!(n(&m["last_seen"]), start + 31);
    assert!(m["job"].is_null() && m["near_city"].is_null() && m["speed_kmh"].is_null());
}

#[tokio::test]
async fn herzschlag_ohne_kennzeichen_mit_unsinn_oder_von_fremden() {
    let f = Firma::new().await;
    for body in [
        json!({}),
        json!({ "plate": "" }),
        json!({ "plate": "   " }),
        json!({ "plate": "<img src=/a.mat>" }),
        json!({ "plate": "X".repeat(65) }),
        json!({ "plate": "SPED 001", "odometer_km": -1 }),
        json!({ "plate": "SPED 001", "speed_kmh": -5 }),
        json!({ "plate": "SPED 001", "wear": { "engine": 1.5 } }),
        json!({ "plate": "SPED 001", "wear": { "wheels": -0.1 } }),
        json!({ "plate": "SPED 001", "fuel_relative": 2 }),
    ] {
        let (s, v) = f.beat(&f.fahrer, body.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body} {v}");
        assert!(v["error"].is_string());
    }
    let (_, v) = f.beat(&f.fahrer, json!({ "plate": "" })).await;
    assert_eq!(v["error"], "Das Kennzeichen fehlt");
    // Nichts davon zählt als Herzschlag
    assert!(f.member(f.fahrer_id).await["last_seen"].is_null());

    // Wer nicht zur Firma gehört: 403, ohne Anmeldung 401, unbekannte Firma 404
    let fremder = register(&f.app, "Fremder").await;
    let (s, _) = f.beat(&fremder, json!({ "plate": "SPED 001" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(
        &f.app,
        "POST",
        &format!("/api/companies/{}/heartbeat", f.id),
        None,
        Some(json!({ "plate": "SPED 001" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = f
        .post(
            &f.fahrer,
            "/api/companies/999/heartbeat",
            json!({ "plate": "SPED 001" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn letzter_bekannter_zustand_nur_fuer_den_eigenen_geliehenen_lkw() {
    let f = Firma::new().await;
    let t1 = f.truck("SPED 001", 1000, full_state(1000)).await;
    let t2 = f.truck("SPED 002", 2000, full_state(2000)).await;
    let hb = |plate: &str, km: f64| {
        json!({
            "plate": plate, "odometer_km": km, "fuel_relative": 0.4, "fuel_liters": 320.0,
            "wear": { "engine": 0.05, "transmission": 0.06, "cabin": 0.07, "chassis": 0.08, "wheels": 0.09 }
        })
    };

    // Nicht geliehen: kein letzter Zustand
    let (s, r) = f.beat(&f.fahrer, hb("SPED 001", 1100.0)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(r["leased_truck_id"].is_null());
    assert_eq!(f.truck_row(t1).2, None);

    // Geliehen: Kennzeichen mit Markup, Land und anderer Schreibweise passt auch
    f.checkout(&f.fahrer, t1).await;
    let leased_at = f.clock.now();
    f.clock.advance(10);
    let (s, r) = f
        .beat(
            &f.fahrer,
            hb(
                "<offset hshift=4 vshift=-5>sped<img src=/x.mat>  001|_germany",
                1234.5,
            ),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(n(&r["leased_truck_id"]), t1);
    let (km, state, seen, last_km, last) = f.truck_row(t1);
    assert_eq!(
        km, 1000,
        "der Kilometerstand des Lkw ändert sich erst bei der Rückgabe"
    );
    assert_eq!(state, full_state(1000));
    assert_eq!(seen, Some(leased_at + 10));
    assert_eq!(last_km, Some(1234));
    let last = last.unwrap();
    assert_eq!(last["odometer"], 1234);
    assert_eq!(last["odometer_float_part"], 0.5);
    assert_eq!(last["engine_wear"], 0.05);
    assert_eq!(last["chassis_wear"], 0.08);
    assert_eq!(last["fuel_relative"], 0.4);
    assert_eq!(last["wheels_wear"], json!([0.09, 0.09, 0.09]), "je Achse");
    // Nicht gemeldet, bleibt wie beim Ausleihen
    assert_eq!(last["integrity_odometer"], 100);
    assert_eq!(last["wheels_wear_unfixable"], json!([0.0, 0.0, 0.0]));

    // Ein späterer Herzschlag mit weniger Werten überschreibt nur, was er meldet
    f.clock.advance(10);
    f.beat(
        &f.fahrer,
        json!({ "plate": "SPED 001", "wear": { "engine": 0.06 } }),
    )
    .await;
    let (_, _, seen, last_km, last) = f.truck_row(t1);
    assert_eq!(seen, Some(leased_at + 20));
    assert_eq!(last_km, Some(1234));
    let last = last.unwrap();
    assert_eq!(last["engine_wear"], 0.06);
    assert_eq!(last["chassis_wear"], 0.08);
    assert_eq!(last["odometer"], 1234);

    // Ein Kilometerstand unter dem beim Ausleihen wird nicht übernommen
    f.beat(&f.fahrer, hb("SPED 001", 900.0)).await;
    let (_, _, _, last_km, last) = f.truck_row(t1);
    assert_eq!(last_km, Some(1234));
    assert_eq!(last.unwrap()["odometer"], 1234);

    // Den anderen Lkw hat der Chef geliehen: Herzschlag des Fahrers setzt dort nichts
    f.checkout(&f.chef, t2).await;
    let (_, r) = f.beat(&f.fahrer, hb("SPED 002", 2500.0)).await;
    assert!(r["leased_truck_id"].is_null());
    assert_eq!(f.truck_row(t2).2, None);

    // Über ein altes Kennzeichen: Rückgabe mit neuem Kennzeichen, wieder ausleihen
    let (s, _) = f
        .checkin(
            &f.fahrer,
            t1,
            json!({ "odometer_km": 1300, "plate": "EF XU 577" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        f.truck_row(t1).2,
        None,
        "Rückgabe leert den letzten Zustand"
    );
    f.checkout(&f.fahrer, t1).await;
    let (_, r) = f.beat(&f.fahrer, hb("SPED 001|germany", 1400.0)).await;
    assert_eq!(n(&r["leased_truck_id"]), t1);
    assert_eq!(f.truck_row(t1).3, Some(1400));
}

#[tokio::test]
async fn altes_kennzeichen_des_eigenen_lkw_schlaegt_aktuelles_eines_anderen() {
    let f = Firma::new().await;
    // Lkw B fährt als "SPED 001", das Spiel ersetzt das Kennzeichen, Rückgabe mit "AB 123"
    let b = f.truck("SPED 001", 1000, full_state(1000)).await;
    f.checkout(&f.fahrer, b).await;
    let (s, v) = f
        .checkin(
            &f.fahrer,
            b,
            json!({ "odometer_km": 1100, "plate": "AB 123" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Der Chef gibt das frei gewordene Kennzeichen einem neuen Lkw A
    let a = f.truck("SPED 001", 5000, full_state(5000)).await;

    // Der Launcher setzt B mit dem Kennzeichen aus dem Bauplan ein, also wieder "SPED 001"
    f.checkout(&f.fahrer, b).await;
    f.clock.advance(10);
    let seen = f.clock.now();
    let (s, r) = f
        .beat(
            &f.fahrer,
            json!({ "plate": "SPED 001", "odometer_km": 1200.0, "speed_kmh": 60 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        n(&r["leased_truck_id"]),
        b,
        "der eigene geliehene Lkw gewinnt"
    );
    let (_, _, last_seen, last_km, _) = f.truck_row(b);
    assert_eq!((last_seen, last_km), (Some(seen), Some(1200)));
    assert_eq!(f.truck_row(a).2, None);

    // Keine Fremd-Markierung, der Fuhrpark zeigt B als gefahren
    let m = f.member_as(&f.chef, f.fahrer_id).await;
    assert!(m.get("foreign_truck").is_none(), "{m}");
    assert_eq!(n(&m["truck_id"]), b);
    assert_eq!(f.truck_as(&f.chef, a).await["foreign_drivers"], json!([]));
    assert_eq!(f.truck_as(&f.chef, b).await["driving"], true);

    // Nach einem Absturz nimmt der Chef B mit dem letzten Zustand zurück
    let (s, r) = f.checkin(&f.chef, b, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["odometer_km"], 1200);

    // Ohne eigene Leihe gilt wieder das aktuelle Kennzeichen: A, als fremder Lkw markiert
    let (_, r) = f.beat(&f.fahrer, json!({ "plate": "SPED 001" })).await;
    assert!(r["leased_truck_id"].is_null());
    let m = f.member_as(&f.chef, f.fahrer_id).await;
    assert_eq!(m["foreign_truck"], json!({ "id": a, "plate": "SPED 001" }));
}

#[tokio::test]
async fn chef_nimmt_ohne_kilometer_zurueck_mit_dem_letzten_zustand() {
    let f = Firma::new().await;
    let t = f.truck("SPED 001", 1000, full_state(1000)).await;
    f.checkout(&f.fahrer, t).await;

    // Noch kein Herzschlag: ohne Kilometerstand geht es nicht, nichts ändert sich
    let (s, v) = f.checkin(&f.chef, t, json!({})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "Für diesen Lkw gibt es keinen letzten bekannten Zustand. Bitte den Kilometerstand angeben"
    );
    // Der Fahrer selbst braucht immer einen Kilometerstand
    let (s, v) = f.checkin(&f.fahrer, t, json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(
        v["error"],
        "Der Kilometerstand fehlt. Ohne ihn kann nur der Chef einen Lkw zurücknehmen"
    );
    assert_eq!(
        f.truck_as(&f.chef, t).await["leased_by"]["user_id"],
        f.fahrer_id
    );

    // Herzschlag beim Fahren, dann stürzt das Spiel ab
    f.clock.advance(60);
    let seen = f.clock.now();
    f.beat(
        &f.fahrer,
        json!({ "plate": "SPED 001", "odometer_km": 1523.25, "wear": { "engine": 0.2, "wheels": 0.3 }, "fuel_relative": 0.25 }),
    )
    .await;
    let l = f.truck_as(&f.chef, t).await;
    assert_eq!(l["last_known"], json!({ "at": seen, "odometer_km": 1523 }));

    // Der Fahrer kann es auch jetzt nicht ohne Kilometer
    let (s, _) = f.checkin(&f.fahrer, t, json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Der Chef nimmt ihn eine Stunde später ohne Kilometerstand zurück
    f.clock.advance(3600);
    let (s, r) = f.checkin(&f.chef, t, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["odometer_km"], 1523);
    assert_eq!(r["driven_km"], 523);
    assert_eq!(r["rev"], 2);
    assert_eq!(r["from_last_known"], json!({ "at": seen }));
    let (km, state, last_seen, last_km, last) = f.truck_row(t);
    assert_eq!(km, 1523);
    assert_eq!(state["odometer"], 1523);
    assert_eq!(state["engine_wear"], 0.2);
    assert_eq!(state["wheels_wear"], json!([0.3, 0.3, 0.3]));
    assert_eq!(state["fuel_relative"], 0.25);
    assert_eq!(
        state["cabin_wear"], 0.03,
        "nicht gemeldet, wie beim Ausleihen"
    );
    assert_eq!((last_seen, last_km, last), (None, None, None), "geleert");
    let l = f.truck_as(&f.chef, t).await;
    assert!(l["leased_by"].is_null() && l["last_known"].is_null());

    // Beim nächsten Fahrer gilt der alte Zustand nicht mehr
    f.checkout(&f.fahrer, t).await;
    let (s, _) = f.checkin(&f.chef, t, json!({})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    // Mit Kilometerstand geht es wie immer, ohne Hinweis auf den letzten Zustand
    let (s, r) = f.checkin(&f.chef, t, json!({ "odometer_km": 1600 })).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r["from_last_known"].is_null());
    assert_eq!(f.truck_row(t).1["engine_wear"], 0.2);
}

#[tokio::test]
async fn fremder_firmen_lkw_sieht_nur_der_chef() {
    let f = Firma::new().await;
    let t = f.truck("SPED 001", 1000, json!({})).await;

    // Der Fahrer fährt einen Firmen-Lkw, den er nicht geliehen hat
    f.beat(&f.fahrer, json!({ "plate": "sped 001", "speed_kmh": 70 }))
        .await;
    let m = f.member_as(&f.chef, f.fahrer_id).await;
    assert_eq!(m["foreign_truck"], json!({ "id": t, "plate": "SPED 001" }));
    assert!(m["truck_id"].is_null());
    let l = f.truck_as(&f.chef, t).await;
    assert_eq!(l["driving"], false, "nicht verliehen");
    assert_eq!(
        l["foreign_drivers"],
        json!([{ "user_id": f.fahrer_id, "name": "Fahrer", "speed_kmh": 70.0 }])
    );
    // Der Fahrer sieht die Markierung nicht
    let m = f.member_as(&f.fahrer, f.fahrer_id).await;
    assert!(m.get("foreign_truck").is_none(), "{m}");
    assert!(m["truck_id"].is_null());
    assert!(
        f.truck_as(&f.fahrer, t)
            .await
            .get("foreign_drivers")
            .is_none()
    );

    // Hat der Chef den Lkw geliehen, bleibt der Fahrer markiert
    f.checkout(&f.chef, t).await;
    f.beat(&f.fahrer, json!({ "plate": "SPED 001" })).await;
    let l = f.truck_as(&f.chef, t).await;
    assert_eq!(l["driving"], false, "der Ausleiher meldet sich nicht");
    assert_eq!(l["foreign_drivers"].as_array().unwrap().len(), 1);
    assert_eq!(f.truck_row(t).2, None, "kein letzter Zustand vom Falschen");

    // Offline: keine Markierung mehr
    f.clock.advance(31);
    let m = f.member_as(&f.chef, f.fahrer_id).await;
    assert!(m.get("foreign_truck").is_none());
    assert_eq!(f.truck_as(&f.chef, t).await["foreign_drivers"], json!([]));
}

#[tokio::test]
async fn fuhrpark_zeigt_ob_ein_verliehener_lkw_gefahren_wird() {
    let f = Firma::new().await;
    let t = f.truck("SPED 001", 1000, json!({})).await;
    let other = f.truck("SPED 002", 1000, json!({})).await;
    f.checkout(&f.fahrer, t).await;
    let l = f.truck_as(&f.fahrer, t).await;
    assert_eq!(l["driving"], false);
    assert!(l.get("speed_kmh").is_none());

    // Der Ausleiher meldet einen anderen Lkw: der verliehene fährt nicht
    f.beat(&f.fahrer, json!({ "plate": "EIGENER 1", "speed_kmh": 50 }))
        .await;
    assert_eq!(f.truck_as(&f.fahrer, t).await["driving"], false);

    // Er meldet genau diesen: alle sehen, dass er gefahren wird
    f.beat(
        &f.fahrer,
        json!({ "plate": "SPED 001", "speed_kmh": 85, "near_city": "Kassel" }),
    )
    .await;
    for token in [&f.chef, &f.fahrer] {
        let l = f.truck_as(token, t).await;
        assert_eq!(l["driving"], true, "{l}");
        assert_eq!(l["speed_kmh"], 85.0);
        assert_eq!(l["near_city"], "Kassel");
        assert_eq!(f.truck_as(token, other).await["driving"], false);
    }
    assert_eq!(n(&f.member(f.fahrer_id).await["truck_id"]), t);

    // Nach 31 Sekunden ohne Herzschlag nicht mehr
    f.clock.advance(31);
    let l = f.truck_as(&f.chef, t).await;
    assert_eq!(l["driving"], false);
    assert!(l.get("speed_kmh").is_none());
}

#[tokio::test]
async fn auftrag_mit_fracht_und_staedten_im_kassenbuch() {
    let f = Firma::new().await;
    let (s, r) = f
        .post(
            &f.fahrer,
            &format!("/api/companies/{}/events", f.id),
            json!({
                "event_id": "job-1", "type": "job_delivered", "amount": 1000, "distance_km": 300,
                "cargo": " Holz\nbretter ", "source_city": "Berlin",
                "target_city": "H".repeat(100)
            }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    // Bei anderen Ereignissen wird nichts davon gespeichert, ohne Angaben bleibt es leer
    let (s, _) = f
        .post(
            &f.fahrer,
            &format!("/api/companies/{}/events", f.id),
            json!({ "event_id": "maut-1", "type": "tollgate", "amount": 10, "cargo": "Holz" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    f.event(&f.fahrer, "job-2", "job_delivered", 500, Some(100.0))
        .await;
    let l = f.ledger().await;
    let by = |id: &str| l.iter().find(|e| e["event_id"] == id).unwrap().clone();
    let job = by("job-1");
    assert_eq!(job["cargo"], "Holz bretter");
    assert_eq!(job["source_city"], "Berlin");
    assert_eq!(job["target_city"], "H".repeat(64));
    for e in [by("maut-1"), by("job-2")] {
        assert!(
            e["cargo"].is_null() && e["source_city"].is_null() && e["target_city"].is_null(),
            "{e}"
        );
    }
}

#[tokio::test]
async fn token_laeuft_nach_30_tagen_ohne_benutzung_ab() {
    let f = Firma::new().await;
    let start = f.clock.now();
    let day = 86_400;

    // 29 Tage später gilt der Token noch, und die Benutzung verlängert ihn
    f.clock.set(start + 29 * day);
    let (s, _) = f.get(&f.fahrer, "/api/me").await;
    assert_eq!(s, StatusCode::OK);
    // 58 Tage nach dem Anlegen, aber nur 29 nach der letzten Benutzung: gilt
    f.clock.set(start + 58 * day);
    let (s, _) = f.get(&f.fahrer, "/api/me").await;
    assert_eq!(s, StatusCode::OK);
    // Genau 30 Tage ohne Benutzung: gilt noch
    f.clock.set(start + 88 * day);
    let (s, _) = f.get(&f.fahrer, "/api/me").await;
    assert_eq!(s, StatusCode::OK);

    // Der Chef hat seinen Token seit dem Anlegen nicht benutzt: abgelaufen
    let (s, v) = f.get(&f.chef, "/api/me").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        v["error"],
        "Die Anmeldung ist abgelaufen, bitte neu anmelden"
    );
    // Er ist gelöscht und gilt auch nicht, wenn die Uhr zurückgeht
    f.clock.set(start);
    let (s, v) = f.get(&f.chef, "/api/me").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(v["error"], "Nicht angemeldet");

    // 30 Tage und eine Sekunde ohne Benutzung: abgelaufen
    f.clock.set(start + 118 * day + 1);
    let (s, _) = f.get(&f.fahrer, "/api/me").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Neu anmelden geht und gibt einen frischen Token
    let (s, v) = call(
        &f.app,
        "POST",
        "/api/login",
        None,
        Some(json!({ "name": "Fahrer", "password": "geheim1234" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    f.clock.advance(30 * day);
    let (s, _) = f.get(v["token"].as_str().unwrap(), "/api/me").await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn passwort_aendern_macht_andere_tokens_ungueltig() {
    let f = Firma::new().await;
    let login = |password: &'static str| {
        let app = f.app.clone();
        async move {
            call(
                &app,
                "POST",
                "/api/login",
                None,
                Some(json!({ "name": "Fahrer", "password": password })),
            )
            .await
        }
    };
    let (_, second) = login("geheim1234").await;
    let second = second["token"].as_str().unwrap().to_string();
    let change = |token: String, old: &'static str, new: &'static str| {
        let app = f.app.clone();
        async move {
            call(
                &app,
                "POST",
                "/api/me/password",
                Some(&token),
                Some(json!({ "old_password": old, "new_password": new })),
            )
            .await
        }
    };

    // Falsches altes Passwort: 403, nicht 401, damit niemand abgemeldet wird
    let (s, v) = change(f.fahrer.clone(), "falsch123", "neuesPasswort1").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(v["error"], "Das alte Passwort stimmt nicht");
    // Zu kurz
    let (s, v) = change(f.fahrer.clone(), "geheim1234", "kurz").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], "Das neue Passwort braucht mindestens 8 Zeichen");
    // Ohne Anmeldung
    let (s, _) = call(
        &f.app,
        "POST",
        "/api/me/password",
        None,
        Some(json!({ "old_password": "geheim1234", "new_password": "neuesPasswort1" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Bis hier gelten beide Tokens
    assert_eq!(f.get(&second, "/api/me").await.0, StatusCode::OK);

    // Ändern: der benutzte Token bleibt, der zweite ist ungültig, der Chef ist unberührt
    let (s, v) = change(f.fahrer.clone(), "geheim1234", "neuesPasswort1").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["other_tokens_removed"], 1);
    assert_eq!(f.get(&f.fahrer, "/api/me").await.0, StatusCode::OK);
    assert_eq!(f.get(&second, "/api/me").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(f.get(&f.chef, "/api/me").await.0, StatusCode::OK);

    // Anmelden nur noch mit dem neuen Passwort
    assert_eq!(login("geheim1234").await.0, StatusCode::UNAUTHORIZED);
    let (s, _) = login("neuesPasswort1").await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn alte_tokens_gelten_ab_der_umstellung_30_tage() {
    use sha2::{Digest, Sha256};
    // Eine Datenbank von vor dem Ablauf der Tokens, mit einem Token von vor einem Jahr
    let path = std::env::temp_dir().join(format!("spedhub-token-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let token = "alter-token";
    let hash: String = Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                pw_hash TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE tokens (token_hash TEXT PRIMARY KEY, user_id INTEGER NOT NULL,
                created_at INTEGER NOT NULL);
             INSERT INTO users VALUES (1, 'Alt', 'x', 1000);",
        )
        .unwrap();
        c.execute("INSERT INTO tokens VALUES (?1, 1, 1000)", [&hash])
            .unwrap();
    }
    let before = spedhub_server::clock::system_now();
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    let after = spedhub_server::clock::system_now();
    let last: i64 = db
        .lock()
        .unwrap()
        .query_row("SELECT last_used FROM tokens", [], |r| r.get(0))
        .unwrap();
    assert!(
        (before..=after).contains(&last),
        "Umstellung zählt als Benutzung"
    );

    let clock = Clock::fixed(last + 29 * 86_400);
    let app = spedhub_server::api::router_with(db.clone(), clock.clone());
    let (s, me) = call(&app, "GET", "/api/me", Some(token), None).await;
    assert_eq!(s, StatusCode::OK, "{me}");
    assert_eq!(me["name"], "Alt");
    clock.advance(30 * 86_400 + 1);
    let (s, _) = call(&app, "GET", "/api/me", Some(token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    drop(app);
    drop(db);
    let _ = std::fs::remove_file(&path);
}

// ------------------------------------------------------------ Schaden und Werkstatt

/// Ein voller Zustand mit diesen reparierbaren Verschleißwerten, dauerhafter Verschleiß 0
fn worn(
    km: i64,
    engine: f64,
    transmission: f64,
    cabin: f64,
    chassis: f64,
    wheels: &[f64],
) -> Value {
    let mut s = full_state(km);
    s["engine_wear"] = json!(engine);
    s["transmission_wear"] = json!(transmission);
    s["cabin_wear"] = json!(cabin);
    s["chassis_wear"] = json!(chassis);
    s["wheels_wear"] = json!(wheels);
    s["wheels_wear_unfixable"] = json!(vec![0.0; wheels.len()]);
    s
}

fn pct(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("keine Zahl: {v}"))
}

impl Firma {
    async fn repair(&self, token: &str, truck: i64) -> (StatusCode, Value) {
        self.post(token, &format!("/api/trucks/{truck}/repair"), json!({}))
            .await
    }

    /// Ausleihen und mit diesem Zustand zurückgeben, beides als Fahrer. Liefert die Antwort
    /// der Rückgabe.
    async fn drive(&self, truck: i64, km: i64, state: Value) -> Value {
        self.checkout(&self.fahrer, truck).await;
        let (s, r) = self
            .checkin(
                &self.fahrer,
                truck,
                json!({ "odometer_km": km, "state": state }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{r}");
        r
    }

    /// Alle Buchungen einer Art, neueste zuerst
    async fn entries(&self, kind: &str) -> Vec<Value> {
        self.ledger()
            .await
            .into_iter()
            .filter(|e| e["kind"] == kind)
            .collect()
    }

    fn rev(&self, truck: i64) -> i64 {
        let conn = self.db.lock().unwrap();
        conn.query_row("SELECT rev FROM trucks WHERE id = ?1", [truck], |r| {
            r.get(0)
        })
        .unwrap()
    }
}

#[tokio::test]
async fn schaden_wird_aus_dem_zustand_berechnet() {
    let f = Firma::new().await;
    let mut state = worn(1000, 0.1, 0.2, 0.3, 0.4, &[0.1, 0.3]);
    state["engine_wear_unfixable"] = json!(0.1);
    state["wheels_wear_unfixable"] = json!([0.2, 0.0]);
    let a = f.truck("SPED 001", 1000, state).await;
    let b = f.truck("SPED 002", 1000, json!({})).await;
    let c = f
        .truck("SPED 003", 1000, json!({ "engine_wear": 0.5 }))
        .await;

    // Motor 30, Getriebe 20, Kabine 10, Fahrgestell 25, Räder 15 (Mittel der Achsen):
    // 3 + 4 + 3 + 10 + 3 = 23. Dauerhaft getrennt: 30 * 0,1 + 15 * 0,1 = 4,5
    for token in [&f.chef, &f.fahrer] {
        let t = f.truck_as(token, a).await;
        assert_eq!(pct(&t["damage"]), 23.0, "{t}");
        assert_eq!(pct(&t["damage_unfixable"]), 4.5, "{t}");
        assert_eq!(t["in_workshop"], false);
        assert_eq!(t["repair_cost"], 3450, "23 Prozentpunkte mal 150 Euro");
    }
    // Ohne Zustand kein Schaden, fehlende Felder zählen als 0
    let t = f.truck_as(&f.fahrer, b).await;
    assert_eq!(
        (
            pct(&t["damage"]),
            pct(&t["damage_unfixable"]),
            &t["repair_cost"]
        ),
        (0.0, 0.0, &json!(0))
    );
    let t = f.truck_as(&f.fahrer, c).await;
    assert_eq!(pct(&t["damage"]), 15.0);

    // Der Satz der Werkstatt ändert die Kosten
    let (s, v) = f.patch(&f.chef, json!({ "repair_rate": 200 })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(f.truck_as(&f.fahrer, a).await["repair_cost"], 4600);
}

#[tokio::test]
async fn ueber_der_schadensgrenze_in_die_werkstatt() {
    let f = Firma::new().await;
    let t = f
        .truck(
            "SPED 001",
            1000,
            worn(1000, 0.0, 0.0, 0.0, 0.0, &[0.0, 0.0]),
        )
        .await;

    // Genau auf der Grenze von 25 Prozent: noch frei
    let r = f
        .drive(t, 1100, worn(1100, 0.0, 0.0, 0.0, 1.0, &[0.0, 0.0]))
        .await;
    assert_eq!(pct(&r["damage"]), 25.0, "{r}");
    assert_eq!(r["in_workshop"], false);
    assert_eq!(f.truck_as(&f.chef, t).await["in_workshop"], false);

    // Ein Zehntel darüber: Werkstatt
    let r = f
        .drive(t, 1200, worn(1200, 0.0, 0.0, 0.01, 1.0, &[0.0, 0.0]))
        .await;
    assert_eq!(pct(&r["damage"]), 25.1, "{r}");
    assert_eq!(r["in_workshop"], true);
    let l = f.truck_as(&f.fahrer, t).await;
    assert_eq!(l["in_workshop"], true);
    assert!(l["leased_by"].is_null());

    // Ausleihen geht für niemanden, auch nicht für den Chef
    for token in [&f.fahrer, &f.chef] {
        let (s, v) = f
            .post(token, &format!("/api/trucks/{t}/checkout"), json!({}))
            .await;
        assert_eq!(s, StatusCode::CONFLICT, "{v}");
        assert_eq!(
            v["error"],
            "Der Lkw steht in der Werkstatt. Ausleihen geht erst, wenn der Chef ihn reparieren lässt"
        );
    }
    assert!(f.truck_as(&f.chef, t).await["leased_by"].is_null());

    // Eine höhere Grenze gilt ab der nächsten Rückgabe, der Lkw bleibt in der Werkstatt
    f.patch(&f.chef, json!({ "damage_limit": 50 })).await;
    assert_eq!(f.truck_as(&f.chef, t).await["in_workshop"], true);
}

#[tokio::test]
async fn chef_rueckgabe_mit_letztem_zustand_schickt_in_die_werkstatt() {
    let f = Firma::new().await;
    let t = f.truck("SPED 001", 1000, full_state(1000)).await;
    f.checkout(&f.fahrer, t).await;
    f.clock.advance(60);
    f.beat(
        &f.fahrer,
        json!({ "plate": "SPED 001", "odometer_km": 1300.0, "wear": { "engine": 0.9, "chassis": 0.5 } }),
    )
    .await;
    // Das Spiel stürzt ab, die Chefin nimmt den Lkw mit dem letzten Zustand zurück:
    // Motor 27, Getriebe 0,4, Kabine 0,3, Fahrgestell 12,5, Räder 0
    let (s, r) = f.checkin(&f.chef, t, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r["from_last_known"].is_object());
    assert_eq!(pct(&r["damage"]), 40.2);
    assert_eq!(r["in_workshop"], true);
    assert_eq!(f.truck_as(&f.chef, t).await["in_workshop"], true);
    let (s, _) = f
        .post(&f.fahrer, &format!("/api/trucks/{t}/checkout"), json!({}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn werkstatt_mit_genug_geld() {
    let f = Firma::new().await;
    f.event(&f.fahrer, "job-1", "job_delivered", 10_000, None)
        .await;
    let t = f
        .truck(
            "SPED 001",
            1000,
            worn(1000, 0.0, 0.0, 0.0, 0.0, &[0.0, 0.0]),
        )
        .await;
    // 6 + 2 + 1 + 15 + 4,5 = 28,5 Prozent, dazu dauerhaft am Motor 1,5
    let mut state = worn(1400, 0.2, 0.1, 0.1, 0.6, &[0.2, 0.4]);
    state["engine_wear_unfixable"] = json!(0.05);
    state["fuel_relative"] = json!(0.3);
    let r = f.drive(t, 1400, state).await;
    assert_eq!(r["in_workshop"], true, "{r}");
    let l = f.truck_as(&f.chef, t).await;
    assert_eq!(
        (
            pct(&l["damage"]),
            pct(&l["damage_unfixable"]),
            &l["repair_cost"]
        ),
        (28.5, 1.5, &json!(4275))
    );
    assert_eq!(f.rev(t), 2);

    // Reparieren lassen darf nur der Chef
    let (s, v) = f.repair(&f.fahrer, t).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], "Das darf nur der Chef");
    let (s, v) = f.repair(&f.chef, 999).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");

    let (s, r) = f.repair(&f.chef, t).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["cost"], 4275);
    assert_eq!(r["balance"], 5725);
    assert_eq!(pct(&r["damage_before"]), 28.5);
    assert_eq!(pct(&r["damage"]), 0.0);
    assert_eq!(r["rev"], 3);
    assert_eq!(r["in_workshop"], false);
    assert_eq!(f.balance().await, 5725);

    // Eine Buchung aus der Firmenkasse
    let w = f.entries("werkstatt").await;
    assert_eq!(w.len(), 1);
    assert_eq!(
        (&w[0]["account"], &w[0]["amount"], &w[0]["note"]),
        (
            &json!("firma"),
            &json!(-4275),
            &json!("Werkstatt SPED 001, 28,5 %")
        )
    );

    // Reparierbarer Verschleiß auf 0, dauerhafter und alles andere bleibt
    let (km, state, ..) = f.truck_row(t);
    assert_eq!(km, 1400);
    for k in [
        "engine_wear",
        "transmission_wear",
        "cabin_wear",
        "chassis_wear",
    ] {
        assert_eq!(state[k], 0.0, "{k}");
    }
    assert_eq!(state["wheels_wear"], json!([0.0, 0.0]));
    assert_eq!(state["engine_wear_unfixable"], 0.05);
    assert_eq!(state["fuel_relative"], 0.3);
    assert_eq!(state["odometer"], 1400);
    let l = f.truck_as(&f.chef, t).await;
    assert_eq!((pct(&l["damage"]), pct(&l["damage_unfixable"])), (0.0, 1.5));
    assert_eq!(
        (&l["in_workshop"], &l["repair_cost"], &l["rev"]),
        (&json!(false), &json!(0), &json!(3))
    );

    // Nichts mehr zu reparieren
    let (s, v) = f.repair(&f.chef, t).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "Am Lkw ist nichts zu reparieren");
    assert_eq!(f.entries("werkstatt").await.len(), 1);

    // Wieder ausleihbar, der nächste Fahrer bekommt den reparierten Zustand
    let (s, out) = f
        .post(&f.fahrer, &format!("/api/trucks/{t}/checkout"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["truck"]["rev"], 3);
    assert_eq!(out["truck"]["state"]["engine_wear"], 0.0);
    assert_eq!(out["truck"]["state"]["wheels_wear"], json!([0.0, 0.0]));
    assert_eq!(out["truck"]["state"]["engine_wear_unfixable"], 0.05);

    // Ein verliehener Lkw kommt nicht in die Werkstatt, auch mit Schaden nicht
    let (s, _) = f
        .checkin(
            &f.fahrer,
            t,
            json!({ "odometer_km": 1500, "state": worn(1500, 0.1, 0.0, 0.0, 0.0, &[0.0, 0.0]) }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    f.checkout(&f.fahrer, t).await;
    let (s, v) = f.repair(&f.chef, t).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "Der Lkw ist gerade an Fahrer verliehen. Reparieren geht erst nach der Rückgabe"
    );
    assert_eq!(f.entries("werkstatt").await.len(), 1);
    assert_eq!(f.balance().await, 5725);
}

#[tokio::test]
async fn werkstatt_mit_zu_wenig_geld() {
    let f = Firma::new().await;
    f.event(&f.fahrer, "job-1", "job_delivered", 1000, None)
        .await;
    let t = f
        .truck(
            "SPED 001",
            1000,
            worn(1000, 0.0, 0.0, 0.0, 0.0, &[0.0, 0.0]),
        )
        .await;
    f.drive(t, 1400, worn(1400, 0.2, 0.1, 0.1, 0.6, &[0.2, 0.4]))
        .await;
    let before = f.truck_row(t).1;

    // 4275 Euro, in der Kasse sind 1000: nichts ändert sich
    let (s, v) = f.repair(&f.chef, t).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "In der Kasse ist nicht genug Geld für die Werkstatt: nötig 4275 €, da sind 1000 €"
    );
    assert_eq!(f.balance().await, 1000);
    assert!(f.entries("werkstatt").await.is_empty());
    assert_eq!(f.truck_row(t).1, before);
    assert_eq!(f.rev(t), 2);
    let l = f.truck_as(&f.chef, t).await;
    assert_eq!((&l["in_workshop"], pct(&l["damage"])), (&json!(true), 28.5));

    // Mit einem kleineren Satz reicht es: 28,5 mal 20 Euro
    f.patch(&f.chef, json!({ "repair_rate": 20 })).await;
    let (s, r) = f.repair(&f.chef, t).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!((&r["cost"], &r["balance"]), (&json!(570), &json!(430)));

    // Satz 0: Die Werkstatt ist kostenlos, gebucht wird nichts
    let u = f
        .truck("SPED 002", 1000, worn(1000, 0.5, 0.0, 0.0, 0.0, &[0.0]))
        .await;
    f.patch(&f.chef, json!({ "repair_rate": 0 })).await;
    let (s, r) = f.repair(&f.chef, u).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!((&r["cost"], &r["balance"]), (&json!(0), &json!(430)));
    assert_eq!(f.entries("werkstatt").await.len(), 1);
    assert_eq!(f.truck_row(u).1["engine_wear"], 0.0);
}

#[tokio::test]
async fn kostenlose_werkstatt_auch_bei_negativer_kasse() {
    // Betriebskosten aus dem Geldabgleich werden ohne Deckung gebucht: Kasse -500
    let f = Firma::new().await;
    first_sync(&f).await;
    let r = sync_written_at(&f, 1500, 60).await;
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -500 }])
    );
    assert_eq!(f.balance().await, -500);

    let t = f
        .truck(
            "SPED 001",
            1000,
            worn(1000, 0.0, 0.0, 0.0, 0.0, &[0.0, 0.0]),
        )
        .await;
    f.drive(t, 1400, worn(1400, 0.2, 0.1, 0.1, 0.6, &[0.2, 0.4]))
        .await;
    assert_eq!(f.truck_as(&f.chef, t).await["in_workshop"], true);

    // Mit Kosten über 0 reicht eine negative Kasse nicht
    let (s, v) = f.repair(&f.chef, t).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "In der Kasse ist nicht genug Geld für die Werkstatt: nötig 4275 €, da sind -500 €"
    );

    // Satz 0: kostenlos, also geht es auch mit negativer Kasse, gebucht wird nichts
    f.patch(&f.chef, json!({ "repair_rate": 0 })).await;
    let (s, r) = f.repair(&f.chef, t).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!((&r["cost"], &r["balance"]), (&json!(0), &json!(-500)));
    assert_eq!(r["in_workshop"], false);
    assert!(f.entries("werkstatt").await.is_empty());
    assert_eq!(f.truck_as(&f.chef, t).await["in_workshop"], false);

    // Kosten, die auf 0 € gerundet werden: 0,1 % mal 4 € sind 0,40 €
    let u = f
        .truck("SPED 002", 1000, worn(1000, 0.0, 0.0, 0.01, 0.0, &[0.0]))
        .await;
    f.patch(&f.chef, json!({ "repair_rate": 4 })).await;
    let l = f.truck_as(&f.chef, u).await;
    assert_eq!((pct(&l["damage"]), &l["repair_cost"]), (0.1, &json!(0)));
    let (s, r) = f.repair(&f.chef, u).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!((&r["cost"], &r["balance"]), (&json!(0), &json!(-500)));
    assert!(f.entries("werkstatt").await.is_empty());
    assert_eq!(f.truck_row(u).1["cabin_wear"], 0.0);
    assert_eq!(f.balance().await, -500);
}

#[tokio::test]
async fn haftung_nur_fuer_neuen_schaden() {
    let f = Firma::new().await;
    f.patch(&f.chef, json!({ "liability_percent": 50 })).await;
    // Vorschaden 10 Prozent am Fahrgestell
    let t = f
        .truck(
            "SPED 001",
            1000,
            worn(1000, 0.0, 0.0, 0.0, 0.4, &[0.0, 0.0]),
        )
        .await;
    f.checkout(&f.fahrer, t).await;
    let damage_out: f64 = {
        let conn = f.db.lock().unwrap();
        conn.query_row(
            "SELECT damage_out FROM leases WHERE truck_id = ?1",
            [t],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(damage_out, 10.0);

    // Zurück mit 16 Prozent: 6 neu, davon zahlt der Fahrer die Hälfte von 6 mal 150 Euro
    let (s, r) = f
        .checkin(
            &f.fahrer,
            t,
            json!({ "odometer_km": 1200, "state": worn(1200, 0.2, 0.0, 0.0, 0.4, &[0.0, 0.0]) }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (pct(&r["damage"]), pct(&r["damage_new"]), &r["liability"]),
        (16.0, 6.0, &json!(450))
    );
    assert_eq!(r["in_workshop"], false);
    let h = f.entries("haftung").await;
    assert_eq!(h.len(), 2, "{h:?}");
    for e in &h {
        assert_eq!(e["user_id"], f.fahrer_id);
        assert_eq!(e["note"], "Haftung SPED 001, 6 Prozentpunkte neuer Schaden");
    }
    let by = |account: &str| h.iter().find(|e| e["account"] == account).unwrap()["amount"].clone();
    assert_eq!((by("fahrer"), by("firma")), (json!(-450), json!(450)));
    // Das Lohnkonto darf ins Minus
    assert_eq!(f.wallet(f.fahrer_id).await, -450);
    assert_eq!(f.balance().await, 450);

    // Nächste Leihe ohne neuen Schaden: keine Haftung, keine Buchung
    let r = f
        .drive(t, 1300, worn(1300, 0.2, 0.0, 0.0, 0.4, &[0.0, 0.0]))
        .await;
    assert_eq!((pct(&r["damage_new"]), &r["liability"]), (0.0, &json!(0)));
    assert_eq!(f.entries("haftung").await.len(), 2);
    assert_eq!(f.wallet(f.fahrer_id).await, -450);
}

#[tokio::test]
async fn haftung_0_bucht_nichts() {
    let f = Firma::new().await;
    assert_eq!(f.company(&f.chef).await["liability_percent"], 0);
    let t = f
        .truck("SPED 001", 1000, worn(1000, 0.0, 0.0, 0.0, 0.0, &[0.0]))
        .await;
    let r = f
        .drive(t, 1200, worn(1200, 0.5, 0.0, 0.0, 0.0, &[0.0]))
        .await;
    assert_eq!((pct(&r["damage_new"]), &r["liability"]), (15.0, &json!(0)));
    assert!(f.entries("haftung").await.is_empty());
    assert_eq!(f.wallet(f.fahrer_id).await, 0);
}

#[tokio::test]
async fn haftung_bei_chef_rueckgabe_zahlt_der_ausleiher() {
    let f = Firma::new().await;
    f.patch(&f.chef, json!({ "liability_percent": 100 })).await;
    let t = f
        .truck(
            "SPED 001",
            1000,
            worn(1000, 0.0, 0.0, 0.0, 0.0, &[0.0, 0.0]),
        )
        .await;

    // Die Chefin nimmt mit Zustand zurück: 3 Prozentpunkte mal 150 Euro
    f.checkout(&f.fahrer, t).await;
    let (s, r) = f
        .checkin(
            &f.chef,
            t,
            json!({ "odometer_km": 1100, "state": worn(1100, 0.1, 0.0, 0.0, 0.0, &[0.0, 0.0]) }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["liability"], 450);
    assert_eq!(r["liability_user_id"], f.fahrer_id);
    assert_eq!(f.wallet(f.fahrer_id).await, -450);
    assert_eq!(f.wallet(f.chef_id).await, 0);

    // Und mit dem letzten bekannten Zustand: Motor jetzt 0,3, also 6 Prozentpunkte neu
    f.checkout(&f.fahrer, t).await;
    f.clock.advance(60);
    f.beat(
        &f.fahrer,
        json!({ "plate": "SPED 001", "odometer_km": 1250.0, "wear": { "engine": 0.3 } }),
    )
    .await;
    let (s, r) = f.checkin(&f.chef, t, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!((pct(&r["damage_new"]), &r["liability"]), (6.0, &json!(900)));
    assert_eq!(f.wallet(f.fahrer_id).await, -1350);
    assert_eq!(f.wallet(f.chef_id).await, 0);
    let h = f.entries("haftung").await;
    assert_eq!(h.len(), 4);
    assert!(h.iter().all(|e| e["user_id"] == f.fahrer_id));
}

#[tokio::test]
async fn leihe_von_vor_der_werkstatt_rechnet_vom_gespeicherten_zustand() {
    let f = Firma::new().await;
    f.patch(&f.chef, json!({ "liability_percent": 100 })).await;
    let t = f
        .truck("SPED 001", 1000, worn(1000, 0.0, 0.0, 0.0, 0.4, &[0.0]))
        .await;
    f.checkout(&f.fahrer, t).await;
    // Wie eine Leihe aus einer alten Datenbank: kein Schaden beim Ausleihen gespeichert
    f.db.lock()
        .unwrap()
        .execute("UPDATE leases SET damage_out = NULL", [])
        .unwrap();
    let (s, r) = f
        .checkin(
            &f.fahrer,
            t,
            json!({ "odometer_km": 1100, "state": worn(1100, 0.1, 0.0, 0.0, 0.4, &[0.0]) }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(pct(&r["damage_out"]), 10.0);
    assert_eq!((pct(&r["damage_new"]), &r["liability"]), (3.0, &json!(450)));
}

#[tokio::test]
async fn einstellungen_fuer_werkstatt_und_haftung() {
    let f = Firma::new().await;
    for token in [&f.chef, &f.fahrer] {
        let c = f.company(token).await;
        assert_eq!(
            (
                &c["damage_limit"],
                &c["repair_rate"],
                &c["liability_percent"]
            ),
            (&json!(25), &json!(150), &json!(0))
        );
    }
    for (body, error) in [
        (
            json!({ "damage_limit": 101 }),
            "Die Schadensgrenze muss zwischen 0 und 100 Prozent liegen",
        ),
        (
            json!({ "damage_limit": -1 }),
            "Die Schadensgrenze muss zwischen 0 und 100 Prozent liegen",
        ),
        (
            json!({ "repair_rate": -1 }),
            "Der Satz der Werkstatt muss zwischen 0 und 1000000 Euro je Prozentpunkt liegen",
        ),
        (
            json!({ "liability_percent": 101 }),
            "Die Haftung muss zwischen 0 und 100 Prozent liegen",
        ),
        (
            json!({ "liability_percent": -5, "damage_limit": 30 }),
            "Die Haftung muss zwischen 0 und 100 Prozent liegen",
        ),
    ] {
        let (s, v) = f.patch(&f.chef, body.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}: {v}");
        assert_eq!(v["error"], error);
    }
    let (s, _) = f.patch(&f.fahrer, json!({ "damage_limit": 50 })).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let c = f.company(&f.chef).await;
    assert_eq!(
        (
            &c["damage_limit"],
            &c["repair_rate"],
            &c["liability_percent"]
        ),
        (&json!(25), &json!(150), &json!(0)),
        "nichts geändert"
    );

    let (s, v) = f
        .patch(
            &f.chef,
            json!({ "damage_limit": 40, "repair_rate": 90, "liability_percent": 25 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (
            &v["damage_limit"],
            &v["repair_rate"],
            &v["liability_percent"]
        ),
        (&json!(40), &json!(90), &json!(25))
    );
    let c = f.company(&f.fahrer).await;
    assert_eq!(
        (
            &c["damage_limit"],
            &c["repair_rate"],
            &c["liability_percent"],
            &c["advance"]
        ),
        (&json!(40), &json!(90), &json!(25), &json!(2000))
    );
    // Die Ränder gehen
    let (s, v) = f
        .patch(
            &f.chef,
            json!({ "damage_limit": 0, "repair_rate": 0, "liability_percent": 100 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = f.patch(&f.chef, json!({ "damage_limit": 100 })).await;
    assert_eq!(s, StatusCode::OK);
}

// ------------------------------------------------------------ Anschaffungen

impl Firma {
    async fn budget(&self, token: &str, body: Value) -> (StatusCode, Value) {
        self.post(token, &format!("/api/companies/{}/budgets", self.id), body)
            .await
    }

    async fn close_budget(&self, token: &str, budget: &Value) -> (StatusCode, Value) {
        self.post(
            token,
            &format!("/api/companies/{}/budgets/{}/close", self.id, budget["id"]),
            json!({}),
        )
        .await
    }

    /// Alle Budgets, neueste zuerst, wie die Chefin sie sieht
    async fn budgets(&self) -> Value {
        let (s, b) = self
            .get(&self.chef, &format!("/api/companies/{}/budgets", self.id))
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b
    }

    /// Abgleich der Chefin um `m` + 1 Minuten mit einem Spielstand von Minute `m`, bestätigt
    async fn chef_sync(&self, money: i64, m: i64) -> Value {
        self.clock.set(t(m + 1));
        let r = self.sync_at(&self.chef, money, None, Some(t(m))).await;
        let (s, w) = self.written_at(&self.chef, &r, Some(t(m + 1))).await;
        assert_eq!(s, StatusCode::OK, "{w}");
        r
    }

    async fn link(&self, token: &str, entry: i64, truck: i64) -> (StatusCode, Value) {
        self.post(
            token,
            &format!("/api/companies/{}/ledger/{entry}/link-truck", self.id),
            json!({ "truck_id": truck }),
        )
        .await
    }

    async fn candidates(&self, token: &str) -> (StatusCode, Value) {
        self.get(
            token,
            &format!("/api/companies/{}/purchase-candidates", self.id),
        )
        .await
    }

    /// Eine Buchung aus dem Kassenbuch
    async fn entry(&self, id: i64) -> Value {
        self.ledger()
            .await
            .into_iter()
            .find(|e| e["id"] == id)
            .unwrap_or_else(|| panic!("keine Buchung {id}"))
    }
}

/// Die Kasse bekommt 100000 € aus einem Auftrag des Fahrers. Seine Ereignisse zählen nicht für
/// den Abgleich der Chefin.
async fn full_cash(f: &Firma) {
    f.event(&f.fahrer, "job-kasse", "job_delivered", 100_000, None)
        .await;
    assert_eq!(f.balance().await, 100_000);
}

#[tokio::test]
async fn budget_freigeben() {
    let f = Firma::new().await;
    full_cash(&f).await;
    let before = f.ledger().await.len();

    // Nur die Chefin, der Fahrer bekommt 403
    let (s, v) = f.budget(&f.fahrer, json!({ "amount": 1000 })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, _) = f
        .get(&f.fahrer, &format!("/api/companies/{}/budgets", f.id))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Unsinnige Beträge
    for amount in [0, -5] {
        let (s, v) = f.budget(&f.chef, json!({ "amount": amount })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    // Größer als die Kasse: 409 mit Meldung, nichts angelegt
    let (s, v) = f.budget(&f.chef, json!({ "amount": 100_001 })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "In der Kasse ist nicht genug Geld für das Budget: nötig 100001 €, da sind 100000 €"
    );
    assert_eq!(f.company(&f.chef).await["budget"], Value::Null);

    // Freigeben: nichts gebucht, Kasse unverändert
    let (s, v) = f
        .budget(
            &f.chef,
            json!({ "amount": 60_000, "note": "  Neuer Scania\n " }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let b = &v["budget"];
    assert_eq!(
        (n(&b["amount"]), n(&b["rest"]), n(&b["consumed"])),
        (60_000, 60_000, 0)
    );
    assert_eq!(b["note"], "Neuer Scania");
    assert_eq!(b["closed_at"], Value::Null);
    assert_eq!(f.ledger().await.len(), before, "nichts gebucht");
    assert_eq!(f.balance().await, 100_000);

    // Die Chefin sieht das offene Budget, der Fahrer nicht
    let c = f.company(&f.chef).await;
    assert_eq!(c["budget"]["id"], b["id"]);
    assert_eq!(n(&c["budget"]["rest"]), 60_000);
    assert!(f.company(&f.fahrer).await.get("budget").is_none());
    let list = f.budgets().await;
    assert_eq!(list["open"]["id"], b["id"]);
    assert_eq!(list["budgets"].as_array().unwrap().len(), 1);

    // Höchstens ein offenes Budget je Firma
    let (s, v) = f.budget(&f.chef, json!({ "amount": 1 })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("schon ein offenes Anschaffungsbudget")
    );
}

#[tokio::test]
async fn abgleich_mit_budget_und_kauf() {
    let f = Firma::new().await;
    full_cash(&f).await;

    // Erster Abgleich der Chefin vor dem Budget: Lohnkonto 0 plus Vorschuss 2000
    let r = f.chef_sync(2000, -1).await;
    assert_eq!((r["first"].as_bool(), n(&r["budget"])), (Some(true), 0));
    assert_eq!(n(&r["write_money"]), 2000);

    f.clock.set(t(5));
    let (s, v) = f.budget(&f.chef, json!({ "amount": 60_000 })).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Nächster Abgleich schreibt Lohnkonto plus Vorschuss plus Budget, bucht nichts
    let r = f.chef_sync(2000, 10).await;
    assert_eq!(n(&r["expected"]), 2000);
    assert_eq!(r["booked"], json!([]));
    assert_eq!(
        (n(&r["wallet"]), n(&r["advance"]), n(&r["budget"])),
        (0, 2000, 60_000)
    );
    assert_eq!(n(&r["write_money"]), 62_000);
    assert_eq!(f.balance().await, 100_000);

    // Der Fahrer bekommt das Budget der Chefin nicht
    let r = f.sync(&f.fahrer, 2000, None).await;
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (0, 2000));

    // Im Spiel kauft die Chefin einen Lkw für 45000 €: genau einmal Betriebskosten
    let r = f.chef_sync(62_000 - 45_000, 30).await;
    assert_eq!(n(&r["expected"]), 62_000);
    assert_eq!(
        r["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -45_000 }])
    );
    assert_eq!(n(&r["budget"]), 15_000, "Rest sinkt um den Kauf");
    assert_eq!(n(&r["write_money"]), 2000 + 15_000);
    assert_eq!(f.balance().await, 55_000);
    assert_eq!(f.costs().await, -45_000);
    let b = &f.budgets().await["open"];
    assert_eq!((n(&b["consumed"]), n(&b["rest"])), (45_000, 15_000));
    assert_eq!(n(&f.company(&f.chef).await["budget"]["rest"]), 15_000);

    // Ohne weitere Ausgaben bleibt alles, wie es ist: kein zweiter Abzug
    let r = f.chef_sync(17_000, 40).await;
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (15_000, 17_000));
    assert_eq!(f.balance().await, 55_000);

    // Tanken der Chefin nach der Freigabe verbraucht das Budget ebenfalls
    let r = f.chef_sync(17_000 - 300, 50).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -300);
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (14_700, 16_700));
    assert_eq!(f.costs().await, -45_300);
    assert_eq!(f.balance().await, 100_000 - 45_300);

    // Mehr als der Rest ausgegeben: Die Firma zahlt alles, der Rest ist 0, nie negativ
    let r = f.chef_sync(16_700 - 20_000, 60).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -20_000);
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (0, 2000));
    let b = &f.budgets().await["open"];
    assert_eq!((n(&b["consumed"]), n(&b["rest"])), (60_000, 0));
}

#[tokio::test]
async fn spaet_gemeldeter_auftrag_verbraucht_budget() {
    let f = Firma::new().await;
    full_cash(&f).await;
    f.chef_sync(2000, -1).await;
    f.clock.set(t(5));
    f.budget(&f.chef, json!({ "amount": 50_000 })).await;
    f.chef_sync(2000, 10).await;

    // Im Spiel: Kauf für 40000 €, dazu ein Auftrag über 1000 €, den das Plugin erst nach dem
    // Abgleich meldet. Der Abgleich bucht deshalb erst 39000 €.
    let r = f.chef_sync(52_000 - 40_000 + 1000, 30).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -39_000);
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (11_000, 13_000));
    f.clock.set(t(35));
    let e = f
        .event_at(
            &f.chef,
            "job-spaet",
            "job_delivered",
            1000,
            None,
            Some(t(20)),
        )
        .await;
    assert_eq!(n(&e["late"]["booked"][0]["amount"]), -1000);
    // Der Auftrag gehört der Firma, im Spiel bleiben nur Lohnkonto, Vorschuss und Rest
    let r = f.chef_sync(13_000, 40).await;
    assert_eq!(r["booked"], json!([]));
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (10_000, 12_000));
    assert_eq!(f.costs().await, -40_000);
    assert_eq!(f.balance().await, 100_000 + 1000 - 40_000);
}

/// Eine spät gemeldete Maut aus einem Abgleich nach der Freigabe: Der Abgleich hat sie als
/// Betriebskosten gebucht und vom Budget abgezogen, die Gutschrift gibt sie zurück.
#[tokio::test]
async fn spaet_gemeldete_maut_gibt_budget_zurueck() {
    let f = Firma::new().await;
    full_cash(&f).await;
    f.chef_sync(2000, -1).await;
    f.clock.set(t(5));
    f.budget(&f.chef, json!({ "amount": 50_000 })).await;
    let r = f.chef_sync(2000, 10).await;
    assert_eq!(n(&r["write_money"]), 52_000);

    // Im Spiel: Kauf für 40000 € und 1000 € Maut um t(20), die das Plugin erst später meldet
    let r = f.chef_sync(52_000 - 41_000, 30).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -41_000);
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (9000, 11_000));
    f.clock.set(t(35));
    let e = f
        .event_at(&f.chef, "maut-spaet", "tollgate", 1000, None, Some(t(20)))
        .await;
    assert_eq!(e["late"]["booked"][0]["kind"], "betriebskosten_korrektur");
    assert_eq!(n(&e["late"]["booked"][0]["amount"]), 1000);

    // Die Maut zahlt die Firma, verbraucht ist nur der Kauf
    let r = f.chef_sync(11_000, 40).await;
    assert_eq!(r["booked"], json!([]));
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (10_000, 12_000));
    assert_eq!(f.costs().await, -40_000);
}

/// Eine Gutschrift für einen Abgleich von vor der Freigabe macht das Budget nicht größer als
/// freigegeben: Die Betriebskosten dieses Abgleichs waren nie vom Budget abgezogen.
#[tokio::test]
async fn korrektur_von_vor_der_freigabe_vergroessert_budget_nicht() {
    let f = Firma::new().await;
    full_cash(&f).await;
    f.chef_sync(2000, -1).await;
    // 1000 € Maut um t(5), das Plugin meldet sie erst nach dem Abgleich
    let r = f.chef_sync(1000, 10).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -1000);

    f.clock.set(t(12));
    f.budget(&f.chef, json!({ "amount": 50_000 })).await;
    f.clock.set(t(13));
    let e = f
        .event_at(&f.chef, "maut-1", "tollgate", 1000, None, Some(t(5)))
        .await;
    assert_eq!(e["late"]["booked"][0]["kind"], "betriebskosten_korrektur");
    let budget = f.company(&f.chef).await["budget"].clone();
    assert_eq!(n(&budget["rest"]), 50_000, "{budget}");

    let r = f.chef_sync(2000, 20).await;
    assert_eq!(n(&r["write_money"]), 52_000);
    // Kauf für genau das Budget: danach ist nichts mehr übrig
    let r = f.chef_sync(2000, 30).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -50_000);
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (0, 2000));
    // Nach der Freigabe sind genau 50000 € als Betriebskosten gebucht
    assert_eq!(f.costs().await, -1000 + 1000 - 50_000);
}

#[tokio::test]
async fn budget_schliessen() {
    let f = Firma::new().await;
    full_cash(&f).await;
    f.chef_sync(2000, -1).await;
    f.clock.set(t(5));
    let (_, v) = f
        .budget(&f.chef, json!({ "amount": 50_000, "note": "Volvo" }))
        .await;
    let budget = v["budget"].clone();
    let r = f.chef_sync(2000, 10).await;
    assert_eq!(n(&r["write_money"]), 52_000);
    let r = f.chef_sync(52_000 - 30_000, 30).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -30_000);
    assert_eq!(n(&r["write_money"]), 22_000);

    // Nur die Chefin schließt
    let (s, _) = f.close_budget(&f.fahrer, &budget).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = f.close_budget(&f.chef, &json!({ "id": 999 })).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Schließen bucht nichts und sagt, wie viel entfällt
    let ledger = f.ledger().await.len();
    let balance = f.balance().await;
    f.clock.set(t(40));
    let (s, v) = f.close_budget(&f.chef, &budget).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (n(&v["budget"]["rest"]), n(&v["budget"]["consumed"])),
        (20_000, 30_000)
    );
    assert_eq!(n(&v["budget"]["closed_at"]), t(40));
    assert_eq!(v["budget"]["closed_by"], "Chefin");
    assert_eq!(f.ledger().await.len(), ledger, "nichts gebucht");
    assert_eq!(f.balance().await, balance);
    assert_eq!(f.company(&f.chef).await["budget"], Value::Null);
    let (s, _) = f.close_budget(&f.chef, &budget).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Nächster Abgleich: Im Spiel steht noch der Stand mit Budget. Das ist genau der erwartete
    // Stand, also keine Betriebskosten. Geschrieben wird ohne Budget.
    let r = f.chef_sync(22_000, 50).await;
    assert_eq!(n(&r["expected"]), 22_000);
    assert_eq!(r["booked"], json!([]));
    assert_eq!(r["unexplained"], Value::Null);
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (0, 2000));
    // Und danach rechnet der Abgleich vom neuen Stand
    let r = f.chef_sync(2000, 60).await;
    assert_eq!(n(&r["expected"]), 2000);
    assert_eq!(r["booked"], json!([]));
    assert_eq!(f.costs().await, -30_000, "nur der Kauf");
    assert_eq!(f.balance().await, 70_000);

    // Was nach dem Schließen gebucht wird, zählt nicht mehr zum alten Budget
    f.chef_sync(2000 - 500, 70).await;
    let list = f.budgets().await;
    assert_eq!(list["open"], Value::Null);
    let old = &list["budgets"][0];
    assert_eq!((n(&old["consumed"]), n(&old["rest"])), (30_000, 20_000));
    assert_eq!(old["note"], "Volvo");

    // Danach geht ein neues Budget, die Kasse muss es decken
    let (s, v) = f.budget(&f.chef, json!({ "amount": 69_500 })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        n(&v["budget"]["rest"]),
        69_500,
        "die 500 € von vorher zählen nicht"
    );
    assert_eq!(f.budgets().await["budgets"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn lkw_kauf_verknuepfen() {
    let f = Firma::new().await;
    full_cash(&f).await;
    f.chef_sync(2000, -1).await;
    f.clock.set(t(5));
    f.budget(&f.chef, json!({ "amount": 50_000 })).await;
    f.chef_sync(2000, 10).await;
    f.chef_sync(52_000 - 45_000, 30).await;
    // Auch der Fahrer hat Betriebskosten. Die stehen nicht unter den Kandidaten der Chefin.
    f.sync(&f.fahrer, 2000, None).await;
    f.sync(&f.fahrer, 1800, None).await;

    let (s, _) = f.candidates(&f.fahrer).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, c) = f.candidates(&f.chef).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    let c = c["entries"].as_array().unwrap().clone();
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(n(&c[0]["amount"]), -45_000);
    assert_eq!(n(&c[0]["created_at"]), t(31));
    assert!(c[0]["note"].as_str().unwrap().starts_with("Abgleich"));
    let kauf = n(&c[0]["id"]);

    // Die Chefin übernimmt den Lkw in den Fuhrpark und verknüpft den Kauf
    let truck = f.truck("SPED 004", 1000, json!({})).await;
    let (s, _) = f.link(&f.fahrer, kauf, truck).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = f.link(&f.chef, kauf, truck).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["truck_plate"], "SPED 004");
    let e = f.entry(kauf).await;
    assert_eq!(
        (n(&e["truck_id"]), &e["truck_plate"]),
        (truck, &json!("SPED 004"))
    );
    // Die Buchung selbst bleibt, wie sie war
    assert_eq!(
        (&e["kind"], n(&e["amount"])),
        (&json!("betriebskosten"), -45_000)
    );
    assert_eq!(f.balance().await, 100_000 - 45_000 - 200);
    let (_, c) = f.candidates(&f.chef).await;
    assert!(c["entries"].as_array().unwrap().is_empty());

    // Doppelt: 409
    let (s, v) = f.link(&f.chef, kauf, truck).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "Die Buchung ist schon als Kauf von SPED 004 verknüpft"
    );
    // Falsche Art: 400
    let auftrag = n(&f.entries("auftrag").await[0]["id"]);
    let (s, v) = f.link(&f.chef, auftrag, truck).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // Unbekannte Buchung: 404
    let (s, _) = f.link(&f.chef, 999_999, truck).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Lkw einer anderen Firma: 404
    let fremd = register(&f.app, "Fremder").await;
    let (_, andere) = f
        .post(&fremd, "/api/companies", json!({ "name": "Südfracht" }))
        .await;
    let (s, fremder_lkw) = f
        .post(
            &fremd,
            &format!("/api/companies/{}/trucks", andere["id"]),
            json!({ "plate": "SUED 1", "model": "volvo.fh16_2012" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let fahrer_kosten = n(&f.entries("betriebskosten").await[0]["id"]);
    let (s, v) = f.link(&f.chef, fahrer_kosten, n(&fremder_lkw["id"])).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"], "Diesen Lkw gibt es in der Firma nicht");
    // Eine Buchung dieser Firma aus der anderen heraus: 404
    let (s, _) = f
        .post(
            &fremd,
            &format!("/api/companies/{}/ledger/{kauf}/link-truck", andere["id"]),
            json!({ "truck_id": n(&fremder_lkw["id"]) }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Das Kassenbuch zeigt immer das aktuelle Kennzeichen
    f.checkout(&f.fahrer, truck).await;
    let (s, _) = f
        .checkin(
            &f.fahrer,
            truck,
            json!({ "odometer_km": 1100, "plate": "EF AB 123" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(f.entry(kauf).await["truck_plate"], "EF AB 123");

    // Verknüpfung lösen: wieder Betriebskosten und wieder Kandidat
    let unlink = format!("/api/companies/{}/ledger/{kauf}/link-truck", f.id);
    let (s, _) = call(&f.app, "DELETE", &unlink, Some(&f.fahrer), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = call(&f.app, "DELETE", &unlink, Some(&f.chef), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let e = f.entry(kauf).await;
    assert_eq!(
        (&e["truck_id"], &e["truck_plate"]),
        (&Value::Null, &Value::Null)
    );
    let (s, _) = call(&f.app, "DELETE", &unlink, Some(&f.chef), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, c) = f.candidates(&f.chef).await;
    assert_eq!(n(&c["entries"][0]["id"]), kauf);
}

impl Firma {
    async fn retire(&self, token: &str, truck: i64) -> (StatusCode, Value) {
        self.post(token, &format!("/api/trucks/{truck}/retire"), json!({}))
            .await
    }

    async fn sell(&self, token: &str, truck: i64, price: i64) -> (StatusCode, Value) {
        self.post(
            token,
            &format!("/api/trucks/{truck}/sell"),
            json!({ "price": price }),
        )
        .await
    }
}

#[tokio::test]
async fn lkw_ausmustern() {
    let f = Firma::new().await;
    full_cash(&f).await;
    let alt = f
        .truck(
            "SPED 001",
            1000,
            worn(1000, 0.0, 0.0, 0.0, 0.2, &[0.0, 0.0]),
        )
        .await;
    let anderer = f.truck("SPED 002", 1000, json!({})).await;
    // Vorgeschichte: Werkstatt und ein verknüpfter Kauf
    let (s, _) = f.repair(&f.chef, alt).await;
    assert_eq!(s, StatusCode::OK);
    f.chef_sync(2000, -1).await;
    f.chef_sync(1000, 10).await;
    let kauf = n(&f.entries("betriebskosten").await[0]["id"]);
    f.link(&f.chef, kauf, alt).await;

    // Verliehen: erst zurückgeben
    f.checkout(&f.fahrer, alt).await;
    let (s, v) = f.retire(&f.chef, alt).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "Der Lkw ist gerade an Fahrer verliehen. Erst zurückgeben, dann ausmustern"
    );
    f.checkin(&f.fahrer, alt, json!({ "odometer_km": 1200 }))
        .await;
    let (s, _) = f.retire(&f.fahrer, alt).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let ledger = f.ledger().await;
    f.clock.set(t(30));
    let (s, v) = f.retire(&f.chef, alt).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["retired"]["reason"], "ausgemustert");
    assert_eq!(f.ledger().await, ledger, "nichts gebucht, alles bleibt");

    // Bleibt in der Liste, hinter den aktiven, mit Grund und Datum
    let (_, l) = f
        .get(&f.fahrer, &format!("/api/companies/{}/trucks", f.id))
        .await;
    let ids: Vec<i64> = l["trucks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| n(&t["id"]))
        .collect();
    assert_eq!(ids, vec![anderer, alt]);
    let t1 = f.truck_as(&f.fahrer, alt).await;
    assert_eq!(
        t1["retired"],
        json!({ "reason": "ausgemustert", "at": t(30), "sale_price": null })
    );
    assert_eq!(n(&t1["odometer_km"]), 1200);
    assert_eq!(f.truck_as(&f.fahrer, anderer).await["retired"], Value::Null);

    // Nicht mehr ausleihbar, nicht reparierbar, nicht noch einmal ausmustern oder verkaufen
    for token in [&f.fahrer, &f.chef] {
        let (s, v) = f
            .post(token, &format!("/api/trucks/{alt}/checkout"), json!({}))
            .await;
        assert_eq!(s, StatusCode::CONFLICT, "{v}");
        assert_eq!(
            v["error"],
            "Der Lkw ist ausgemustert und lässt sich nicht mehr ausleihen"
        );
    }
    let (s, _) = f.repair(&f.chef, alt).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = f.retire(&f.chef, alt).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = f.sell(&f.chef, alt, 100).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Ein Herzschlag mit seinem Kennzeichen markiert niemanden als fremd gefahren
    f.clock.set(t(31));
    let (s, hb) = f.beat(&f.fahrer, json!({ "plate": "SPED 001" })).await;
    assert_eq!(s, StatusCode::OK, "{hb}");
    assert_eq!(hb["leased_truck_id"], Value::Null);
    let m = f.member_as(&f.chef, f.fahrer_id).await;
    assert!(m.get("foreign_truck").is_none(), "{m}");
    assert_eq!(f.truck_as(&f.chef, alt).await["foreign_drivers"], json!([]));

    // Ein neuer Lkw darf das Kennzeichen bekommen. Der alte behält es bei den früheren.
    let neu = f.truck("SPED 001", 5000, json!({})).await;
    let t1 = f.truck_as(&f.chef, alt).await;
    let freed = format!("SPED 001 (Lkw {alt})");
    assert_eq!(t1["plate"], freed);
    assert_eq!(t1["old_plates"], json!(["SPED 001"]));
    assert_eq!(f.truck_as(&f.chef, neu).await["plate"], "SPED 001");
    // Der Herzschlag gehört jetzt zum neuen
    f.checkout(&f.fahrer, neu).await;
    let (_, hb) = f.beat(&f.fahrer, json!({ "plate": "SPED 001" })).await;
    assert_eq!(n(&hb["leased_truck_id"]), neu);
    // Die Geschichte bleibt: Werkstatt mit altem Kennzeichen, Kauf zeigt das aktuelle
    assert_eq!(
        f.entries("werkstatt").await[0]["note"],
        "Werkstatt SPED 001, 5 %"
    );
    assert_eq!(f.entry(kauf).await["truck_plate"], freed);
    // Ein aktiver Lkw gibt sein Kennzeichen weiter nicht her
    let (s, v) = f
        .post(
            &f.chef,
            &format!("/api/companies/{}/trucks", f.id),
            json!({ "plate": "SPED 002", "model": "scania.s_2016" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
}

#[tokio::test]
async fn rueckgabe_mit_kennzeichen_eines_ausgemusterten_lkw() {
    let f = Firma::new().await;
    let alt = f.truck("SPED 001", 1000, json!({})).await;
    let neu = f.truck("SPED 002", 1000, json!({})).await;
    f.retire(&f.chef, alt).await;
    // Das Spiel gibt dem geliehenen Lkw das Kennzeichen des ausgemusterten
    f.checkout(&f.fahrer, neu).await;
    let (s, v) = f
        .checkin(
            &f.fahrer,
            neu,
            json!({ "odometer_km": 1100, "plate": "SPED 001" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(f.truck_as(&f.chef, neu).await["plate"], "SPED 001");
    assert_eq!(
        f.truck_as(&f.chef, alt).await["plate"],
        format!("SPED 001 (Lkw {alt})")
    );
}

#[tokio::test]
async fn lkw_verkaufen() {
    let f = Firma::new().await;
    full_cash(&f).await;
    let a = f.truck("SPED 002", 1000, json!({})).await;
    let b = f.truck("SPED 003", 1000, json!({})).await;

    // Verliehen: 409
    f.checkout(&f.fahrer, a).await;
    let (s, v) = f.sell(&f.chef, a, 30_000).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "Der Lkw ist gerade an Fahrer verliehen. Erst zurückgeben, dann verkaufen"
    );
    f.checkin(&f.fahrer, a, json!({ "odometer_km": 1500 }))
        .await;
    let (s, _) = f.sell(&f.fahrer, a, 30_000).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = f.sell(&f.chef, a, -1).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(f.balance().await, 100_000);

    // Verkauf: Erlös in die Kasse
    f.clock.set(t(20));
    let (s, v) = f.sell(&f.chef, a, 30_000).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(n(&v["balance"]), 130_000);
    assert_eq!(f.balance().await, 130_000);
    let e = &f.entries("lkw_verkauf").await[0];
    assert_eq!(
        (&e["account"], n(&e["amount"]), &e["note"], &e["user"]),
        (
            &json!("firma"),
            30_000,
            &json!("Verkauf SPED 002"),
            &json!("Chefin")
        )
    );
    let t1 = f.truck_as(&f.fahrer, a).await;
    assert_eq!(
        t1["retired"],
        json!({ "reason": "verkauft", "at": t(20), "sale_price": 30_000 })
    );
    let (s, v) = f
        .post(&f.fahrer, &format!("/api/trucks/{a}/checkout"), json!({}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(
        v["error"],
        "Der Lkw ist verkauft und lässt sich nicht mehr ausleihen"
    );
    let (s, v) = f.sell(&f.chef, a, 1).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(
        v["error"],
        "Der Lkw ist verkauft und steht nicht mehr im Fuhrpark"
    );

    // Für 0 Euro: verkauft, aber nichts gebucht
    let (s, _) = f.sell(&f.chef, b, 0).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(f.entries("lkw_verkauf").await.len(), 1);
    assert_eq!(f.balance().await, 130_000);
    assert_eq!(
        f.truck_as(&f.chef, b).await["retired"]["sale_price"],
        json!(0)
    );
}
