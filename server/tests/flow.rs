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

    // Auftrag: 30 % Lohn, 70 % Kasse. Doppelt geschickt wird nur einmal gebucht.
    let job = json!({ "event_id": "job-1", "type": "job_delivered", "amount": 1000 });
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
        (Some(300), Some(700))
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
        (Some(300), Some(700))
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
    assert_eq!(r["wallet"], 200);
    let toll = json!({ "event_id": "toll-1", "type": "tollgate", "amount": 50 });
    let (_, r) = call(
        &app,
        "POST",
        &format!("/api/companies/{id}/events"),
        Some(&fahrer),
        Some(toll),
    )
    .await;
    assert_eq!(r["balance"], 650);
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
    assert_eq!(c["balance"], 650);
    assert_eq!(c["members"].as_array().unwrap().len(), 2);
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
    assert_eq!(l["entries"].as_array().unwrap().len(), 4);

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
        Some(geld(651)),
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
        (Some(500), Some(350))
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
        (Some(900), Some(-50))
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
