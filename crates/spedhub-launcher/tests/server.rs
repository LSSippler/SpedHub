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
    let back = read_for_return(&docs.nordfracht, "SPED 001")
        .unwrap()
        .unwrap();
    assert_eq!(back.odometer_km, 4200);
    let r = fahrer
        .checkin(truck_id, back.odometer_km, Some(&back.state))
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
    let e = chef.checkin(truck_id, 100, None).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Server: Der Kilometerstand kann nicht kleiner werden"
    );
    chef.checkin(truck_id, 4300, None).unwrap();
    let _ = fs::remove_dir_all(dir);
}
