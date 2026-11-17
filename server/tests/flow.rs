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
    assert!(std::str::from_utf8(&body).unwrap().contains("Kassenbuch"));
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
        let mut body = json!({ "event_id": event_id, "type": kind, "amount": amount });
        if let Some(km) = km {
            body["distance_km"] = json!(km);
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
        let mut body = json!({ "ingame_money": money });
        if let Some(xp) = xp {
            body["xp"] = json!(xp);
        }
        let (s, r) = self
            .post(token, &format!("/api/companies/{}/sync", self.id), body)
            .await;
        assert_eq!(s, StatusCode::OK, "{r}");
        r
    }

    async fn written(&self, token: &str, sync: &Value) -> (StatusCode, Value) {
        self.post(
            token,
            &format!(
                "/api/companies/{}/sync/{}/written",
                self.id, sync["sync_id"]
            ),
            json!({}),
        )
        .await
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

    // Im Spiel: Auftrag +1500, Strafe -100, Maut -30, Abbruch -70, und 200 € fürs Tanken
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
