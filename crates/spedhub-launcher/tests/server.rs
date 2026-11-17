//! Der Client gegen einen echten spedhub-server, dazu der ganze Ablauf: Lkw übernehmen,
//! ausleihen, in den Spielstand schreiben, fahren, zurückgeben.
mod common;

use common::*;
use serde_json::json;
use spedhub_launcher::Error;
use spedhub_launcher::client::Client;
use spedhub_launcher::savegame::{
    deploy_truck, new_truck_from_save, package_from_checkout, read_for_return,
};
use spedhub_save::{Save, set_truck_state, truck_state, write_save};
use std::{fs, time::SystemTime};

#[test]
fn anmelden_und_fehler_im_klartext() {
    let dir = temp_dir("anmelden");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();

    let mut c = Client::new(&url);
    let login = c.register("Alex", "geheim1234").unwrap();
    assert_eq!(login.name, "Alex");
    assert_eq!(c.token(), Some(login.token.as_str()));

    // Fehler kommen mit der Meldung des Servers
    let e = Client::new(&url).register("Al", "geheim1234").unwrap_err();
    assert_eq!(e.to_string(), "Server: Der Name braucht 3 bis 32 Zeichen");
    let e = Client::new(&url)
        .register("alex", "geheim1234")
        .unwrap_err();
    assert_eq!(e.to_string(), "Server: Der Name ist schon vergeben");
    let e = Client::new(&url).login("Alex", "falsch123").unwrap_err();
    assert_eq!(e.to_string(), "Server: Name oder Passwort stimmt nicht");
    assert!(e.is_unauthorized());

    let mut c = Client::new(&format!("{url}/"));
    c.login("alex", "geheim1234").unwrap();
    let me = c.me().unwrap();
    assert_eq!(me.name, "Alex");
    assert!(me.companies.is_empty());

    // Abgemeldet oder falscher Token
    let token = c.token().unwrap().to_string();
    c.logout().unwrap();
    let e = Client::with_token(&url, &token).me().unwrap_err();
    assert!(e.is_unauthorized(), "{e:?}");

    // Server nicht da, Adresse kaputt
    let e = Client::new("http://127.0.0.1:1").me().unwrap_err();
    assert!(matches!(e, Error::Network(_)), "{e:?}");
    assert!(e.to_string().starts_with("Server nicht erreichbar"));
    let e = Client::new("spedhub.example").me().unwrap_err();
    assert!(e.to_string().contains("http://"), "{e}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn uebernehmen_ausleihen_einsetzen_zurueckgeben() {
    let dir = temp_dir("ablauf");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let docs = documents("ablauf-docs");
    let backups = dir.join("backups");

    // Chef gründet die Firma, Fahrer tritt bei
    let mut chef = Client::new(&url);
    let chef_login = chef.register("Chefin", "geheim1234").unwrap();
    let firma = post_raw(
        &format!("{url}/api/companies"),
        &chef_login.token,
        json!({ "name": "Nordfracht" }),
    );
    let company = firma["id"].as_i64().unwrap();
    let mut fahrer = Client::new(&url);
    let fahrer_login = fahrer.register("Fahrer", "geheim1234").unwrap();
    post_raw(
        &format!("{url}/api/companies/join"),
        &fahrer_login.token,
        json!({ "invite_code": firma["invite_code"] }),
    );

    let me = fahrer.me().unwrap();
    assert_eq!(me.companies.len(), 1);
    assert_eq!(me.companies[0].role, "fahrer");
    assert!(chef.company(company).unwrap().is_chef());
    assert!(!fahrer.company(company).unwrap().is_chef());

    // 1. Chef übernimmt den Scania aus seinem Spielstand in den Fuhrpark
    let quelle = docs.nordfracht.join("save").join("1").join("game.sii");
    let new = new_truck_from_save(&quelle, "SPED 001").unwrap();
    let truck_id = chef.add_truck(company, &new).unwrap();
    let e = fahrer.add_truck(company, &new).unwrap_err();
    assert_eq!(e.to_string(), "Server: Das darf nur der Chef");
    let e = chef.add_truck(company, &new).unwrap_err();
    assert!(
        e.to_string()
            .contains("Kennzeichen gibt es in der Firma schon")
    );

    let trucks = fahrer.trucks(company).unwrap();
    assert_eq!(trucks.len(), 1);
    assert_eq!(
        (
            trucks[0].plate.as_str(),
            trucks[0].model.as_str(),
            trucks[0].odometer_km
        ),
        ("SPED 001", "scania.s_2016", 3865)
    );
    assert!(trucks[0].leased_by.is_none());

    // 2. Fahrer leiht ihn aus, der Chef bekommt ihn danach nicht
    let checkout = fahrer.checkout(truck_id).unwrap();
    let e = chef.checkout(truck_id).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Server: Der Lkw ist gerade an Fahrer verliehen"
    );
    let lease = fahrer.trucks(company).unwrap()[0]
        .leased_by
        .clone()
        .unwrap();
    assert_eq!(lease.user_id, me.user_id);

    // 3. In den neuesten Spielstand des Firmenprofils schreiben
    let package = package_from_checkout(&checkout.truck).unwrap();
    let deployed = deploy_truck(
        &docs.nordfracht,
        &package,
        &backups,
        &FakeGame::closed(),
        SystemTime::now(),
    )
    .unwrap();

    // 4. Im Spiel fahren: 335 km und etwas Verschleiß, wie es das Spiel speichern würde
    let game_path = deployed.slot_dir.join("game.sii");
    let (mut save, _) = Save::read(&game_path).unwrap();
    let mut state = truck_state(&save, Some("SPED 001")).unwrap();
    state.odometer = 4200;
    state.engine_wear = 0.05;
    set_truck_state(&mut save, Some("SPED 001"), &state).unwrap();
    write_save(&game_path, &save).unwrap();

    // 5. Zurückgeben mit den Werten aus dem Spielstand
    let back = read_for_return(&docs.nordfracht, "SPED 001", None)
        .unwrap()
        .unwrap();
    assert_eq!(back.odometer_km, 4200);
    let r = fahrer
        .checkin(truck_id, back.odometer_km, Some(&back.state), None)
        .unwrap();
    assert_eq!((r.driven_km, r.rev), (335, 2));

    let trucks = chef.trucks(company).unwrap();
    assert!(trucks[0].leased_by.is_none());
    assert_eq!(trucks[0].odometer_km, 4200);

    // Beim nächsten Ausleihen kommt der neue Zustand mit
    let again = chef.checkout(truck_id).unwrap();
    let p = package_from_checkout(&again.truck).unwrap();
    assert_eq!(p.state.odometer, 4200);
    assert_eq!(p.state.engine_wear, 0.05);

    // Von Hand mit kleinerem Kilometerstand: der Server lehnt ab
    let e = chef.checkin(truck_id, 100, None, None).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Server: Der Kilometerstand kann nicht kleiner werden"
    );
    chef.checkin(truck_id, 4300, None, None).unwrap();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rueckgabe_mit_neuem_kennzeichen_vom_spiel() {
    let dir = temp_dir("kennzeichen");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let docs = documents("kennzeichen-docs");
    let app = spedhub_launcher::settings::AppPaths::new(dir.join("SpedHub"));

    let mut chef = Client::new(&url);
    let login = chef.register("Chefin", "geheim1234").unwrap();
    let firma = post_raw(
        &format!("{url}/api/companies"),
        &login.token,
        json!({ "name": "Nordfracht" }),
    );
    let company = firma["id"].as_i64().unwrap();
    let quelle = docs.nordfracht.join("save").join("1").join("game.sii");
    let truck_id = chef
        .add_truck(company, &new_truck_from_save(&quelle, "SPED 001").unwrap())
        .unwrap();

    // Ausleihen und einsetzen, so wie die Oberfläche es tut: Paket ablegen, dann schreiben
    let checkout = chef.checkout(truck_id).unwrap();
    let package = package_from_checkout(&checkout.truck).unwrap();
    app.store_lease(truck_id, &package).unwrap();
    let deployed = deploy_truck(
        &docs.nordfracht,
        &package,
        &app.backups(),
        &FakeGame::closed(),
        SystemTime::now(),
    )
    .unwrap();

    // Das Spiel speichert: neue Namen, neues Kennzeichen, 425 km gefahren
    fs::copy(testdata(NACH_SPIEL), deployed.slot_dir.join("game.sii")).unwrap();

    // Rückgabe: über das Kennzeichen nichts, über die Teile der Lkw mit neuem Kennzeichen
    let lease = app.load_lease(truck_id).unwrap();
    let back = read_for_return(&docs.nordfracht, &checkout.truck.plate, Some(&lease))
        .unwrap()
        .unwrap();
    assert_eq!(back.new_plate.as_deref(), Some("EF XU 577"));
    let r = chef
        .checkin(
            truck_id,
            back.odometer_km,
            Some(&back.state),
            back.new_plate.as_deref(),
        )
        .unwrap();
    assert_eq!((r.plate.as_str(), r.driven_km), ("EF XU 577", 425));
    app.remove_lease(truck_id);

    // Der Server kennt jetzt das neue Kennzeichen und hat das alte behalten
    let t = chef.trucks(company).unwrap().remove(0);
    assert_eq!(t.plate, "EF XU 577");
    assert_eq!(t.old_plates, vec!["SPED 001".to_string()]);
    assert_eq!(t.odometer_km, 4290);
    assert!(t.leased_by.is_none());

    // Nächste Leihe: Der Lkw steht noch im Spielstand, ein zweites Einsetzen wird verhindert
    let again = chef.checkout(truck_id).unwrap();
    assert_eq!(again.truck.plate, "EF XU 577");
    let p = package_from_checkout(&again.truck).unwrap();
    let e = deploy_truck(
        &docs.nordfracht,
        &p,
        &app.backups(),
        &FakeGame::closed(),
        SystemTime::now(),
    )
    .unwrap_err();
    assert!(
        e.to_string().contains("steht schon im neuesten Spielstand"),
        "{e}"
    );

    // Diesmal findet die Rückgabe ihn direkt über das Kennzeichen
    let back = read_for_return(&docs.nordfracht, &again.truck.plate, Some(&p))
        .unwrap()
        .unwrap();
    assert_eq!(back.new_plate, None);
    let r = chef
        .checkin(truck_id, back.odometer_km, Some(&back.state), None)
        .unwrap();
    assert_eq!(r.plate, "EF XU 577");
    let _ = fs::remove_dir_all(dir);
}
