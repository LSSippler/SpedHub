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
    assert!(html.contains("Kennzeichen ändern") && html.contains("plate-suggestion"));
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

    /// Herzschlag wie vom Plugin, mit Marke und Modell der Lkw aus [`Firma::truck`]
    /// (`scania` und `s_2016`, zusammen `scania.s_2016`), wenn `body` kein `truck_model` hat.
    /// Genau so, wie er ist, geht er mit [`Firma::beat_raw`].
    async fn beat(&self, token: &str, mut body: Value) -> (StatusCode, Value) {
        if let Some(o) = body.as_object_mut()
            && !o.contains_key("truck_model")
        {
            o.insert("truck_brand".into(), json!("scania"));
            o.insert("truck_model".into(), json!("s_2016"));
        }
        self.beat_raw(token, body).await
    }

    async fn beat_raw(&self, token: &str, body: Value) -> (StatusCode, Value) {
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

    // B fährt noch mit "SPED 001", etwa aus einem Spielstand von vor der Rückgabe
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

// ------------------------------------------------------------ Nur das Firmenprofil zählt

impl Firma {
    async fn put(&self, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
        call(&self.app, "PUT", path, Some(token), Some(body)).await
    }

    /// Das Firmenprofil melden, wie es der Launcher tut
    async fn report_profile(&self, token: &str, name: Value) -> (StatusCode, Value) {
        self.put(
            token,
            &format!("/api/companies/{}/profile", self.id),
            json!({ "profile_name": name }),
        )
        .await
    }

    /// Ereignis wie vom Plugin, mit Profil. Liefert Status und Antwort, ohne zu prüfen.
    async fn event_from(
        &self,
        token: &str,
        event_id: &str,
        kind: &str,
        amount: i64,
        profile: Option<&str>,
        occurred_at: Option<i64>,
    ) -> (StatusCode, Value) {
        let mut body = json!({ "event_id": event_id, "type": kind, "amount": amount });
        if let Some(p) = profile {
            body["profile"] = json!(p);
        }
        if let Some(t) = occurred_at {
            body["occurred_at"] = json!(t);
        }
        self.post(token, &format!("/api/companies/{}/events", self.id), body)
            .await
    }

    async fn rejected(&self, token: &str) -> (StatusCode, Value) {
        self.get(
            token,
            &format!("/api/companies/{}/rejected-events", self.id),
        )
        .await
    }
}

#[tokio::test]
async fn firmenprofil_melden_und_sehen() {
    let f = Firma::new().await;
    // Noch nichts gemeldet
    assert!(f.member(f.fahrer_id).await["profile_name"].is_null());

    // Der Fahrer meldet sein Firmenprofil, Leerraum am Rand zählt nicht
    let (s, r) = f
        .report_profile(&f.fahrer, json!("  Nordfracht Müller  "))
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["profile_name"], "Nordfracht Müller");
    let (s, _) = f.report_profile(&f.chef, json!("Chefin")).await;
    assert_eq!(s, StatusCode::OK);

    // Der Chef sieht die Profile aller, der Fahrer nur seins
    assert_eq!(
        f.member_as(&f.chef, f.fahrer_id).await["profile_name"],
        "Nordfracht Müller"
    );
    assert_eq!(
        f.member_as(&f.chef, f.chef_id).await["profile_name"],
        "Chefin"
    );
    assert_eq!(
        f.member_as(&f.fahrer, f.fahrer_id).await["profile_name"],
        "Nordfracht Müller"
    );
    let chef_seen_by_driver = f.member_as(&f.fahrer, f.chef_id).await;
    assert!(
        chef_seen_by_driver.get("profile_name").is_none(),
        "{chef_seen_by_driver}"
    );

    // Unsinn: zu lang, Steuerzeichen
    for bad in [json!("x".repeat(101)), json!("Nord\nfracht")] {
        let (s, v) = f.report_profile(&f.fahrer, bad).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    assert_eq!(
        f.member(f.fahrer_id).await["profile_name"],
        "Nordfracht Müller",
        "unverändert"
    );

    // Fremde, ohne Anmeldung, unbekannte Firma
    let fremder = register(&f.app, "Fremder").await;
    let (s, _) = f.report_profile(&fremder, json!("X")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(
        &f.app,
        "PUT",
        &format!("/api/companies/{}/profile", f.id),
        None,
        Some(json!({ "profile_name": "X" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = f
        .put(
            &f.fahrer,
            "/api/companies/999/profile",
            json!({ "profile_name": "X" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Leer oder null löscht das gemeldete Profil
    let (s, r) = f.report_profile(&f.fahrer, json!("")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(r["profile_name"].is_null());
    assert!(f.member(f.fahrer_id).await["profile_name"].is_null());
    let (s, _) = f.report_profile(&f.fahrer, Value::Null).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn ereignis_aus_dem_firmenprofil_wird_gebucht_andere_abgelehnt() {
    let f = Firma::new().await;
    let (s, _) = f.report_profile(&f.fahrer, json!("Nordfracht")).await;
    assert_eq!(s, StatusCode::OK);
    first_sync(&f).await;

    // Aus dem Firmenprofil: gebucht, mit Profil im Kassenbuch
    let (s, r) = f
        .event_from(
            &f.fahrer,
            "a-1",
            "job_delivered",
            1000,
            Some("Nordfracht"),
            Some(t(5)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(f.balance().await, 1000);
    let l = f.ledger().await;
    assert_eq!(l.len(), 1);
    assert_eq!(l[0]["profile"], "Nordfracht");

    // Aus einem privaten Profil: nicht gebucht, 422 mit Meldung, protokolliert
    let (s, v) = f
        .event_from(
            &f.fahrer,
            "p-1",
            "job_delivered",
            10_000,
            Some("Privat"),
            Some(t(10)),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let msg = v["error"].as_str().unwrap();
    assert!(
        msg.contains("'Privat'") && msg.contains("'Nordfracht'"),
        "{msg}"
    );
    let (s, _) = f
        .event_from(&f.fahrer, "p-2", "fined", 300, Some("Privat"), Some(t(11)))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    // Andere Schreibweise ist ein anderes Profil, Leerraum am Rand zählt nicht
    let (s, _) = f
        .event_from(
            &f.fahrer,
            "p-3",
            "tollgate",
            40,
            Some("nordfracht"),
            Some(t(12)),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, r) = f
        .event_from(
            &f.fahrer,
            "a-2",
            "tollgate",
            40,
            Some(" Nordfracht "),
            Some(t(13)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    // Ohne Profil, etwa von einem älteren Plugin: ebenso abgelehnt
    let (s, v) = f
        .event_from(&f.fahrer, "o-1", "fined", 100, None, Some(t(14)))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(v["error"].as_str().unwrap().contains("kein Profil"));
    // Noch einmal geschickt: wieder abgelehnt, aber nur einmal protokolliert
    let (s, _) = f
        .event_from(
            &f.fahrer,
            "p-1",
            "job_delivered",
            10_000,
            Some("Privat"),
            Some(t(10)),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    // Nichts davon ist gebucht: Kasse, Lohnkonto, Kassenbuch
    assert_eq!(f.balance().await, 1000 - 40);
    assert_eq!(f.wallet(f.fahrer_id).await, 0);
    let l = f.ledger().await;
    assert_eq!(l.len(), 2, "{l:?}");
    assert!(l.iter().all(|e| e["profile"] == "Nordfracht"));

    // Der Geldabgleich zählt nur gebuchte Ereignisse: keine Betriebskosten aus dem privaten
    // Auftrag über 10000 Euro
    let r = sync_written_at(&f, 2000 + 1000 - 40, 30).await;
    assert_eq!(n(&r["expected"]), 2000 + 1000 - 40, "{r}");
    assert_eq!(r["booked"], json!([]));
    assert!(r["unexplained"].is_null());

    // Der Chef sieht die abgelehnten Ereignisse, neueste zuerst
    let (s, list) = f.rejected(&f.chef).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let e = list["entries"].as_array().unwrap();
    assert_eq!(e.len(), 4, "{list}");
    let ids: Vec<&str> = e.iter().map(|x| x["event_id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["o-1", "p-3", "p-2", "p-1"]);
    let p1 = &e[3];
    assert_eq!(p1["user"], "Fahrer");
    assert_eq!(n(&p1["user_id"]), f.fahrer_id);
    assert_eq!(p1["type"], "job_delivered");
    assert_eq!(n(&p1["amount"]), 10_000);
    assert_eq!(p1["profile"], "Privat");
    assert!(p1["reason"].as_str().unwrap().contains("'Privat'"));
    // Abgelehnt, als die Uhr des Servers noch auf 10:00 stand
    assert_eq!(n(&p1["created_at"]), t(0));
    assert_eq!(p1["payload"]["event_id"], "p-1");
    assert_eq!(n(&p1["payload"]["occurred_at"]), t(10));
    assert!(e[0]["profile"].is_null());
    assert!(e[0]["reason"].as_str().unwrap().contains("ohne Profil"));

    // Der Fahrer sieht die Liste nicht
    let (s, _) = f.rejected(&f.fahrer).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn ohne_gemeldetes_firmenprofil_wird_wie_bisher_gebucht() {
    let f = Firma::new().await;
    // Kein Profil gemeldet, etwa mit einem älteren Launcher: alles wird gebucht
    let (s, _) = f
        .event_from(&f.fahrer, "a-1", "job_delivered", 1000, None, None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = f
        .event_from(&f.fahrer, "a-2", "tollgate", 40, Some("Privat"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(f.balance().await, 960);
    // Das Profil steht trotzdem an der Buchung
    let l = f.ledger().await;
    let by = |id: &str| l.iter().find(|e| e["event_id"] == id).unwrap().clone();
    assert!(by("a-1")["profile"].is_null());
    assert_eq!(by("a-2")["profile"], "Privat");
    let (_, list) = f.rejected(&f.chef).await;
    assert_eq!(list["entries"], json!([]));

    // Das Firmenprofil eines anderen Fahrers gilt nicht für diesen
    let (s, _) = f.report_profile(&f.chef, json!("Chefin")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = f
        .event_from(&f.fahrer, "a-3", "tollgate", 10, Some("Privat"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn herzschlag_aus_anderem_profil_aendert_presence_nicht() {
    let f = Firma::new().await;
    let truck = f.truck("SPED 001", 1000, full_state(1000)).await;
    f.checkout(&f.fahrer, truck).await;
    // Ohne gemeldetes Profil zählt jeder Herzschlag
    let (s, r) = f
        .beat(
            &f.fahrer,
            json!({ "plate": "SPED 001", "odometer_km": 1100, "profile": "Privat" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["recorded"], true);
    let start = f.clock.now();
    assert_eq!(n(&f.member(f.fahrer_id).await["last_seen"]), start);

    let (s, _) = f.report_profile(&f.fahrer, json!("Nordfracht")).await;
    assert_eq!(s, StatusCode::OK);

    // Aus dem privaten Profil und ohne Profil: 200, aber verworfen
    f.clock.set(start + 20);
    for body in [
        json!({ "plate": "SPED 001", "odometer_km": 5000, "speed_kmh": 80, "profile": "Privat" }),
        json!({ "plate": "SPED 001", "odometer_km": 5000 }),
    ] {
        let (s, r) = f.beat(&f.fahrer, body.clone()).await;
        assert_eq!(s, StatusCode::OK, "{body} {r}");
        assert_eq!(r["recorded"], false, "{r}");
        assert!(r["note"].as_str().unwrap().contains("Nordfracht"), "{r}");
        assert!(r["leased_truck_id"].is_null());
    }
    // Presence und letzter bekannter Zustand bleiben, wie sie waren
    let m = f.member(f.fahrer_id).await;
    assert_eq!(n(&m["last_seen"]), start);
    assert!(m["speed_kmh"].is_null(), "{m}");
    let (_, _, seen_at, last_km, _) = f.truck_row(truck);
    assert_eq!((seen_at, last_km), (Some(start), Some(1100)));
    // Nach 31 Sekunden offline, die verworfenen Herzschläge halten ihn nicht online
    f.clock.set(start + 31);
    assert_eq!(f.member(f.fahrer_id).await["online"], false);

    // Aus dem Firmenprofil: wieder online
    let (s, r) = f
        .beat(
            &f.fahrer,
            json!({ "plate": "SPED 001", "odometer_km": 1200, "profile": "Nordfracht" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r["recorded"], true);
    assert_eq!(n(&r["leased_truck_id"]), truck);
    let m = f.member(f.fahrer_id).await;
    assert_eq!(m["online"], true);
    assert_eq!(n(&m["last_seen"]), start + 31);
    assert_eq!(f.truck_row(truck).3, Some(1200));
}

#[tokio::test]
async fn alte_datenbank_bekommt_firmenprofil_und_protokoll() {
    let path = std::env::temp_dir().join(format!("spedhub-profil-{}.db", std::process::id()));
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
             INSERT INTO ledger VALUES (1, 1, 'firma', 1, 'maut', -50, 'toll-1', '', 2000);",
        )
        .unwrap();
    }
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    {
        let conn = db.lock().unwrap();
        // Die alte Mitgliedschaft und die alte Buchung haben kein Profil
        let profile: Option<String> = conn
            .query_row("SELECT profile_name FROM memberships", [], |r| r.get(0))
            .unwrap();
        assert_eq!(profile, None);
        let (amount, profile): (i64, Option<String>) = conn
            .query_row("SELECT amount, profile FROM ledger", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((amount, profile), (-50, None));
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM rejected_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
    // Zweites Öffnen geht auch, danach läuft alles wie in einer neuen Datenbank
    drop(db);
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    let f = Firma::with_db(db).await;
    let (s, _) = f.report_profile(&f.fahrer, json!("Nordfracht")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = f
        .event_from(&f.fahrer, "x-1", "tollgate", 10, Some("Privat"), None)
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = f
        .event_from(&f.fahrer, "x-2", "tollgate", 10, Some("Nordfracht"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, list) = f.rejected(&f.chef).await;
    assert_eq!(list["entries"].as_array().unwrap().len(), 1);
    drop(f);
    let _ = std::fs::remove_file(&path);
}

// ------------------------------------------------------------ Storno

impl Firma {
    /// Buchungen stornieren, wie der Chef im Kassenbuch
    async fn reverse(&self, token: &str, ids: &[i64], reason: &str) -> (StatusCode, Value) {
        self.post(
            token,
            &format!("/api/companies/{}/ledger/reverse", self.id),
            json!({ "entry_ids": ids, "reason": reason }),
        )
        .await
    }

    /// Stornieren, das klappen muss
    async fn reverse_ok(&self, ids: &[i64], reason: &str) -> Value {
        let (s, r) = self.reverse(&self.chef, ids, reason).await;
        assert_eq!(s, StatusCode::OK, "{r}");
        r
    }

    /// id der Buchung eines Ereignisses
    async fn entry_id(&self, event_id: &str) -> i64 {
        let l = self.ledger().await;
        n(&l.iter()
            .find(|e| e["event_id"] == event_id)
            .unwrap_or_else(|| panic!("kein Ereignis {event_id}"))["id"])
    }

    /// Diff, gebuchte und erlassene Betriebskosten eines Abgleichs
    fn sync_row(&self, sync_id: i64) -> (i64, i64, i64) {
        let conn = self.db.lock().unwrap();
        conn.query_row(
            "SELECT diff, costs_booked, costs_waived FROM sync_points WHERE id = ?1",
            [sync_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    /// Wirkung eines Stornos für den Fahrer
    fn driver_of<'a>(&self, r: &'a Value) -> &'a Value {
        self.driver_of_user(r, self.fahrer_id)
    }

    fn driver_of_user<'a>(&self, r: &'a Value, user_id: i64) -> &'a Value {
        r["drivers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["user_id"] == user_id)
            .unwrap_or_else(|| panic!("kein {user_id} in {r}"))
    }

    fn ledger_count(&self) -> i64 {
        let conn = self.db.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get(0))
            .unwrap()
    }
}

#[tokio::test]
async fn storno_einer_buchung() {
    let f = Firma::new().await;
    f.event(&f.fahrer, "job-1", "job_delivered", 1000, Some(400.0))
        .await;
    let job = f.entry_id("job-1").await;
    let before = f.entry(job).await;
    assert_eq!(before["reversible"], true);
    assert!(before["reversed_by"].is_null());
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 400);

    f.clock.advance(3600);
    let now = f.clock.now();
    let r = f.reverse_ok(&[job], "  Aus dem privaten Profil  ").await;
    let e = r["entries"].as_array().unwrap();
    assert_eq!(e.len(), 1, "{r}");
    let storno = &e[0];
    assert_eq!(
        (
            &storno["kind"],
            &storno["account"],
            n(&storno["amount"]),
            n(&storno["reverses_id"]),
            n(&storno["user_id"]),
            &storno["note"],
            n(&storno["created_at"])
        ),
        (
            &json!("storno"),
            &json!("firma"),
            -1000,
            job,
            f.fahrer_id,
            &json!("Aus dem privaten Profil"),
            now
        )
    );
    assert_eq!(n(&r["balance"]), 0);
    let d = f.driver_of(&r);
    assert_eq!(d["user"], "Fahrer");
    assert_eq!(n(&d["wallet"]), 0);
    // Noch kein Abgleich: zählt nirgends
    assert_eq!(
        (&d["corrections"], n(&d["open_window"]), &d["waived"]),
        (&json!([]), 0, &json!([]))
    );

    // Das Original bleibt, wie es war, und sagt, wer es wann warum storniert hat
    let after = f.entry(job).await;
    for k in [
        "account",
        "kind",
        "amount",
        "note",
        "created_at",
        "event_id",
        "distance_km",
        "game_amount",
        "user_id",
    ] {
        assert_eq!(after[k], before[k], "{k}");
    }
    assert_eq!(after["reversed_by"], storno["id"]);
    assert_eq!(n(&after["reversed_at"]), now);
    assert_eq!(after["reverse_reason"], "Aus dem privaten Profil");
    assert_eq!(after["reversed_by_user"], "Chefin");
    assert_eq!(after["reversible"], false);
    // Das Storno verweist auf das Original, mit dessen Art und Zeitpunkt
    let s = f.entry(n(&storno["id"])).await;
    assert_eq!(n(&s["reverses_id"]), job);
    assert_eq!(s["reverses_kind"], "auftrag");
    assert_eq!(s["reverses_created_at"], before["created_at"]);
    assert_eq!(s["booked_by_user"], "Chefin");
    assert_eq!(s["user"], "Fahrer");
    assert_eq!(s["reversible"], false);
    assert!(s["event_id"].is_null() && s["game_amount"].is_null());
    // In der Datenbank: Original unverändert, Storno mit dem Chef in booked_by
    {
        let conn = f.db.lock().unwrap();
        let (amount, kind, rev): (i64, String, Option<i64>) = conn
            .query_row(
                "SELECT amount, kind, reverses_id FROM ledger WHERE id = ?1",
                [job],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((amount, kind.as_str(), rev), (1000, "auftrag", None));
        let by: i64 = conn
            .query_row(
                "SELECT booked_by FROM ledger WHERE id = ?1",
                [n(&storno["id"])],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(by, f.chef_id);
    }
    // Kasse und Firmenkilometer ohne den Auftrag. Der Fahrer sieht das Storno auch.
    assert_eq!(f.balance().await, 0);
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 0);
    let (s, l) = f
        .get(&f.fahrer, &format!("/api/companies/{}/ledger", f.id))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(l["entries"][0]["kind"], "storno");
    assert_eq!(l["entries"][1]["reverse_reason"], "Aus dem privaten Profil");
}

#[tokio::test]
async fn storno_mehrerer_buchungen_alles_oder_nichts() {
    let f = Firma::new().await;
    f.event(&f.fahrer, "a", "job_delivered", 1000, None).await;
    f.event(&f.fahrer, "b", "tollgate", 40, None).await;
    f.event(&f.fahrer, "c", "fined", 100, None).await;
    let (a, b, c) = (
        f.entry_id("a").await,
        f.entry_id("b").await,
        f.entry_id("c").await,
    );
    let count = f.ledger_count();

    // Eine unbekannte id: nichts gebucht, die Meldung nennt sie
    let (s, v) = f
        .reverse(&f.chef, &[a, b, 999_999], "privates Profil")
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("Buchung 999999"),
        "{v}"
    );
    assert_eq!(f.ledger_count(), count);
    assert_eq!(f.balance().await, 960);

    // Mehrere auf einmal, doppelt genannte zählen einmal
    let r = f.reverse_ok(&[b, a, a], "privates Profil").await;
    let ids: Vec<i64> = r["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| n(&e["reverses_id"]))
        .collect();
    assert_eq!(ids, vec![a, b], "älteste zuerst");
    assert_eq!(n(&r["balance"]), 0);
    assert_eq!(f.ledger_count(), count + 2);

    // Doppeltes Storno: 409, auch zusammen mit einer stornierbaren, und c bleibt
    let (s, v) = f.reverse(&f.chef, &[c, a], "noch einmal").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let msg = v["error"].as_str().unwrap();
    assert!(
        msg.contains(&format!("Buchung {a} (Auftrag")) && msg.contains("schon storniert"),
        "{msg}"
    );
    assert_eq!(f.ledger_count(), count + 2);
    assert_eq!(f.entry(c).await["reversible"], true);
    assert_eq!(f.wallet(f.fahrer_id).await, -100);

    // Storno eines Stornos: 409
    let storno = n(&r["entries"][0]["id"]);
    let (s, v) = f.reverse(&f.chef, &[storno], "zurück").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("selbst ein Storno"),
        "{v}"
    );
    assert_eq!(f.ledger_count(), count + 2);

    // Die letzte geht
    f.reverse_ok(&[c], "privates Profil").await;
    assert_eq!(f.wallet(f.fahrer_id).await, 0);
    assert_eq!(f.balance().await, 0);
}

#[tokio::test]
async fn storno_nur_durch_den_chef_mit_grund() {
    let f = Firma::new().await;
    f.event(&f.fahrer, "a", "job_delivered", 1000, None).await;
    let a = f.entry_id("a").await;
    let count = f.ledger_count();

    // Fahrer: 403
    let (s, v) = f.reverse(&f.fahrer, &[a], "privates Profil").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    // Ohne Grund, zu kurz, zu lang, mit Zeilenumbruch: 400
    for reason in [
        json!(null),
        json!(""),
        json!("  ab  "),
        json!("x".repeat(201)),
        json!("a\nb c"),
    ] {
        let (s, v) = f
            .post(
                &f.chef,
                &format!("/api/companies/{}/ledger/reverse", f.id),
                json!({ "entry_ids": [a], "reason": reason }),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{reason} {v}");
    }
    let (s, v) = f
        .post(
            &f.chef,
            &format!("/api/companies/{}/ledger/reverse", f.id),
            json!({ "entry_ids": [a] }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("Grund"));
    // Keine oder zu viele Buchungen: 400
    let (s, _) = f.reverse(&f.chef, &[], "privates Profil").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let many: Vec<i64> = (1..=501).collect();
    let (s, v) = f.reverse(&f.chef, &many, "privates Profil").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("500"));

    // Eine Buchung aus einer fremden Firma: 404, und in der fremden Firma ist die Chefin nicht
    let fremdchef = register(&f.app, "Fremdchef").await;
    let (_, g) = call(
        &f.app,
        "POST",
        "/api/companies",
        Some(&fremdchef),
        Some(json!({ "name": "Südfracht" })),
    )
    .await;
    let gid = n(&g["id"]);
    let (s, _) = f
        .post(
            &fremdchef,
            &format!("/api/companies/{gid}/events"),
            json!({ "event_id": "x", "type": "job_delivered", "amount": 5000 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, gl) = f
        .get(&fremdchef, &format!("/api/companies/{gid}/ledger"))
        .await;
    let foreign = n(&gl["entries"][0]["id"]);
    let (s, v) = f.reverse(&f.chef, &[a, foreign], "privates Profil").await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains(&format!("Buchung {foreign}"))
    );
    let (s, _) = f
        .post(
            &f.chef,
            &format!("/api/companies/{gid}/ledger/reverse"),
            json!({ "entry_ids": [foreign], "reason": "fremd" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(f.ledger_count(), count + 1, "nur die Buchung der Südfracht");
    assert_eq!(f.balance().await, 1000);

    // Korrekturen aus dem Geldabgleich lassen sich nicht stornieren
    first_sync(&f).await;
    sync_written_at(&f, 1000, 30).await;
    f.clock.set(t(35));
    let e = f
        .event_at(&f.fahrer, "maut", "tollgate", 1000, None, Some(t(10)))
        .await;
    assert_eq!(e["late"]["booked"][0]["kind"], "betriebskosten_korrektur");
    let k = n(&f.entries("betriebskosten_korrektur").await[0]["id"]);
    assert_eq!(f.entry(k).await["reversible"], false);
    let (s, v) = f.reverse(&f.chef, &[k], "falsch").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("Korrektur"), "{v}");
}

#[tokio::test]
async fn storno_wirkt_auf_kasse_und_lohnkonto() {
    let f = Firma::new().await;
    f.event(&f.fahrer, "job", "job_delivered", 1000, Some(500.0))
        .await;
    // Lohn: 500 km mal 40 Cent plus 50 Euro Pauschale
    let (s, r) = f
        .post(
            &f.chef,
            &format!("/api/companies/{}/payrolls/run", f.id),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(f.wallet(f.fahrer_id).await, 250);
    assert_eq!(f.balance().await, 750);
    f.event(&f.fahrer, "strafe", "fined", 100, None).await;
    f.event(&f.fahrer, "abbruch", "job_cancelled", 70, None)
        .await;
    f.event(&f.fahrer, "maut", "tollgate", 50, None).await;
    f.event(&f.fahrer, "faehre", "ferry", 30, None).await;
    assert_eq!(f.wallet(f.fahrer_id).await, 80);
    assert_eq!(f.balance().await, 670);

    // Strafe und Abbruch storniert: Das Lohnkonto steigt wieder
    let ids = [f.entry_id("strafe").await, f.entry_id("abbruch").await];
    let r = f.reverse_ok(&ids, "privates Profil").await;
    assert_eq!(n(&f.driver_of(&r)["wallet"]), 250);
    assert_eq!(f.balance().await, 670);
    // Maut und Fähre: Die Kasse bekommt das Geld zurück
    let ids = [f.entry_id("maut").await, f.entry_id("faehre").await];
    let r = f.reverse_ok(&ids, "privates Profil").await;
    assert_eq!(n(&r["balance"]), 750);
    // Der Auftrag: Die Kasse verliert den Erlös, den gezahlten Lohn holt der Server nicht zurück
    let r = f
        .reverse_ok(&[f.entry_id("job").await], "privates Profil")
        .await;
    assert_eq!(n(&r["balance"]), -250);
    assert_eq!(f.wallet(f.fahrer_id).await, 250);
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 0);

    // Die Summe der Lohnabrechnung in der Kasse lässt sich nicht stornieren
    let total = f
        .entries("lohn")
        .await
        .into_iter()
        .find(|e| e["account"] == "firma")
        .unwrap();
    assert_eq!(total["reversible"], false);
    let (s, v) = f.reverse(&f.chef, &[n(&total["id"])], "zurück").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("Lohnbuchungen der einzelnen Fahrer")
    );
    // Die Lohnbuchung des Fahrers schon: Lohnkonto runter, Kasse rauf
    let runs = f.payrolls().await;
    let l = line(&runs[0], f.fahrer_id);
    let lohn = n(&l["entry_id"]);
    assert!(l["reversed_by"].is_null());
    assert_eq!(f.entry(lohn).await["reversible"], true);
    let r = f.reverse_ok(&[lohn], "Auftrag war privat").await;
    let e = r["entries"].as_array().unwrap();
    assert_eq!(e.len(), 2, "Storno und Gegenbuchung in der Kasse: {r}");
    let by = |account: &str| {
        e.iter()
            .find(|x| x["account"] == account)
            .unwrap_or_else(|| panic!("{account} fehlt in {r}"))
    };
    assert_eq!(
        (n(&by("fahrer")["amount"]), n(&by("firma")["amount"])),
        (-250, 250)
    );
    assert!(e.iter().all(|x| n(&x["reverses_id"]) == lohn));
    assert_eq!(n(&r["balance"]), 0);
    assert_eq!(n(&f.driver_of(&r)["wallet"]), 0);
    assert_eq!(f.wallet(f.fahrer_id).await, 0);
    // Die Abrechnung zeigt das Storno an der Zeile, die Lohnbuchung ist danach nicht mehr stornierbar
    let runs = f.payrolls().await;
    let l = line(&runs[0], f.fahrer_id);
    assert_eq!(l["reversed_by"], by("fahrer")["id"]);
    assert_eq!(l["reverse_reason"], "Auftrag war privat");
    assert_eq!(l["reversed_by_user"], "Chefin");
    let (s, _) = f.reverse(&f.chef, &[lohn], "noch einmal").await;
    assert_eq!(s, StatusCode::CONFLICT);
    // Alles zurück auf null, nichts verschwunden oder doppelt
    assert_eq!(f.balance().await, 0);
}

#[tokio::test]
async fn ueberweisung_und_haftung_werden_als_paar_storniert() {
    let f = Firma::new().await;
    f.event(&f.fahrer, "job", "job_delivered", 1000, None).await;
    let (s, _) = f
        .post(
            &f.chef,
            &format!("/api/companies/{}/transfer", f.id),
            json!({ "user_id": f.fahrer_id, "amount": 300, "note": "Vorschuss" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((f.balance().await, f.wallet(f.fahrer_id).await), (700, 300));
    let u = f.entries("ueberweisung").await;
    let side = |account: &str| n(&u.iter().find(|e| e["account"] == account).unwrap()["id"]);
    // Nur die Seite auf dem Lohnkonto gewählt: Die Kasse wird mit storniert
    let r = f.reverse_ok(&[side("fahrer")], "versehentlich").await;
    assert_eq!(r["entries"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!((f.balance().await, f.wallet(f.fahrer_id).await), (1000, 0));
    let (s, v) = f.reverse(&f.chef, &[side("firma")], "versehentlich").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");

    // Haftung: 6 Prozentpunkte neuer Schaden, die Hälfte von 6 mal 150 Euro
    f.patch(&f.chef, json!({ "liability_percent": 50 })).await;
    let truck = f
        .truck("SPED 001", 1000, worn(1000, 0.0, 0.0, 0.0, 0.4, &[0.0]))
        .await;
    let r = f
        .drive(truck, 1100, worn(1100, 0.2, 0.0, 0.0, 0.4, &[0.0]))
        .await;
    assert_eq!(r["liability"], 450);
    assert_eq!(
        (f.balance().await, f.wallet(f.fahrer_id).await),
        (1450, -450)
    );
    let h = f.entries("haftung").await;
    let firma = n(&h.iter().find(|e| e["account"] == "firma").unwrap()["id"]);
    let r = f.reverse_ok(&[firma], "Schaden war schon da").await;
    assert_eq!(r["entries"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!((f.balance().await, f.wallet(f.fahrer_id).await), (1000, 0));
    assert!(
        f.entries("haftung")
            .await
            .iter()
            .all(|e| e["reversible"] == false)
    );
}

#[tokio::test]
async fn storniertes_ereignis_im_offenen_fenster_zaehlt_nicht() {
    let f = Firma::new().await;
    first_sync(&f).await;
    // Ein privater Auftrag mit Zeitpunkt und eine Maut ohne Zeitpunkt, beide nach dem Abgleich
    f.clock.set(t(5));
    f.event_at(&f.fahrer, "a", "job_delivered", 1000, None, Some(t(5)))
        .await;
    f.event(&f.fahrer, "b", "tollgate", 40, None).await;
    let ids = [f.entry_id("a").await, f.entry_id("b").await];
    let r = f.reverse_ok(&ids, "privates Profil").await;
    let d = f.driver_of(&r);
    assert_eq!(
        (&d["corrections"], n(&d["open_window"])),
        (&json!([]), 2),
        "{r}"
    );
    // Im Spiel der Firma ist nichts passiert: Der Abgleich erwartet 2000 und bucht nichts
    let r = sync_written_at(&f, 2000, 30).await;
    assert_eq!((n(&r["expected"]), n(&r["events"])), (2000, 0));
    assert_eq!(r["booked"], json!([]));
    assert!(r["unexplained"].is_null());
    assert_eq!(f.balance().await, 0);
}

#[tokio::test]
async fn storniertes_ereignis_in_abgeglichenem_fenster_bucht_korrektur() {
    let f = Firma::new().await;
    first_sync(&f).await;
    // Privater Auftrag um 10:05: Der Abgleich zählt ihn, im Spiel der Firma fehlt er
    f.clock.set(t(5));
    f.event_at(&f.fahrer, "a", "job_delivered", 1000, None, Some(t(5)))
        .await;
    let s1 = sync_written_at(&f, 2000, 30).await;
    assert_eq!(
        s1["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -1000 }])
    );
    let sid = n(&s1["sync_id"]);
    // Die Betriebskosten tragen ihren Abgleich
    let costs = f.entries("betriebskosten").await;
    {
        let conn = f.db.lock().unwrap();
        let sync_id: Option<i64> = conn
            .query_row(
                "SELECT sync_id FROM ledger WHERE id = ?1",
                [n(&costs[0]["id"])],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sync_id, Some(sid));
    }
    assert_eq!(f.balance().await, 0);

    f.clock.set(t(40));
    let r = f
        .reverse_ok(&[f.entry_id("a").await], "privates Profil")
        .await;
    let d = f.driver_of(&r);
    assert_eq!(n(&d["open_window"]), 0);
    let c = &d["corrections"][0];
    assert_eq!(n(&c["sync_id"]), sid);
    assert_eq!((n(&c["diff_before"]), n(&c["diff"])), (-1000, 0));
    assert_eq!(n(&c["events"]), 1);
    assert_eq!(
        c["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten_korrektur", "amount": 1000 }])
    );
    // Erlös weg, Betriebskosten zurück: Die Firma hat, was sie wirklich hatte
    assert_eq!(n(&r["balance"]), 0);
    assert_eq!(f.costs().await, 0);
    assert_eq!(f.sync_row(sid), (0, 0, 0));
    let k = &f.entries("betriebskosten_korrektur").await[0];
    let note = k["note"].as_str().unwrap();
    assert!(
        note.starts_with("Storno: Auftrag, Abgleich vom 16.11.2026 10:31"),
        "{note}"
    );
    // Der nächste Abgleich bucht nichts
    let r = sync_written_at(&f, 2000, 60).await;
    assert_eq!(r["booked"], json!([]));
    assert!(r["unexplained"].is_null());
}

#[tokio::test]
async fn stornierte_strafe_erklaert_ungeklaerten_betrag() {
    let f = Firma::new().await;
    first_sync(&f).await;
    // Private Strafe um 10:05: Der Abgleich erwartet 300 weniger, als im Spiel der Firma ist
    f.clock.set(t(5));
    f.event_at(&f.fahrer, "s", "fined", 300, None, Some(t(5)))
        .await;
    let r = sync_written_at(&f, 2000, 30).await;
    assert_eq!(n(&r["unexplained"]["amount"]), 300);
    assert_eq!(n(&r["write_money"]), 1700);
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 1);

    f.clock.set(t(40));
    let r = f
        .reverse_ok(&[f.entry_id("s").await], "privates Profil")
        .await;
    let c = &f.driver_of(&r)["corrections"][0];
    assert_eq!((n(&c["diff_before"]), n(&c["diff"])), (300, 0));
    assert_eq!(c["booked"], json!([]));
    assert_eq!(c["unexplained"]["closed"], true);
    let u = f.unexplained().await;
    assert_eq!(u[0]["note"], "Durch stornierte Ereignisse erklärt");
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 0);
    // Das Lohnkonto hat die 300 zurück, der nächste Abgleich schreibt sie ins Spiel
    assert_eq!(n(&f.driver_of(&r)["wallet"]), 0);
    let r = sync_written_at(&f, 1700, 60).await;
    assert_eq!(r["booked"], json!([]));
    assert!(r["unexplained"].is_null());
    assert_eq!(n(&r["write_money"]), 2000);
}

#[tokio::test]
async fn storniertes_ereignis_ohne_zeitpunkt_im_abgeglichenen_fenster() {
    let f = Firma::new().await;
    // Vor dem ersten Abgleich: steckt im Anfangsstand, ein Storno ändert am Abgleich nichts
    f.event(&f.fahrer, "vorher", "job_delivered", 500, None)
        .await;
    first_sync(&f).await;
    // Ohne Zeitpunkt nach dem ersten Abgleich: zählt im zweiten
    f.clock.set(t(5));
    f.event(&f.fahrer, "a", "job_delivered", 1000, None).await;
    let s1 = sync_written_at(&f, 2000, 30).await;
    assert_eq!(n(&s1["events"]), 1);
    assert_eq!(n(&s1["booked"][0]["amount"]), -1000);
    f.clock.set(t(40));
    let ids = [f.entry_id("vorher").await, f.entry_id("a").await];
    let r = f.reverse_ok(&ids, "privates Profil").await;
    let d = f.driver_of(&r);
    assert_eq!(d["corrections"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(n(&d["corrections"][0]["sync_id"]), n(&s1["sync_id"]));
    assert_eq!(n(&d["open_window"]), 0);
    assert_eq!(f.costs().await, 0);
    assert_eq!(f.balance().await, 0);
}

/// Der zweite Test im Spiel: Ereignisse aus einem privaten Profil sind gebucht, und der
/// Abgleich des Firmenprofils hat daraus falsche Betriebskosten gebucht. Der Chef storniert die
/// Ereignisse über den Zeitraum, danach stimmt die Kasse.
#[tokio::test]
async fn privates_profil_nachtraeglich_stornieren() {
    let f = Firma::new().await;
    first_sync(&f).await;
    // Ohne gemeldetes Firmenprofil bucht der Server alles, auch aus "Privat"
    let private: [(&str, &str, i64, Option<f64>); 12] = [
        ("p1", "job_delivered", 1000, Some(300.0)),
        ("p2", "tollgate", 50, None),
        ("p3", "fined", 300, None),
        ("p4", "job_delivered", 2000, Some(600.0)),
        ("p5", "tollgate", 40, None),
        ("p6", "ferry", 200, None),
        ("p7", "job_cancelled", 500, None),
        ("p8", "tollgate", 30, None),
        ("p9", "train", 150, None),
        ("p10", "fined", 100, None),
        ("p11", "job_delivered", 1500, Some(450.0)),
        ("p12", "tollgate", 20, None),
    ];
    for (i, (id, kind, amount, km)) in private.iter().enumerate() {
        let m = 5 + 3 * i as i64;
        f.clock.set(t(m));
        let mut body = json!({ "event_id": id, "type": kind, "amount": amount,
                               "occurred_at": t(m), "profile": "Privat" });
        if let Some(km) = km {
            body["distance_km"] = json!(km);
        }
        let (s, v) = f
            .post(&f.fahrer, &format!("/api/companies/{}/events", f.id), body)
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    // Spielbeträge 4500 - 300 - 100 - 500 - 140 - 200 - 150 = 3110, Kasse plus 4010
    assert_eq!(f.balance().await, 4010);
    assert_eq!(f.wallet(f.fahrer_id).await, -900);
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 1350);

    // Im Firmenprofil hat der Fahrer nur für 200 Euro getankt. Der Abgleich erwartet 5110.
    let s1 = sync_written_at(&f, 1800, 60).await;
    assert_eq!(n(&s1["expected"]), 5110);
    assert_eq!(n(&s1["booked"][0]["amount"]), -3310);
    assert_eq!(n(&s1["write_money"]), 1100);
    assert_eq!(f.balance().await, 700);

    // Der Chef sucht den Zeitraum heraus: nur die privaten Ereignisse, ohne die Betriebskosten
    f.clock.set(t(120));
    let (s, l) = f
        .get(
            &f.chef,
            &format!(
                "/api/companies/{}/ledger?limit=500&from={}&to={}",
                f.id,
                t(5),
                t(60)
            ),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{l}");
    let ids: Vec<i64> = l["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["reversible"] == true)
        .map(|e| n(&e["id"]))
        .collect();
    assert_eq!(ids.len(), 12, "{l}");
    let r = f
        .reverse_ok(&ids, "Ereignisse aus dem privaten Profil")
        .await;
    assert_eq!(r["entries"].as_array().unwrap().len(), 12);
    let d = f.driver_of(&r);
    let c = &d["corrections"][0];
    assert_eq!(
        d["corrections"].as_array().unwrap().len(),
        1,
        "ein Fenster: {r}"
    );
    assert_eq!(n(&c["events"]), 12);
    assert_eq!((n(&c["diff_before"]), n(&c["diff"])), (-3310, -200));
    assert_eq!(n(&c["booked"][0]["amount"]), 3110);
    assert!(
        c["booked"][0]["kind"] == "betriebskosten_korrektur"
            && f.entries("betriebskosten_korrektur").await[0]["note"]
                .as_str()
                .unwrap()
                .starts_with("Storno: 12 Ereignisse, Abgleich vom")
    );

    // Die Kasse hat genau das Tanken bezahlt, das Lohnkonto ist wieder bei 0
    assert_eq!(n(&r["balance"]), -200);
    assert_eq!(f.costs().await, -200);
    assert_eq!(n(&d["wallet"]), 0);
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 0);
    assert_eq!(f.company(&f.chef).await["open_unexplained"], 0);
    // Der nächste Abgleich findet den geschriebenen Stand und bucht nichts Falsches
    let s2 = sync_written_at(&f, 1100, 150).await;
    assert_eq!(n(&s2["expected"]), 1100);
    assert_eq!(s2["booked"], json!([]));
    assert!(s2["unexplained"].is_null());
    assert_eq!(n(&s2["write_money"]), 2000);
    assert_eq!(f.balance().await, -200);
    // Und die Lohnabrechnung zahlt für die privaten Aufträge nichts
    let (s, r) = f
        .post(
            &f.chef,
            &format!("/api/companies/{}/payrolls/run", f.id),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(f.wallet(f.fahrer_id).await, 0);
}

/// Erst die Ereignisse stornieren, dann die Betriebskosten: Die sind dann schon korrigiert,
/// ein Storno gäbe zu viel zurück und wird abgelehnt.
#[tokio::test]
async fn betriebskosten_nach_den_ereignissen_stornieren() {
    let f = Firma::new().await;
    first_sync(&f).await;
    f.clock.set(t(5));
    f.event_at(&f.fahrer, "a", "job_delivered", 1000, None, Some(t(5)))
        .await;
    sync_written_at(&f, 2000, 30).await;
    let costs = n(&f.entries("betriebskosten").await[0]["id"]);
    f.clock.set(t(35));
    f.reverse_ok(&[f.entry_id("a").await], "privates Profil")
        .await;
    assert_eq!(f.balance().await, 0);
    let count = f.ledger_count();
    let (s, v) = f.reverse(&f.chef, &[costs], "privates Profil").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let msg = v["error"].as_str().unwrap();
    assert!(
        msg.contains(&format!("Buchung {costs} (Betriebskosten"))
            && msg.contains("keine Betriebskosten mehr gebucht")
            && msg.contains("1000 € zu viel"),
        "{msg}"
    );
    assert_eq!(f.ledger_count(), count);
    assert_eq!(f.balance().await, 0);

    // Zweites Fenster: privater Auftrag über 1000 und echtes Tanken für 300
    f.clock.set(t(40));
    f.event_at(&f.fahrer, "b", "job_delivered", 1000, None, Some(t(40)))
        .await;
    let s2 = sync_written_at(&f, 1700, 60).await;
    assert_eq!(n(&s2["booked"][0]["amount"]), -1300);
    let costs2 = n(&f.entries("betriebskosten").await[0]["id"]);
    f.clock.set(t(65));
    // Ereignis und Betriebskosten zusammen: erst die Ereignisse, dann ist die Buchung zu groß
    let (s, v) = f
        .reverse(&f.chef, &[costs2, f.entry_id("b").await], "privates Profil")
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let msg = v["error"].as_str().unwrap();
    assert!(
        msg.contains("nur noch 300 € Betriebskosten gebucht") && msg.contains("1000 € zu viel"),
        "{msg}"
    );
    assert_eq!(
        f.ledger_count(),
        count + 2,
        "nur Auftrag und Betriebskosten"
    );
    // Nur das Ereignis: Das Tanken bleibt bei der Firma
    f.reverse_ok(&[f.entry_id("b").await], "privates Profil")
        .await;
    assert_eq!(f.balance().await, -300);
    assert_eq!(f.costs().await, -300);
}

/// Erst die Betriebskosten stornieren, dann die Ereignisse: Die Firma hat die Kosten erlassen,
/// das Storno der Ereignisse gibt nichts ein zweites Mal zurück. Spätere Korrekturen legen die
/// erlassenen Kosten nicht wieder an.
#[tokio::test]
async fn betriebskosten_vor_den_ereignissen_stornieren() {
    let f = Firma::new().await;
    first_sync(&f).await;
    f.clock.set(t(5));
    f.event_at(&f.fahrer, "a", "job_delivered", 1000, None, Some(t(5)))
        .await;
    let s1 = sync_written_at(&f, 2000, 30).await;
    let sid = n(&s1["sync_id"]);
    let costs = n(&f.entries("betriebskosten").await[0]["id"]);
    f.clock.set(t(35));
    let r = f
        .reverse_ok(&[costs], "kommt aus dem privaten Auftrag")
        .await;
    assert_eq!(
        f.driver_of(&r)["waived"],
        json!([{ "sync_id": sid, "amount": 1000 }])
    );
    assert_eq!(n(&r["balance"]), 1000);
    assert_eq!(f.sync_row(sid), (-1000, 0, 1000));
    // Jetzt das Ereignis: keine Korrektur, die Kosten sind schon erlassen
    let r = f
        .reverse_ok(&[f.entry_id("a").await], "privates Profil")
        .await;
    let c = &f.driver_of(&r)["corrections"][0];
    assert_eq!((n(&c["diff_before"]), n(&c["diff"])), (-1000, 0));
    assert_eq!(c["booked"], json!([]));
    assert_eq!(n(&r["balance"]), 0);
    assert_eq!(
        f.sync_row(sid),
        (0, 0, 0),
        "der Auftrag erklärt den erlassenen Teil"
    );
    // Später gemeldet: ein echter Auftrag über 500 im selben Fenster. Im Spiel fehlen jetzt
    // 500, das sind neue Kosten und die bucht der Server wie in der anderen Reihenfolge.
    f.clock.set(t(40));
    let e = f
        .event_at(&f.fahrer, "echt", "job_delivered", 500, None, Some(t(20)))
        .await;
    assert_eq!(n(&e["late"]["booked"][0]["amount"]), -500);
    assert_eq!(f.balance().await, 0);

    // Zweites Fenster: echte Maut über 800, erst nach dem Abgleich gemeldet. Der Chef hält die
    // Betriebskosten für falsch und storniert sie. Dann kommt die Maut: keine zweite Gutschrift.
    let s2 = sync_written_at(&f, 2000 - 800, 60).await;
    let sid2 = n(&s2["sync_id"]);
    let costs2 = n(&f.entries("betriebskosten").await[0]["id"]);
    f.clock.set(t(65));
    f.reverse_ok(&[costs2], "unklar").await;
    assert_eq!(f.sync_row(sid2), (-800, 0, 800));
    let before = f.balance().await;
    f.clock.set(t(70));
    let e = f
        .event_at(&f.fahrer, "maut", "tollgate", 800, None, Some(t(50)))
        .await;
    assert_eq!(e["late"]["booked"], json!([]));
    assert_eq!(f.balance().await, before - 800, "nur die Maut selbst");
    assert_eq!(f.sync_row(sid2), (0, 0, 0));

    // Drittes Fenster: erlassen, dann zeigt ein später Auftrag noch mehr Aufwand. Nur der
    // neue Teil wird gebucht, die erlassenen Kosten nicht wieder.
    let s3 = sync_written_at(&f, 2000 - 600, 90).await;
    let sid3 = n(&s3["sync_id"]);
    let costs3 = n(&f.entries("betriebskosten").await[0]["id"]);
    f.clock.set(t(95));
    f.reverse_ok(&[costs3], "unklar").await;
    f.clock.set(t(96));
    let e = f
        .event_at(&f.fahrer, "spaet", "job_delivered", 400, None, Some(t(80)))
        .await;
    assert_eq!(
        e["late"]["booked"],
        json!([{ "account": "firma", "kind": "betriebskosten", "amount": -400 }])
    );
    assert_eq!(f.sync_row(sid3), (-1000, 400, 600));
}

/// Betriebskosten aus einem Abgleich von vor dem Storno tragen kein `sync_id`. Der Server
/// ordnet sie über `last_ledger_id` zu, ohne die Buchung zu ändern.
#[tokio::test]
async fn alte_betriebskosten_ohne_abgleich_werden_zugeordnet() {
    let f = Firma::new().await;
    first_sync(&f).await;
    f.clock.set(t(5));
    f.event_at(&f.fahrer, "a", "job_delivered", 1000, None, Some(t(5)))
        .await;
    let s1 = sync_written_at(&f, 2000, 30).await;
    let sid = n(&s1["sync_id"]);
    // Ein zweiter Abgleich ohne neue Buchung hat dieselbe last_ledger_id
    let s2 = sync_written_at(&f, 2000, 31).await;
    assert_eq!(s2["booked"], json!([]));
    let costs = n(&f.entries("betriebskosten").await[0]["id"]);
    {
        // So sehen Betriebskosten aus der Zeit vor dem Storno aus
        let conn = f.db.lock().unwrap();
        conn.execute("UPDATE ledger SET sync_id = NULL WHERE id = ?1", [costs])
            .unwrap();
        assert_eq!(
            spedhub_server::reversal::cost_sync(&conn, f.id, costs).unwrap(),
            Some(sid)
        );
    }
    f.clock.set(t(40));
    f.reverse_ok(&[costs], "privates Profil").await;
    assert_eq!(f.sync_row(sid), (-1000, 0, 1000));
    f.reverse_ok(&[f.entry_id("a").await], "privates Profil")
        .await;
    assert_eq!(f.balance().await, 0, "nichts doppelt zurück");
    assert_eq!(f.costs().await, -1000);
}

#[tokio::test]
async fn storno_von_betriebskosten_und_anschaffungsbudget() {
    let f = Firma::new().await;
    full_cash(&f).await;
    // Betriebskosten von vor der Freigabe: Ihr Storno vergrößert das Budget nicht
    f.chef_sync(2000, -1).await;
    f.clock.set(t(2));
    f.event_at(&f.chef, "privat", "job_delivered", 700, None, Some(t(2)))
        .await;
    f.chef_sync(2000, 3).await;
    let old_costs = n(&f.entries("betriebskosten").await[0]["id"]);
    f.clock.set(t(5));
    f.budget(&f.chef, json!({ "amount": 50_000 })).await;
    f.reverse_ok(&[old_costs], "privat").await;
    assert_eq!(n(&f.budgets().await["open"]["rest"]), 50_000);
    let r = f.chef_sync(2000, 10).await;
    assert_eq!(n(&r["write_money"]), 52_000);

    // Nach der Freigabe: Ein privater Auftrag lässt den Abgleich 1000 Betriebskosten buchen,
    // die das Budget verbrauchen
    f.clock.set(t(15));
    f.event_at(&f.chef, "p2", "job_delivered", 1000, None, Some(t(15)))
        .await;
    let r = f.chef_sync(52_000, 20).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -1000);
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (49_000, 51_000));
    let costs = n(&f.entries("betriebskosten").await[0]["id"]);
    // Storniert die Chefin die Betriebskosten, ist das Budget wieder frei
    f.clock.set(t(25));
    f.reverse_ok(&[costs], "privater Auftrag").await;
    let b = &f.budgets().await["open"];
    assert_eq!((n(&b["consumed"]), n(&b["rest"])), (0, 50_000));
    // Der private Auftrag danach: Die Kosten sind erlassen, am Budget ändert sich nichts
    f.reverse_ok(&[f.entry_id("p2").await], "privat").await;
    assert_eq!(n(&f.budgets().await["open"]["rest"]), 50_000);
    // Der nächste Abgleich findet den geschriebenen Stand und schreibt das volle Budget
    let r = f.chef_sync(51_000, 30).await;
    assert_eq!(r["booked"], json!([]));
    assert_eq!((n(&r["budget"]), n(&r["write_money"])), (50_000, 52_000));
    // Noch der private Auftrag von vor der Freigabe: keine Korrektur, die Kosten sind erlassen
    let r = f.reverse_ok(&[f.entry_id("privat").await], "privat").await;
    assert_eq!(
        f.driver_of_user(&r, f.chef_id)["corrections"][0]["booked"],
        json!([])
    );
    assert_eq!(n(&f.budgets().await["open"]["rest"]), 50_000);
    assert_eq!(f.balance().await, 100_000);

    // Nach dem Schließen gibt ein Storno nichts mehr zurück
    f.clock.set(t(35));
    let r = f.chef_sync(52_000 - 400, 40).await;
    assert_eq!(n(&r["booked"][0]["amount"]), -400);
    let costs = n(&f.entries("betriebskosten").await[0]["id"]);
    let open = f.budgets().await["open"].clone();
    let (s, _) = f.close_budget(&f.chef, &open).await;
    assert_eq!(s, StatusCode::OK);
    f.clock.set(t(45));
    f.reverse_ok(&[costs], "Tanken war privat").await;
    let b = &f.budgets().await["budgets"][0];
    assert_eq!((n(&b["consumed"]), n(&b["rest"])), (400, 49_600));
}

#[tokio::test]
async fn firmenkilometer_und_lohn_ohne_stornierte_auftraege() {
    let f = Firma::new().await;
    // Montag: zwei Aufträge, einer storniert. Dienstag: nur ein stornierter.
    f.at(16, 12, 0);
    f.event(&f.fahrer, "a", "job_delivered", 1000, Some(500.0))
        .await;
    f.event(&f.fahrer, "b", "job_delivered", 1000, Some(300.0))
        .await;
    f.at(17, 12, 0);
    f.event(&f.fahrer, "c", "job_delivered", 1000, Some(200.0))
        .await;
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 1000);
    let ids = [f.entry_id("b").await, f.entry_id("c").await];
    f.reverse_ok(&ids, "privates Profil").await;
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 500);

    // Mittwoch 4:00: Montag und Dienstag werden abgerechnet
    f.at(18, 4, 0);
    assert_eq!(f.run_due(), 2);
    let runs = f.payrolls().await;
    let monday = runs.iter().find(|r| r["day"] == "2026-11-16").unwrap();
    let l = line(monday, f.fahrer_id);
    assert_eq!(
        (n(&l["km"]), n(&l["flat_days"]), n(&l["wage"])),
        (500, 1, 250)
    );
    // Am Dienstag gab es nur den stornierten Auftrag: keine Zeile, keine Pauschale
    let tuesday = runs.iter().find(|r| r["day"] == "2026-11-17").unwrap();
    assert_eq!(tuesday["lines"], json!([]), "{tuesday}");
    assert_eq!(f.wallet(f.fahrer_id).await, 250);
    // Ein Storno nach der Abrechnung holt den Lohn nicht von selbst zurück
    f.reverse_ok(&[f.entry_id("a").await], "privat").await;
    assert_eq!(f.wallet(f.fahrer_id).await, 250);
    assert_eq!(f.member(f.fahrer_id).await["company_km"], 0);
}

#[tokio::test]
async fn kassenbuch_nach_zeitraum_filtern() {
    let f = Firma::new().await;
    f.at(16, 10, 0);
    f.event(&f.fahrer, "a", "tollgate", 10, None).await;
    f.at(17, 0, 0);
    f.event(&f.fahrer, "b", "tollgate", 20, None).await;
    f.at(17, 23, 59);
    f.event(&f.fahrer, "c", "tollgate", 30, None).await;
    let (s, r) = f
        .post(
            &f.chef,
            &format!("/api/companies/{}/payrolls/run", f.id),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    f.at(18, 0, 0);
    f.event(&f.fahrer, "d", "tollgate", 40, None).await;

    let range = |from: i64, to: i64| {
        format!(
            "/api/companies/{}/ledger?limit=500&from={from}&to={to}",
            f.id
        )
    };
    // Dienstag, lokaler Tag: von einschließlich Mitternacht bis ausschließlich Mitternacht
    let (s, l) = f
        .get(
            &f.fahrer,
            &range(berlin(2026, 11, 17, 0, 0), berlin(2026, 11, 18, 0, 0)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{l}");
    let ids: Vec<&str> = l["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["event_id"].as_str())
        .collect();
    assert_eq!(ids, vec!["c", "b"]);
    // Nur von, nur bis
    let (_, l) = f
        .get(
            &f.chef,
            &format!(
                "/api/companies/{}/ledger?from={}",
                f.id,
                berlin(2026, 11, 18, 0, 0)
            ),
        )
        .await;
    assert_eq!(l["entries"].as_array().unwrap().len(), 1);
    let (_, l) = f
        .get(
            &f.chef,
            &format!(
                "/api/companies/{}/ledger?to={}",
                f.id,
                berlin(2026, 11, 17, 0, 0)
            ),
        )
        .await;
    assert_eq!(l["entries"][0]["event_id"], "a");
    assert_eq!(l["entries"].as_array().unwrap().len(), 1);
    // Die Lohnabrechnungen lassen sich genauso filtern
    let (_, p) = f
        .get(
            &f.chef,
            &format!(
                "/api/companies/{}/payrolls?from={}&to={}",
                f.id,
                berlin(2026, 11, 17, 0, 0),
                berlin(2026, 11, 18, 0, 0)
            ),
        )
        .await;
    // Am Dienstagabend: der fällige Montag und die Abrechnung von Hand
    let kinds: Vec<&str> = p["payrolls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["hand", "taeglich"]);
    let (_, p) = f
        .get(
            &f.chef,
            &format!(
                "/api/companies/{}/payrolls?from={}",
                f.id,
                berlin(2026, 11, 18, 0, 0)
            ),
        )
        .await;
    assert_eq!(p["payrolls"], json!([]));
}

#[tokio::test]
async fn alte_datenbank_bekommt_storno() {
    let path = std::env::temp_dir().join(format!("spedhub-storno-{}.db", std::process::id()));
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
             CREATE TABLE sync_points (id INTEGER PRIMARY KEY, company_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL, ingame_money INTEGER NOT NULL, xp INTEGER,
                expected INTEGER, write_money INTEGER NOT NULL, last_ledger_id INTEGER NOT NULL,
                written INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
             INSERT INTO users VALUES (1, 'Alt', 'x', 1000);
             INSERT INTO companies VALUES (1, 'Altfracht', 'ABCD1234', 30, 1000);
             INSERT INTO memberships VALUES (1, 1, 'chef', 1000);
             INSERT INTO ledger VALUES (1, 1, 'firma', 1, 'maut', -50, 'toll-1', '', 2000);
             INSERT INTO sync_points VALUES (1, 1, 1, 2000, NULL, NULL, 2000, 1, 1, 2000);",
        )
        .unwrap();
    }
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    {
        let conn = db.lock().unwrap();
        // Neue Spalten leer oder 0, die alte Buchung unverändert, der Index ist da
        let (amount, rev, by): (i64, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT amount, reverses_id, booked_by FROM ledger WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((amount, rev, by), (-50, None, None));
        let waived: i64 = conn
            .query_row("SELECT costs_waived FROM sync_points", [], |r| r.get(0))
            .unwrap();
        assert_eq!(waived, 0);
        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'ledger_reverses_once'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
    }
    // Zweites Öffnen geht auch
    drop(db);
    let db = spedhub_server::db::open(path.to_str().unwrap()).unwrap();
    let f = Firma::with_db(db).await;
    // Eine Buchung, wie sie vor dem Storno geschrieben wurde, lässt sich stornieren
    let old = {
        let conn = f.db.lock().unwrap();
        conn.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, event_id, created_at)
             VALUES (?1, 'firma', ?2, 'maut', -30, 'alt-1', 2000)",
            [f.id, f.fahrer_id],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let r = f.reverse_ok(&[old], "alte Buchung").await;
    assert_eq!(n(&r["balance"]), 0);
    // Der Index hält auch ohne die Prüfung im Server: ein zweites Storno derselben Buchung
    // auf demselben Konto scheitert
    {
        let conn = f.db.lock().unwrap();
        let err = conn.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, created_at, reverses_id)
             VALUES (?1, 'firma', ?2, 'storno', 30, 3000, ?3)",
            [f.id, f.fahrer_id, old],
        );
        assert!(err.is_err());
    }
    drop(f);
    let _ = std::fs::remove_file(&path);
}

// ------------------------------------------------------------ Kennzeichen und Modell

impl Firma {
    /// Lkw mit eigenem Modell anlegen (als Chef), liefert die ID
    async fn truck_model(&self, plate: &str, model: &str) -> i64 {
        let (s, t) = self
            .post(
                &self.chef,
                &format!("/api/companies/{}/trucks", self.id),
                json!({ "plate": plate, "model": model, "odometer_km": 1000, "state": full_state(1000) }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{t}");
        n(&t["id"])
    }

    async fn suggestion(&self, token: &str) -> (StatusCode, Value) {
        self.get(
            token,
            &format!("/api/companies/{}/plate-suggestion", self.id),
        )
        .await
    }

    async fn suggested(&self) -> String {
        let (s, v) = self.suggestion(&self.chef).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["plate"].as_str().unwrap().to_string()
    }

    async fn add(&self, body: Value) -> (StatusCode, Value) {
        self.post(
            &self.chef,
            &format!("/api/companies/{}/trucks", self.id),
            body,
        )
        .await
    }

    async fn set_plate(&self, token: &str, truck: i64, plate: &str) -> (StatusCode, Value) {
        call(
            &self.app,
            "PATCH",
            &format!("/api/trucks/{truck}"),
            Some(token),
            Some(json!({ "plate": plate })),
        )
        .await
    }

    /// Wen der Chef als fremd gefahren sieht: (id, Kennzeichen) oder nichts
    async fn foreign(&self) -> Option<(i64, String)> {
        let m = self.member_as(&self.chef, self.fahrer_id).await;
        m.get("foreign_truck")
            .map(|t| (n(&t["id"]), t["plate"].as_str().unwrap().to_string()))
    }
}

#[tokio::test]
async fn zwei_firmen_lkw_mit_gleichem_kennzeichen_und_anderem_modell() {
    let f = Firma::new().await;
    // Scania A fuhr als "SPED 001", das Spiel hat es ersetzt, die Rückgabe brachte "EF XU 577"
    let a = f.truck("SPED 001", 1000, full_state(1000)).await;
    f.checkout(&f.fahrer, a).await;
    let (s, _) = f
        .checkin(
            &f.fahrer,
            a,
            json!({ "odometer_km": 1100, "plate": "EF XU 577" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    // Volvo B trägt "SPED 001" jetzt als aktuelles Kennzeichen
    let b = f.truck_model("SPED 001", "volvo.fh16_2012").await;
    let volvo = json!({ "truck_brand": "volvo", "truck_model": "fh16_2012" });
    let with = |plate: &str, model: &Value| {
        let mut body = model.clone();
        body["plate"] = json!(plate);
        body["speed_kmh"] = json!(70);
        body
    };

    // Mit dem Modell des Volvo: B, als fremd gefahren markiert (niemand hat ihn geliehen)
    let (s, r) = f.beat(&f.fahrer, with("SPED 001", &volvo)).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(f.foreign().await, Some((b, "SPED 001".into())));
    // Mit dem Modell des Scania und demselben Text, auch mit Markup und Land: A über sein
    // altes Kennzeichen, obwohl das aktuelle von B gleich lautet
    f.beat(
        &f.fahrer,
        json!({ "plate": "<offset hshift=4>sped<img src=/x.mat> 001|_germany", "speed_kmh": 70 }),
    )
    .await;
    assert_eq!(f.foreign().await, Some((a, "EF XU 577".into())));
    assert_eq!(f.truck_as(&f.chef, b).await["foreign_drivers"], json!([]));
    assert_eq!(
        f.truck_as(&f.chef, a).await["foreign_drivers"][0]["user_id"],
        f.fahrer_id
    );

    // Der Fahrer leiht A: Herzschlag mit Scania wird letzter Zustand von A
    f.checkout(&f.fahrer, a).await;
    f.clock.advance(10);
    let seen = f.clock.now();
    let (_, r) = f
        .beat(
            &f.fahrer,
            json!({ "plate": "SPED 001", "odometer_km": 1150.0, "speed_kmh": 70 }),
        )
        .await;
    assert_eq!(n(&r["leased_truck_id"]), a);
    assert_eq!(f.truck_row(a).2, Some(seen));
    assert_eq!(f.truck_row(a).3, Some(1150));
    assert!(f.foreign().await.is_none());
    assert_eq!(f.truck_as(&f.chef, a).await["driving"], true);

    // Mit dem Volvo, obwohl er A geliehen hat: B ist gemeint, A bleibt unberührt
    f.clock.advance(10);
    let (_, r) = f
        .beat(
            &f.fahrer,
            json!({ "plate": "SPED 001", "odometer_km": 9999.0, "truck_brand": "volvo",
                    "truck_model": "volvo.fh16_2012" }),
        )
        .await;
    assert!(r["leased_truck_id"].is_null(), "{r}");
    assert_eq!(f.truck_row(a).2, Some(seen), "kein neuer letzter Zustand");
    assert_eq!(f.truck_row(a).3, Some(1150));
    assert_eq!(f.truck_row(b).2, None);
    assert_eq!(f.foreign().await, Some((b, "SPED 001".into())));
    assert_eq!(f.truck_as(&f.chef, a).await["driving"], false);
}

#[tokio::test]
async fn herzschlag_mit_fremdem_modell_ist_kein_firmen_lkw() {
    let f = Firma::new().await;
    // Der Fall aus dem Spiel: Der eigene Lkw des Fahrers hat dasselbe Kennzeichen wie ein
    // Firmen-Lkw, ist aber ein anderes Modell
    let t = f.truck("SPED 001", 1000, full_state(1000)).await;
    let frei = f.truck("SPED 002", 1000, full_state(1000)).await;
    f.checkout(&f.fahrer, t).await;
    let daf = |plate: &str| {
        json!({ "plate": plate, "odometer_km": 5000.0, "speed_kmh": 80,
                "truck_brand": "daf", "truck_model": "xf_euro6",
                "wear": { "engine": 0.5 } })
    };

    let (s, r) = f.beat(&f.fahrer, daf("SPED 001")).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["recorded"], true);
    assert!(r["leased_truck_id"].is_null(), "kein Firmen-Lkw");
    // Kein letzter Zustand, keine Fahrt im Fuhrpark, keine Markierung
    assert_eq!(f.truck_row(t).2, None);
    assert_eq!(f.truck_row(t).4, None);
    let l = f.truck_as(&f.chef, t).await;
    assert_eq!(l["driving"], false);
    assert_eq!(l["foreign_drivers"], json!([]));
    assert!(l["last_known"].is_null());
    // Online ist er trotzdem, mit Kennzeichen und Modell, aber ohne Firmen-Lkw
    let m = f.member_as(&f.chef, f.fahrer_id).await;
    assert_eq!(m["online"], true);
    assert_eq!(m["plate"], "SPED 001");
    assert_eq!(m["model"], "daf.xf_euro6");
    assert!(m["truck_id"].is_null());
    assert!(m.get("foreign_truck").is_none(), "{m}");

    // Ein nicht geliehener Firmen-Lkw mit fremdem Modell: auch keine Markierung
    f.beat(&f.fahrer, daf("SPED 002")).await;
    assert!(f.foreign().await.is_none());
    assert_eq!(
        f.truck_as(&f.chef, frei).await["foreign_drivers"],
        json!([])
    );

    // Ohne letzten Zustand nimmt der Chef nicht ohne Kilometerstand zurück
    let (s, _) = f.checkin(&f.chef, t, json!({})).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Mit dem richtigen Modell passt es
    let (_, r) = f.beat(&f.fahrer, json!({ "plate": "SPED 001" })).await;
    assert_eq!(n(&r["leased_truck_id"]), t);
}

#[tokio::test]
async fn herzschlag_ohne_modell_gehoert_zu_keinem_lkw() {
    let f = Firma::new().await;
    let t = f.truck("SPED 001", 1000, full_state(1000)).await;
    let frei = f.truck("SPED 002", 1000, full_state(1000)).await;
    f.checkout(&f.fahrer, t).await;
    // Ein älteres Plugin schickt kein Modell, oder nur die Marke
    for body in [
        json!({ "plate": "SPED 001", "odometer_km": 1200.0 }),
        json!({ "plate": "SPED 001", "odometer_km": 1200.0, "truck_brand": "scania" }),
        json!({ "plate": "SPED 001", "odometer_km": 1200.0, "truck_model": "  " }),
    ] {
        let (s, r) = f.beat_raw(&f.fahrer, body.clone()).await;
        assert_eq!(s, StatusCode::OK, "{body} {r}");
        assert_eq!(r["recorded"], true);
        assert!(r["leased_truck_id"].is_null(), "{body}");
        assert_eq!(f.truck_row(t).2, None, "{body}");
        let m = f.member_as(&f.chef, f.fahrer_id).await;
        assert_eq!(m["online"], true);
        assert!(m["model"].is_null() && m["truck_id"].is_null(), "{m}");
        assert!(m.get("foreign_truck").is_none(), "{m}");
    }
    f.beat_raw(&f.fahrer, json!({ "plate": "SPED 002" })).await;
    assert!(f.foreign().await.is_none());
    assert_eq!(
        f.truck_as(&f.chef, frei).await["foreign_drivers"],
        json!([])
    );
    assert_eq!(f.truck_as(&f.chef, t).await["driving"], false);
    // Zu lange Angaben sind kein Fehler, sie werden gekürzt wie andere Texte
    let (s, _) = f
        .beat_raw(
            &f.fahrer,
            json!({ "plate": "SPED 001", "truck_model": "x".repeat(100) }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn kennzeichen_vorschlag_aus_kuerzel_und_naechster_nummer() {
    // Die Firma heißt "Nordfracht": Kürzel NOR
    let f = Firma::new().await;
    let (s, v) = f.suggestion(&f.chef).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!({ "plate": "NOR 001", "prefix": "NOR" }));
    // Nur der Chef legt Lkw an und bekommt Vorschläge
    let (s, _) = f.suggestion(&f.fahrer).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Anlegen ohne Kennzeichen: der Vorschlag
    let (s, t) = f.add(json!({ "model": "scania.s_2016" })).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(
        (t["plate"].clone(), t["plate_suggested"].clone()),
        (json!("NOR 001"), json!(true))
    );
    let (_, t2) = f
        .add(json!({ "plate": "", "model": "scania.s_2016" }))
        .await;
    assert_eq!(t2["plate"], "NOR 002");
    assert_eq!(f.suggested().await, "NOR 003");

    // Vergebene Nummern werden übersprungen, auch von Hand vergebene
    f.add(json!({ "plate": "nor 003", "model": "scania.s_2016" }))
        .await;
    f.add(json!({ "plate": "NOR 005", "model": "scania.s_2016" }))
        .await;
    assert_eq!(f.suggested().await, "NOR 004");

    // Ein früheres Kennzeichen zählt als vergeben: NOR 004 bekommt bei der Rückgabe ein neues
    let (_, t4) = f.add(json!({ "model": "scania.s_2016" })).await;
    assert_eq!(t4["plate"], "NOR 004");
    let t4 = n(&t4["id"]);
    f.checkout(&f.fahrer, t4).await;
    f.checkin(
        &f.fahrer,
        t4,
        json!({ "odometer_km": 10, "plate": "EF XU 577" }),
    )
    .await;
    assert_eq!(f.suggested().await, "NOR 006");

    // Auch Kennzeichen ausgemusterter und verkaufter Lkw
    let (_, t6) = f.add(json!({ "model": "scania.s_2016" })).await;
    assert_eq!(t6["plate"], "NOR 006");
    let (s, _) = f.retire(&f.chef, n(&t6["id"])).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(f.suggested().await, "NOR 007");
    let (_, t7) = f.add(json!({ "model": "volvo.fh16_2012" })).await;
    f.sell(&f.chef, n(&t7["id"]), 0).await;
    assert_eq!(f.suggested().await, "NOR 008");
    // Der Vorschlag ist ein gültiges Kennzeichen zum Einsetzen
    let plate = f.suggested().await;
    assert_eq!(
        spedhub_save::plate_text(&spedhub_save::custom_plate(&plate, "germany").unwrap()),
        plate
    );
}

#[tokio::test]
async fn kennzeichen_beim_anlegen_pruefen() {
    let f = Firma::new().await;
    // Nur, was sich als eigenes Kennzeichen in den Spielstand schreiben lässt
    for bad in [
        "MÜNCHEN 1",
        "SPED 001|_germany",
        "A_B 1",
        "<img src=/a.mat>AB",
        "ABCDEFGHIJKLMNOPQ",
    ] {
        let (s, v) = f
            .add(json!({ "plate": bad, "model": "scania.s_2016" }))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
        assert!(v["error"].as_str().unwrap().contains("Kennzeichen"), "{v}");
    }
    let (s, v) = f.add(json!({ "plate": "SPED 001", "model": " " })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], "Das Modell darf nicht leer sein");
    assert_eq!(f.ledger().await.len(), 0);
    let (_, l) = f
        .get(&f.chef, &format!("/api/companies/{}/trucks", f.id))
        .await;
    assert_eq!(l["trucks"], json!([]), "nichts angelegt");

    // Klein geschrieben und mit doppeltem Leerraum: so, wie es im Spiel steht
    let (s, t) = f
        .add(json!({ "plate": " sped  1-a. ", "model": "scania.s_2016" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["plate"], "SPED 1-A.");
    assert_eq!(t["plate_suggested"], false);
    let (s, t) = f
        .add(json!({ "plate": "ABCDEFGHIJKLMNOP", "model": "scania.s_2016" }))
        .await;
    assert_eq!(s, StatusCode::OK, "16 Zeichen gehen: {t}");
    // Eindeutig je Firma, auch in anderer Schreibweise
    let (s, v) = f
        .add(json!({ "plate": "Sped 1-a.", "model": "volvo.fh16_2012" }))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "Das Kennzeichen gibt es in der Firma schon");
}

#[tokio::test]
async fn chef_aendert_das_kennzeichen() {
    let f = Firma::new().await;
    let a = f.truck("SPED 001", 1000, full_state(1000)).await;
    let b = f.truck("SPED 002", 1000, full_state(1000)).await;

    // Nur der Chef, nur mit gültigem Kennzeichen
    let (s, _) = f.set_plate(&f.fahrer, a, "NOR 100").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = f.set_plate(&f.chef, a, "NÖR 1").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = f.set_plate(&f.chef, a, " ").await;
    assert_eq!(
        (s, v["error"].clone()),
        (StatusCode::BAD_REQUEST, json!("Das Kennzeichen fehlt"))
    );
    let (s, _) = f.set_plate(&f.chef, 999, "NOR 100").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Das Kennzeichen eines anderen aktiven Lkw: 409
    let (s, v) = f.set_plate(&f.chef, a, "sped 002").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "Das Kennzeichen SPED 002 hat in der Firma schon ein anderer Lkw"
    );

    // Während einer Leihe nicht: Im Spielstand des Fahrers steht noch das alte
    f.checkout(&f.fahrer, a).await;
    let (s, v) = f.set_plate(&f.chef, a, "NOR 100").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "Der Lkw ist gerade an Fahrer verliehen. Das Kennzeichen lässt sich erst nach der Rückgabe ändern"
    );
    f.checkin(&f.fahrer, a, json!({ "odometer_km": 1100 }))
        .await;

    // Geändert: das alte kommt zu den früheren
    let (s, v) = f.set_plate(&f.chef, a, "nor 100").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v,
        json!({ "id": a, "plate": "NOR 100", "old_plates": ["SPED 001"], "changed": true })
    );
    let l = f.truck_as(&f.fahrer, a).await;
    assert_eq!(l["plate"], "NOR 100");
    assert_eq!(l["old_plates"], json!(["SPED 001"]));
    // Dasselbe noch einmal ändert nichts
    let (_, v) = f.set_plate(&f.chef, a, "NOR 100").await;
    assert_eq!(v["changed"], false);
    assert_eq!(v["old_plates"], json!(["SPED 001"]));
    // Beim Ausleihen kommt das neue mit, das der Launcher in den Spielstand schreibt
    let (_, out) = f
        .post(&f.fahrer, &format!("/api/trucks/{a}/checkout"), json!({}))
        .await;
    assert_eq!(out["truck"]["plate"], "NOR 100");
    // Ein Herzschlag mit dem alten Kennzeichen gehört weiter zu A
    let (_, r) = f.beat(&f.fahrer, json!({ "plate": "SPED 001" })).await;
    assert_eq!(n(&r["leased_truck_id"]), a);
    let (_, r) = f.beat(&f.fahrer, json!({ "plate": "NOR 100" })).await;
    assert_eq!(n(&r["leased_truck_id"]), a);
    f.checkin(&f.fahrer, a, json!({ "odometer_km": 1200 }))
        .await;
    // Das frei gewordene Kennzeichen darf B bekommen
    let (s, v) = f.set_plate(&f.chef, b, "SPED 001").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["old_plates"], json!(["SPED 002"]));

    // Das Kennzeichen eines ausgemusterten Lkw wandert, ausgemusterte ändern es nicht mehr
    f.retire(&f.chef, b).await;
    let (s, v) = f.set_plate(&f.chef, b, "NOR 200").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        v["error"],
        "Der Lkw ist ausgemustert und bekommt kein neues Kennzeichen"
    );
    let (s, _) = f.set_plate(&f.chef, a, "SPED 001").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        f.truck_as(&f.chef, b).await["plate"],
        format!("SPED 001 (Lkw {b})")
    );
    assert_eq!(
        f.truck_as(&f.chef, a).await["old_plates"],
        json!(["SPED 001", "NOR 100"])
    );
}

#[tokio::test]
async fn ereignis_speichert_marke_und_modell() {
    let f = Firma::new().await;
    let (s, r) = f
        .post(
            &f.fahrer,
            &format!("/api/companies/{}/events", f.id),
            json!({ "event_id": "job-1", "type": "job_delivered", "amount": 1000, "distance_km": 300,
                    "truck_brand": " scania ", "truck_model": "vehicle.scania.s_2016" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (s, _) = f
        .post(
            &f.fahrer,
            &format!("/api/companies/{}/events", f.id),
            json!({ "event_id": "maut-1", "type": "tollgate", "amount": 10,
                    "truck_brand": "volvo", "truck_model": "x".repeat(100) }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    f.event(&f.fahrer, "alt-1", "fined", 50, None).await;
    let l = f.ledger().await;
    let by = |id: &str| l.iter().find(|e| e["event_id"] == id).unwrap().clone();
    let job = by("job-1");
    assert_eq!(
        (job["truck_brand"].clone(), job["truck_model"].clone()),
        (json!("scania"), json!("vehicle.scania.s_2016"))
    );
    // Bei jeder Art, gekürzt wie andere Texte
    assert_eq!(by("maut-1")["truck_brand"], "volvo");
    assert_eq!(by("maut-1")["truck_model"], "x".repeat(64));
    // Ohne Angabe, etwa vom älteren Plugin oder aus dem Dashboard: leer
    let old = by("alt-1");
    assert!(
        old["truck_brand"].is_null() && old["truck_model"].is_null(),
        "{old}"
    );
    // Andere Buchungen haben keins
    assert!(
        l.iter()
            .filter(|e| e["event_id"].is_null())
            .all(|e| e["truck_model"].is_null())
    );

    // Ein abgelehntes Ereignis behält beides in seinem Inhalt
    call(
        &f.app,
        "PUT",
        &format!("/api/companies/{}/profile", f.id),
        Some(&f.fahrer),
        Some(json!({ "profile_name": "Nordfracht" })),
    )
    .await;
    let (s, _) = f
        .post(
            &f.fahrer,
            &format!("/api/companies/{}/events", f.id),
            json!({ "event_id": "privat-1", "type": "fined", "amount": 10, "profile": "Privat",
                    "truck_brand": "daf", "truck_model": "xf_euro6" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, r) = f
        .get(&f.chef, &format!("/api/companies/{}/rejected-events", f.id))
        .await;
    assert_eq!(r["entries"][0]["payload"]["truck_model"], "xf_euro6");
}
