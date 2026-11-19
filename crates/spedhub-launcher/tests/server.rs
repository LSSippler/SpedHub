//! Der Client gegen einen echten spedhub-server, dazu der ganze Ablauf: Lkw übernehmen,
//! ausleihen, in den Spielstand schreiben, fahren, zurückgeben.
mod common;

use common::*;
use serde_json::json;
use spedhub_launcher::Error;
use spedhub_launcher::client::{Client, TruckState};
use spedhub_launcher::flows::{
    PackageSource, describe, lease_package, return_truck, sync_and_write,
};
use spedhub_launcher::savegame::{
    SYNC_SLOT_NAME, deploy_truck, new_truck_from_save, package_from_checkout, read_for_return,
};
use spedhub_launcher::settings::AppPaths;
use spedhub_save::{
    Save, info_money, info_name, list_trucks, money, set_license_plate, set_money, set_truck_state,
    truck_state, write_save,
};
use std::{
    fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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
    let new = new_truck_from_save(&quelle, "SPED 001", "SPED 001").unwrap();
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
        .add_truck(
            company,
            &new_truck_from_save(&quelle, "SPED 001", "SPED 001").unwrap(),
        )
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
    // Wie das Spiel die Datei neu schreiben: Unter Windows behält fs::copy die alte
    // Änderungszeit, deshalb ausdrücklich auf jetzt setzen.
    let game_sii = deployed.slot_dir.join("game.sii");
    fs::copy(testdata(NACH_SPIEL), &game_sii).unwrap();
    fs::File::options()
        .write(true)
        .open(&game_sii)
        .unwrap()
        .set_modified(SystemTime::now())
        .unwrap();

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

/// Firma mit Chefin und Fahrer auf einem echten Server.
struct Setup {
    server: TestServer,
    company: i64,
    chef: Client,
    chef_token: String,
    invite: String,
    fahrer: Client,
}

fn setup(dir: &std::path::Path) -> Setup {
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let mut chef = Client::new(&url);
    let chef_token = chef.register("Chefin", "geheim1234").unwrap().token;
    let firma = post_raw(
        &format!("{url}/api/companies"),
        &chef_token,
        json!({ "name": "Nordfracht" }),
    );
    let mut fahrer = Client::new(&url);
    let t = fahrer.register("Fahrer", "geheim1234").unwrap().token;
    post_raw(
        &format!("{url}/api/companies/join"),
        &t,
        json!({ "invite_code": firma["invite_code"] }),
    );
    Setup {
        server,
        company: firma["id"].as_i64().unwrap(),
        chef,
        chef_token,
        invite: firma["invite_code"].as_str().unwrap().to_string(),
        fahrer,
    }
}

fn slot_count(profile: &std::path::Path) -> usize {
    fs::read_dir(profile.join("save")).unwrap().count()
}

/// Wie das Spiel einen Speicherplatz neu schreibt: Kontostand ändern, Änderungszeit `at`.
fn play(slot_dir: &std::path::Path, money_now: i64, at: SystemTime) {
    let path = slot_dir.join("game.sii");
    let (mut save, _) = Save::read(&path).unwrap();
    set_money(&mut save, money_now).unwrap();
    write_save(&path, &save).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(at)
        .unwrap();
}

/// `t` als Unix-Sekunden, wie das Plugin den Zeitpunkt eines Events schickt.
fn unix(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Sekunden vor jetzt. Die Tests legen ihre Zeitpunkte in die jüngste Vergangenheit und mit
/// Abstand zueinander, damit sie nicht davon abhängen, wie schnell die Sekunde umspringt.
fn ago(start: SystemTime, secs: u64) -> SystemTime {
    start - Duration::from_secs(secs)
}

#[test]
fn geld_abgleichen_mit_neuem_speicherplatz() {
    let dir = temp_dir("abgleich");
    let s = setup(&dir);
    let url = s.server.url();
    let docs = documents("abgleich-docs");
    let backups = dir.join("backups");
    // Vorschuss 3000, damit der neue Kontostand vom alten (2000) zu unterscheiden ist
    send_raw(
        "PATCH",
        &format!("{url}/api/companies/{}", s.company),
        &s.chef_token,
        json!({ "advance": 3000 }),
    );

    // Zeitpunkte: Der Spielstand "autosave" ist 60 Sekunden alt. Der Launcher schreibt den
    // ersten Abgleich vor 50 Sekunden, das Spiel liefert den Auftrag vor 45 und speichert
    // vor 40, der zweite Abgleich schreibt vor 30 Sekunden.
    let start = SystemTime::now();

    // Erster Abgleich: nichts gebucht, neuer Speicherplatz mit 0 + 3000
    let before = slot_count(&docs.nordfracht);
    let o = sync_and_write(
        &s.fahrer,
        s.company,
        &docs.nordfracht,
        None,
        &backups,
        &FakeGame::closed(),
        ago(start, 50),
    )
    .unwrap();
    assert!(o.sync.first);
    assert!(o.confirmed);
    assert_eq!((o.sync.ingame_money, o.sync.write_money), (2000, 3000));
    assert_eq!(o.deployed.name, SYNC_SLOT_NAME);
    assert_eq!(o.deployed.source_slot, "autosave");
    assert_eq!(slot_count(&docs.nordfracht), before + 1);
    let (game, _) = Save::read(o.deployed.slot_dir.join("game.sii")).unwrap();
    let (info, _) = Save::read(o.deployed.slot_dir.join("info.sii")).unwrap();
    assert_eq!(money(&game).unwrap(), 3000);
    assert_eq!(info_money(&info).unwrap(), 3000);
    assert_eq!(info_name(&info).unwrap(), "SpedHub: Geldabgleich");
    assert!(o.deployed.backup_dir.join("game.sii").is_file());
    assert!(describe(&o.sync).contains("Erster Abgleich"));
    // Erfahrung aus info.sii ist angekommen
    let me = s.fahrer.me().unwrap();
    let m = s.fahrer.company(s.company).unwrap();
    let mine = m.members.iter().find(|x| x.user_id == me.user_id).unwrap();
    assert_eq!((mine.game_level, mine.stage), (0, Some(1)));

    // Im Spiel: Auftrag über 1500 €, 200 € fürs Tanken. Das Plugin meldet den Auftrag mit
    // dem Zeitpunkt, zu dem er passiert ist.
    play(&o.deployed.slot_dir, 3000 + 1500 - 200, ago(start, 40));
    post_raw(
        &format!("{url}/api/companies/{}/events", s.company),
        s.fahrer.token().unwrap(),
        json!({ "event_id": "job-1", "type": "job_delivered", "amount": 1500, "distance_km": 400,
                "occurred_at": unix(ago(start, 45)) }),
    );
    let o = sync_and_write(
        &s.fahrer,
        s.company,
        &docs.nordfracht,
        None,
        &backups,
        &FakeGame::closed(),
        ago(start, 30),
    )
    .unwrap();
    assert!(!o.sync.first);
    // Der Server hat die Speicherzeit des gelesenen Spielstands bekommen
    assert_eq!(o.sync.saved_at, Some(unix(ago(start, 40))));
    assert_eq!(o.sync.expected, Some(4500));
    assert_eq!(o.sync.booked.len(), 1);
    assert_eq!(
        (o.sync.booked[0].kind.as_str(), o.sync.booked[0].amount),
        ("betriebskosten", -200)
    );
    assert_eq!(o.sync.write_money, 3000);
    assert!(describe(&o.sync).contains("Betriebskosten 200 €"));
    let (game, _) = Save::read(o.deployed.slot_dir.join("game.sii")).unwrap();
    assert_eq!(money(&game).unwrap(), 3000);
    assert_eq!(s.chef.company(s.company).unwrap().balance, 1300);

    // Läuft das Spiel schon vorher, fragt der Launcher den Server gar nicht erst
    let snap = snapshot(&docs.nordfracht);
    let e = sync_and_write(
        &s.fahrer,
        s.company,
        &docs.nordfracht,
        None,
        &backups,
        &FakeGame::running(),
        SystemTime::now(),
    )
    .unwrap_err();
    assert_eq!(e, Error::GameRunning);
    // Startet es zwischen Abgleich und Schreiben: nichts geschrieben, der Abgleich bleibt
    // unbestätigt und der nächste rechnet vom gelesenen Stand
    let e = sync_and_write(
        &s.fahrer,
        s.company,
        &docs.nordfracht,
        None,
        &backups,
        &FakeGame::sequence(vec![Ok(false), Ok(true)]),
        SystemTime::now(),
    )
    .unwrap_err();
    assert_eq!(e, Error::GameRunning);
    assert_eq!(snapshot(&docs.nordfracht), snap, "nichts geschrieben");
    let o = sync_and_write(
        &s.fahrer,
        s.company,
        &docs.nordfracht,
        None,
        &backups,
        &FakeGame::closed(),
        SystemTime::now(),
    )
    .unwrap();
    assert_eq!(o.sync.expected, Some(3000));
    assert!(o.sync.booked.is_empty());
    assert!(o.sync.unexplained.is_none());
    assert!(describe(&o.sync).contains("Kontostand passt"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn spaet_gemeldeter_auftrag_nach_dem_abgleich() {
    let dir = temp_dir("spaet");
    let s = setup(&dir);
    let url = s.server.url();
    let docs = documents("spaet-docs");
    let backups = dir.join("backups");
    let start = SystemTime::now();
    let sync = |now: SystemTime| {
        sync_and_write(
            &s.fahrer,
            s.company,
            &docs.nordfracht,
            None,
            &backups,
            &FakeGame::closed(),
            now,
        )
        .unwrap()
    };

    // Erster Abgleich, geschrieben vor 50 Sekunden: 0 + 2000
    let o = sync(ago(start, 50));
    assert!(o.sync.first && o.confirmed);
    assert_eq!(o.sync.write_money, 2000);

    // Im Spiel ein Auftrag über 1000 € vor 45 Sekunden, gespeichert vor 40. Das Plugin
    // erreicht den Server erst nach dem nächsten Abgleich.
    play(&o.deployed.slot_dir, 3000, ago(start, 40));
    let o = sync(ago(start, 30));
    assert!(o.confirmed);
    assert_eq!(o.sync.expected, Some(2000));
    assert!(o.sync.booked.is_empty());
    let entry = o.sync.unexplained.clone().unwrap();
    assert_eq!(entry.amount, 1000);
    assert!(describe(&o.sync).contains("1000 € mehr im Spiel als erklärt"));

    let events = format!("{url}/api/companies/{}/events", s.company);
    let r = post_raw(
        &events,
        s.fahrer.token().unwrap(),
        json!({ "event_id": "job-1", "type": "job_delivered", "amount": 1000, "distance_km": 300,
                "occurred_at": unix(ago(start, 45)) }),
    );
    // Der Auftrag erklärt den Betrag: ungeklärt ist geschlossen, Betriebskosten gibt es keine
    assert_eq!(r["late"]["sync_id"], o.sync.sync_id);
    assert_eq!(r["late"]["booked"], json!([]));
    assert_eq!(r["late"]["unexplained"]["id"], entry.id);
    assert_eq!(r["late"]["unexplained"]["closed"], true);
    let u = send_raw(
        "GET",
        &format!("{url}/api/companies/{}/unexplained?all=true", s.company),
        &s.chef_token,
        json!({}),
    );
    assert_eq!(
        u["entries"][0]["note"],
        "Durch spät gemeldete Ereignisse erklärt"
    );
    assert!(u["entries"][0]["closed_at"].is_i64());

    // Der nächste Abgleich ohne Fahrt zählt den Auftrag nicht noch einmal
    let o = sync(SystemTime::now());
    assert_eq!(o.sync.expected, Some(2000));
    assert!(o.sync.booked.is_empty());
    assert!(o.sync.unexplained.is_none());
    assert!(describe(&o.sync).contains("Kontostand passt"));

    // Die Firma hat den ganzen Erlös, nirgends Betriebskosten
    assert_eq!(s.chef.company(s.company).unwrap().balance, 1000);
    let l = send_raw(
        "GET",
        &format!("{url}/api/companies/{}/ledger", s.company),
        &s.chef_token,
        json!({}),
    );
    let kinds: Vec<&str> = l["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["auftrag"]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn einsetzen_und_abgleich_in_einem_schritt() {
    let dir = temp_dir("einsetzen-abgleich");
    let s = setup(&dir);
    let docs = documents("einsetzen-abgleich-docs");
    let quelle = docs.nordfracht.join("save").join("1").join("game.sii");
    let truck_id = s
        .chef
        .add_truck(
            s.company,
            &new_truck_from_save(&quelle, "SPED 001", "SPED 001").unwrap(),
        )
        .unwrap();
    let checkout = s.fahrer.checkout(truck_id).unwrap();
    let package = package_from_checkout(&checkout.truck).unwrap();

    let before = slot_count(&docs.nordfracht);
    let o = sync_and_write(
        &s.fahrer,
        s.company,
        &docs.nordfracht,
        Some(&package),
        &dir.join("backups"),
        &FakeGame::closed(),
        SystemTime::now(),
    )
    .unwrap();
    // Genau ein neuer Speicherplatz, mit Lkw und neuem Kontostand
    assert_eq!(slot_count(&docs.nordfracht), before + 1);
    assert_eq!(o.deployed.name, "SpedHub: SPED 001");
    let (game, _) = Save::read(o.deployed.slot_dir.join("game.sii")).unwrap();
    let (info, _) = Save::read(o.deployed.slot_dir.join("info.sii")).unwrap();
    assert_eq!(truck_state(&game, Some("SPED 001")).unwrap().odometer, 3865);
    assert_eq!(
        (money(&game).unwrap(), info_money(&info).unwrap()),
        (2000, 2000)
    );
    assert!(o.sync.first && o.confirmed);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bauplan_vom_server_wenn_hier_keiner_liegt() {
    let dir = temp_dir("bauplan");
    let s = setup(&dir);
    let url = s.server.url();
    let docs = documents("bauplan-docs");
    let quelle = docs.nordfracht.join("save").join("1").join("game.sii");
    let truck_id = s
        .chef
        .add_truck(
            s.company,
            &new_truck_from_save(&quelle, "SPED 001", "SPED 001").unwrap(),
        )
        .unwrap();
    let checkout = s.fahrer.checkout(truck_id).unwrap();
    let expected = package_from_checkout(&checkout.truck).unwrap();

    // Ein anderer Rechner: leere Ablage, der Bauplan kommt vom Server und wird abgelegt
    let other_pc = AppPaths::new(dir.join("anderer-rechner"));
    assert!(other_pc.load_lease(truck_id).is_none());
    let (p, from) = lease_package(&s.fahrer, &other_pc, truck_id, "SPED 001").unwrap();
    assert_eq!(from, PackageSource::Server);
    assert_eq!(p, expected);
    assert_eq!(other_pc.load_lease(truck_id), Some(expected.clone()));
    let (_, from) = lease_package(&s.fahrer, &other_pc, truck_id, "SPED 001").unwrap();
    assert_eq!(from, PackageSource::Local);

    // Die Chefin darf auch, ein Kollege nicht
    let (p, _) = lease_package(
        &s.chef,
        &AppPaths::new(dir.join("chef")),
        truck_id,
        "SPED 001",
    )
    .unwrap();
    assert_eq!(p, expected);
    let mut kollege = Client::new(&url);
    let t = kollege.register("Kollege", "geheim1234").unwrap().token;
    post_raw(
        &format!("{url}/api/companies/join"),
        &t,
        json!({ "invite_code": s.invite }),
    );
    let e = lease_package(
        &kollege,
        &AppPaths::new(dir.join("kollege")),
        truck_id,
        "SPED 001",
    )
    .unwrap_err();
    assert_eq!(
        e.to_string(),
        "Server: Den Lkw hat jemand anderes ausgeliehen"
    );

    // Damit lässt sich auf dem anderen Rechner einsetzen und zurückgeben
    let d = sync_and_write(
        &s.fahrer,
        s.company,
        &docs.nordfracht,
        Some(&p),
        &other_pc.backups(),
        &FakeGame::closed(),
        SystemTime::now(),
    )
    .unwrap();
    assert_eq!(d.deployed.name, "SpedHub: SPED 001");
    let back = read_for_return(&docs.nordfracht, "SPED 001", Some(&p))
        .unwrap()
        .unwrap();
    assert_eq!(back.odometer_km, 3865);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn passwort_aendern_der_launcher_bleibt_angemeldet() {
    let dir = temp_dir("passwort");
    let s = setup(&dir);
    let url = s.server.url();
    // Eine zweite Anmeldung, zum Beispiel im Dashboard
    let mut dashboard = Client::new(&url);
    dashboard.login("Fahrer", "geheim1234").unwrap();

    // Fehler im Klartext, ohne dass die Anmeldung als abgelaufen gilt
    let e = s
        .fahrer
        .change_password("falsch123", "neuesPasswort1")
        .unwrap_err();
    assert_eq!(e.to_string(), "Server: Das alte Passwort stimmt nicht");
    assert!(!e.is_unauthorized());
    let e = s.fahrer.change_password("geheim1234", "kurz").unwrap_err();
    assert_eq!(
        e.to_string(),
        "Server: Das neue Passwort braucht mindestens 8 Zeichen"
    );
    dashboard.me().unwrap();

    // Ändern: Der Token des Launchers gilt weiter, der zweite nicht mehr
    s.fahrer
        .change_password("geheim1234", "neuesPasswort1")
        .unwrap();
    assert_eq!(s.fahrer.me().unwrap().name, "Fahrer");
    let e = dashboard.me().unwrap_err();
    assert!(e.is_unauthorized(), "{e:?}");
    // Die Chefin ist unberührt, anmelden geht nur noch mit dem neuen Passwort
    s.chef.me().unwrap();
    assert!(
        Client::new(&url)
            .login("Fahrer", "geheim1234")
            .unwrap_err()
            .is_unauthorized()
    );
    Client::new(&url).login("Fahrer", "neuesPasswort1").unwrap();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn fuhrpark_zeigt_ob_der_lkw_gerade_gefahren_wird() {
    let dir = temp_dir("faehrt");
    let s = setup(&dir);
    let url = s.server.url();
    let truck = post_raw(
        &format!("{url}/api/companies/{}/trucks", s.company),
        &s.chef_token,
        json!({ "plate": "SPED 001", "model": "scania.s_2016", "odometer_km": 1000 }),
    );
    let id = truck["id"].as_i64().unwrap();
    s.fahrer.checkout(id).unwrap();
    let t = &s.fahrer.trucks(s.company).unwrap()[0];
    assert!(!t.driving);
    assert_eq!(t.speed_kmh, None);

    // Herzschlag, wie ihn das Plugin schickt: mit Marke und Modell. Ohne Modell gehört er zu
    // keinem Firmen-Lkw.
    post_raw(
        &format!("{url}/api/companies/{}/heartbeat", s.company),
        s.fahrer.token().unwrap(),
        json!({ "plate": "SPED 001", "speed_kmh": 85.0 }),
    );
    assert!(!s.chef.trucks(s.company).unwrap()[0].driving);
    post_raw(
        &format!("{url}/api/companies/{}/heartbeat", s.company),
        s.fahrer.token().unwrap(),
        json!({ "plate": "SPED 001", "speed_kmh": 85.0, "truck_brand": "scania", "truck_model": "s_2016" }),
    );
    let t = &s.chef.trucks(s.company).unwrap()[0];
    assert!(t.driving);
    assert_eq!(t.speed_kmh, Some(85.0));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rueckgabe_mit_viel_verschleiss_bringt_den_lkw_in_die_werkstatt() {
    let dir = temp_dir("werkstatt");
    let s = setup(&dir);
    let url = s.server.url();
    let docs = documents("werkstatt-docs");
    let backups = dir.join("backups");

    // Chef übernimmt den Scania aus dem Spielstand, der Fahrer leiht ihn und setzt ihn ein
    let quelle = docs.nordfracht.join("save").join("1").join("game.sii");
    let truck_id = s
        .chef
        .add_truck(
            s.company,
            &new_truck_from_save(&quelle, "SPED 001", "SPED 001").unwrap(),
        )
        .unwrap();
    let before = s.fahrer.trucks(s.company).unwrap().remove(0);
    assert!(!before.in_workshop);
    let checkout = s.fahrer.checkout(truck_id).unwrap();
    let package = package_from_checkout(&checkout.truck).unwrap();
    let deployed = deploy_truck(
        &docs.nordfracht,
        &package,
        &backups,
        &FakeGame::closed(),
        SystemTime::now(),
    )
    .unwrap();

    // Im Spiel einen Unfall gebaut: viel Verschleiß an Motor und Fahrgestell
    let game_path = deployed.slot_dir.join("game.sii");
    let (mut save, _) = Save::read(&game_path).unwrap();
    let mut state = truck_state(&save, Some("SPED 001")).unwrap();
    state.odometer += 120;
    state.engine_wear = 0.5;
    state.chassis_wear = 0.6;
    set_truck_state(&mut save, Some("SPED 001"), &state).unwrap();
    write_save(&game_path, &save).unwrap();

    // Rückgabe über den echten Ablauf: Zustand aus dem Spielstand lesen und melden
    let back = read_for_return(&docs.nordfracht, "SPED 001", None)
        .unwrap()
        .unwrap();
    let r = s
        .fahrer
        .checkin(truck_id, back.odometer_km, Some(&back.state), None)
        .unwrap();
    assert!(r.in_workshop, "{r:?}");
    assert!(r.damage > 25.0, "{r:?}");
    assert_eq!(r.liability, 0, "Haftung steht auf 0");

    // Die Fuhrparkliste zeigt die Werkstatt und den Schaden
    let t = s.fahrer.trucks(s.company).unwrap().remove(0);
    assert!(t.in_workshop);
    assert!(t.leased_by.is_none());
    assert_eq!(t.state(), TruckState::Workshop);
    // Die Schadensgrenze kommt mit der Firma, für den Strich im Schadensbalken
    assert_eq!(s.fahrer.company(s.company).unwrap().damage_limit, Some(25));
    assert_eq!(t.damage, r.damage);
    assert!(t.damage_text().starts_with(&format!(
        "{} %",
        spedhub_launcher::client::format_percent(r.damage)
    )));
    assert_eq!(t.repair_cost, (t.damage * 150.0).round() as i64);

    // Ausleihen gibt eine Meldung im Klartext, auch für den Chef
    let e = s.fahrer.checkout(truck_id).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Server: Der Lkw steht in der Werkstatt. Ausleihen geht erst, wenn der Chef ihn reparieren lässt"
    );
    assert!(s.chef.checkout(truck_id).is_err());

    // Kasse füllen, reparieren lassen, danach geht es wieder
    post_raw(
        &format!("{url}/api/companies/{}/events", s.company),
        s.fahrer.token().unwrap(),
        json!({ "event_id": "job-1", "type": "job_delivered", "amount": 100000 }),
    );
    post_raw(
        &format!("{url}/api/trucks/{truck_id}/repair"),
        &s.chef_token,
        json!({}),
    );
    let t = s.fahrer.trucks(s.company).unwrap().remove(0);
    assert!(!t.in_workshop);
    assert_eq!(t.damage, 0.0);
    assert_eq!(t.state(), TruckState::Free);
    let again = s.fahrer.checkout(truck_id).unwrap();
    assert_eq!(
        s.fahrer.trucks(s.company).unwrap()[0].state(),
        TruckState::Leased
    );
    let p = package_from_checkout(&again.truck).unwrap();
    assert_eq!((p.state.engine_wear, p.state.chassis_wear), (0.0, 0.0));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn schaden_als_text() {
    let mut t: spedhub_launcher::client::Truck = serde_json::from_value(json!({
        "id": 1, "plate": "SPED 001", "model": "scania.s_2016", "odometer_km": 1000, "rev": 1,
        "leased_by": null
    }))
    .unwrap();
    // Ältere Server ohne Schaden
    assert_eq!((t.damage, t.in_workshop, t.repair_cost), (0.0, false, 0));
    assert_eq!(t.damage_text(), "0 %");
    t.damage = 12.5;
    assert_eq!(t.damage_text(), "12,5 %");
    t.damage_unfixable = 3.0;
    assert_eq!(t.damage_text(), "12,5 %, dauerhaft 3 %");
}

#[test]
fn zustand_im_fuhrpark() {
    use spedhub_launcher::client::Truck;
    let truck = |extra: serde_json::Value| -> Truck {
        let mut v = json!({
            "id": 1, "plate": "SPED 001", "model": "scania.s_2016", "odometer_km": 1000,
            "rev": 1, "leased_by": null
        });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        serde_json::from_value(v).unwrap()
    };
    let lease = json!({ "user_id": 2, "name": "Bora", "since": 5 });
    let retired = json!({ "reason": "ausgemustert", "at": 9 });
    assert_eq!(truck(json!({})).state(), TruckState::Free);
    assert_eq!(
        truck(json!({ "leased_by": lease })).state(),
        TruckState::Leased
    );
    assert_eq!(
        truck(json!({ "in_workshop": true })).state(),
        TruckState::Workshop
    );
    // Ausgemustert geht vor allem anderen
    assert_eq!(
        truck(json!({ "in_workshop": true, "retired": retired })).state(),
        TruckState::Retired
    );
    // Kilometerstand: der gemeldete, wenn der Server einen höheren kennt
    assert_eq!(truck(json!({})).shown_km(), 1000);
    assert_eq!(truck(json!({ "current_km": 1523 })).shown_km(), 1523);
    assert_eq!(truck(json!({ "current_km": 900 })).shown_km(), 1000);
}

#[test]
fn uebernommenen_lkw_als_kauf_verknuepfen() {
    let dir = temp_dir("kauf");
    let s = setup(&dir);
    let url = s.server.url();
    let docs = documents("kauf-docs");
    let company = s.company;

    // Kasse füllen, dann Anschaffungsbudget freigeben
    post_raw(
        &format!("{url}/api/companies/{company}/events"),
        s.fahrer.token().unwrap(),
        json!({ "event_id": "job-1", "type": "job_delivered", "amount": 100000 }),
    );
    let first = s.chef.sync(company, 2000, None, None).unwrap();
    assert_eq!((first.budget, first.write_money), (0, 2000));
    s.chef.sync_written(company, first.sync_id, None).unwrap();
    post_raw(
        &format!("{url}/api/companies/{company}/budgets"),
        &s.chef_token,
        json!({ "amount": 60000, "note": "Neuer Scania" }),
    );

    // Der Abgleich schreibt Lohnkonto plus Vorschuss plus Budget
    let r = s.chef.sync(company, 2000, None, None).unwrap();
    assert_eq!((r.budget, r.write_money), (60000, 62000));
    assert!(
        describe(&r).contains(
            "Neuer Kontostand im Spiel: 62000 € (Lohnkonto 0 € plus Vorschuss 2000 € plus Anschaffungsbudget 60000 €)"
        ),
        "{}",
        describe(&r)
    );
    s.chef.sync_written(company, r.sync_id, None).unwrap();

    // Im Spiel für 45000 € gekauft: genau einmal Betriebskosten, der Rest sinkt
    let r = s.chef.sync(company, 62000 - 45000, None, None).unwrap();
    assert_eq!(r.booked.len(), 1);
    assert_eq!(
        (r.booked[0].kind.as_str(), r.booked[0].amount),
        ("betriebskosten", -45000)
    );
    assert_eq!((r.budget, r.write_money), (15000, 17000));
    s.chef.sync_written(company, r.sync_id, None).unwrap();

    // Lkw übernehmen, dann den Kauf verknüpfen, wie es der Launcher nach "Lkw übernehmen" anbietet
    let quelle = docs.nordfracht.join("save").join("1").join("game.sii");
    let truck_id = s
        .chef
        .add_truck(
            company,
            &new_truck_from_save(&quelle, "SPED 001", "SPED 001").unwrap(),
        )
        .unwrap();
    let candidates = s.chef.purchase_candidates(company).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].amount, -45000);
    let now = unix(SystemTime::now());
    assert!(
        candidates[0]
            .label(now)
            .starts_with("45000 Euro, gerade eben (Abgleich: erwartet 62000 €"),
        "{}",
        candidates[0].label(now)
    );
    let e = s.fahrer.purchase_candidates(company).unwrap_err();
    assert_eq!(e.to_string(), "Server: Das darf nur der Chef");

    let linked = s
        .chef
        .link_purchase(company, candidates[0].id, truck_id)
        .unwrap();
    assert_eq!(
        (
            linked.ledger_id,
            linked.truck_id,
            linked.truck_plate.as_str()
        ),
        (candidates[0].id, truck_id, "SPED 001")
    );
    let e = s
        .chef
        .link_purchase(company, candidates[0].id, truck_id)
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "Server: Die Buchung ist schon als Kauf von SPED 001 verknüpft"
    );
    assert!(s.chef.purchase_candidates(company).unwrap().is_empty());
    let ledger = get_raw(
        &format!("{url}/api/companies/{company}/ledger"),
        &s.chef_token,
    );
    let entry = ledger["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == candidates[0].id)
        .unwrap();
    assert_eq!(entry["truck_plate"], "SPED 001");
    assert_eq!(entry["amount"], -45000);

    // Ausgemustert: steht weiter in der Liste, lässt sich aber nicht mehr ausleihen
    post_raw(
        &format!("{url}/api/trucks/{truck_id}/retire"),
        &s.chef_token,
        json!({}),
    );
    let t = s.fahrer.trucks(company).unwrap().remove(0);
    let retired = t.retired.clone().unwrap();
    assert_eq!(
        (retired.reason.as_str(), retired.text()),
        ("ausgemustert", "ausgemustert".into())
    );
    let e = s.fahrer.checkout(truck_id).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Server: Der Lkw ist ausgemustert und lässt sich nicht mehr ausleihen"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn kauf_und_abgang_als_text() {
    use spedhub_launcher::client::{PurchaseCandidate, Retired, Truck, age_text};
    assert_eq!(age_text(-5), "gerade eben");
    assert_eq!(age_text(59), "gerade eben");
    assert_eq!(age_text(60), "vor 1 Minute");
    assert_eq!(age_text(5 * 60 + 10), "vor 5 Minuten");
    assert_eq!(age_text(3600), "vor 1 Stunde");
    assert_eq!(age_text(3 * 3600), "vor 3 Stunden");
    assert_eq!(age_text(86_400), "vor 1 Tag");
    assert_eq!(age_text(4 * 86_400), "vor 4 Tagen");
    let c = PurchaseCandidate {
        id: 1,
        created_at: 1000,
        amount: -45000,
        note: String::new(),
    };
    assert_eq!(c.label(1000 + 7200), "45000 Euro, vor 2 Stunden");

    let sold = |p: Option<i64>| Retired {
        reason: "verkauft".into(),
        at: 0,
        sale_price: p,
    };
    assert_eq!(sold(Some(30000)).text(), "verkauft für 30000 Euro");
    assert_eq!(sold(Some(0)).text(), "verkauft");
    assert_eq!(sold(None).text(), "verkauft");

    // Ältere Server ohne Abgang und ohne Budget
    let t: Truck = serde_json::from_value(json!({
        "id": 1, "plate": "SPED 001", "model": "scania.s_2016", "odometer_km": 1000, "rev": 1,
        "leased_by": null
    }))
    .unwrap();
    assert_eq!(t.retired, None);
    let t: Truck = serde_json::from_value(json!({
        "id": 1, "plate": "SPED 001", "model": "scania.s_2016", "odometer_km": 1000, "rev": 1,
        "leased_by": null, "retired": { "reason": "verkauft", "at": 5, "sale_price": 100 }
    }))
    .unwrap();
    assert_eq!(t.retired.unwrap().text(), "verkauft für 100 Euro");
}

#[test]
fn version_beim_anmelden_pruefen() {
    let dir = temp_dir("version");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let own = spedhub_launcher::version::VERSION;

    let mut c = Client::new(&url);
    let login = c.register("Alex", "geheim1234").unwrap();
    assert_eq!(login.server_version.as_deref(), Some(own));
    assert_eq!(login.compatible, Some(true));
    assert_eq!(login.version_warning(), None);
    let login = Client::new(&url).login("Alex", "geheim1234").unwrap();
    assert_eq!(login.compatible, Some(true));

    let health = Client::new(&url).health().unwrap();
    assert!(health.ok);
    assert_eq!(health.version.as_deref(), Some(own));
    assert_eq!(
        spedhub_launcher::client::version_warning(health.version.as_deref()),
        None
    );

    // Ein Server, der nicht passt, oder einer ohne Version (vor 0.2)
    let w = spedhub_launcher::client::version_warning(Some("9.9.0")).unwrap();
    assert!(w.contains("9.9.0") && w.contains(own), "{w}");
    let old: spedhub_launcher::client::Login =
        serde_json::from_value(json!({ "user_id": 1, "name": "Alex", "token": "t" })).unwrap();
    assert_eq!(old.server_version, None);
    assert!(
        old.version_warning()
            .unwrap()
            .contains("meldet keine Version")
    );
}

#[test]
fn version_geht_bei_jeder_anfrage_mit() {
    use std::io::{Read, Write};
    // Ein Server, der nur den Kopf festhält und mit einer Anmeldung antwortet
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut s, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && s.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let text = String::from_utf8_lossy(&head).to_lowercase();
            let len: usize = text
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let mut body = vec![0u8; len];
            let _ = s.read_exact(&mut body);
            let answer = r#"{"user_id":1,"name":"Alex","token":"t","server_version":"0.1.0","compatible":false,"ok":true,"version":"0.1.0"}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{answer}",
                answer.len()
            );
            let _ = s.write_all(reply.as_bytes());
            tx.send(text).unwrap();
        }
    });
    let expected = format!("x-spedhub-version: {}", spedhub_launcher::version::VERSION);
    let login = Client::new(&url).login("Alex", "geheim1234").unwrap();
    let head = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(head.contains(&expected), "{head}");
    assert_eq!(login.compatible, Some(false));
    if !spedhub_launcher::version::VERSION.starts_with("0.1.") {
        assert!(login.version_warning().unwrap().contains("Version 0.1.0"));
    }
    Client::new(&url).health().unwrap();
    let head = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(head.starts_with("get /api/health"), "{head}");
    assert!(head.contains(&expected), "{head}");
}

#[test]
fn firmenprofil_beim_server_melden() {
    use spedhub_launcher::flows::profile_to_report;
    use spedhub_launcher::profiles::profile_name_of;
    let dir = temp_dir("profil-melden");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let docs = documents("profil-melden-docs");

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

    // Noch nichts gemeldet: Der Launcher meldet das gewählte Profil, ohne Wahl nichts
    let chosen = profile_name_of(&docs.mueller).unwrap();
    let c = fahrer.company(company).unwrap();
    let me = c
        .members
        .iter()
        .find(|m| m.user_id == fahrer_login.user_id)
        .unwrap();
    assert_eq!(me.profile_name, None);
    assert_eq!(profile_to_report(&c, fahrer_login.user_id, None), None);
    assert_eq!(
        profile_to_report(&c, fahrer_login.user_id, Some("  ")),
        None
    );
    let name = profile_to_report(&c, fahrer_login.user_id, Some(&chosen)).unwrap();
    assert_eq!(name, "Müller Logistik");
    fahrer.set_profile(company, Some(&name)).unwrap();

    // Danach kennt der Server es, eine zweite Meldung ist nicht nötig
    let c = fahrer.company(company).unwrap();
    let me = c
        .members
        .iter()
        .find(|m| m.user_id == fahrer_login.user_id)
        .unwrap();
    assert_eq!(me.profile_name.as_deref(), Some("Müller Logistik"));
    assert_eq!(
        profile_to_report(&c, fahrer_login.user_id, Some(&chosen)),
        None
    );
    // Ein anderes Profil gewählt: wieder melden
    assert_eq!(
        profile_to_report(&c, fahrer_login.user_id, Some("Nordfracht")).as_deref(),
        Some("Nordfracht")
    );
    // Der Chef sieht das Profil des Fahrers
    let c = chef.company(company).unwrap();
    let f = c
        .members
        .iter()
        .find(|m| m.user_id == fahrer_login.user_id)
        .unwrap();
    assert_eq!(f.profile_name.as_deref(), Some("Müller Logistik"));

    // Ab jetzt bucht der Server nur noch aus diesem Profil
    let event =
        |id: &str, profile: &str| send_event(&url, &fahrer_login.token, company, id, profile);
    assert_eq!(event("e-1", "Müller Logistik"), 200);
    assert_eq!(event("e-2", "Privat"), 422);

    // Fehler im Klartext
    let e = Client::new(&url)
        .set_profile(company, Some("X"))
        .unwrap_err();
    assert!(e.is_unauthorized(), "{e:?}");
    let e = fahrer
        .set_profile(company, Some(&"x".repeat(101)))
        .unwrap_err();
    assert!(e.to_string().contains("höchstens 100 Zeichen"), "{e}");
    let _ = fs::remove_dir_all(dir);
}

/// Ein Ereignis, wie es das Plugin schickt. Liefert den Status.
fn send_event(url: &str, token: &str, company: i64, id: &str, profile: &str) -> u16 {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    agent
        .post(&format!("{url}/api/companies/{company}/events"))
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(json!({ "event_id": id, "type": "tollgate", "amount": 5, "profile": profile }))
        .unwrap()
        .status()
        .as_u16()
}

#[test]
fn uebernehmen_mit_vorschlag_und_einsetzen_mit_kennzeichen_der_firma() {
    let dir = temp_dir("firmen-kennzeichen");
    let s = setup(&dir);
    let docs = documents("firmen-kennzeichen-docs");
    let app = AppPaths::new(dir.join("SpedHub"));

    // Die Chefin übernimmt den Scania (im Spielstand SPED 001) mit dem Vorschlag des Servers
    let suggestion = s.chef.plate_suggestion(s.company).unwrap();
    assert_eq!(
        suggestion, "NOR 001",
        "Kürzel von Nordfracht und die erste Nummer"
    );
    let e = s.fahrer.plate_suggestion(s.company).unwrap_err();
    assert_eq!(e.to_string(), "Server: Das darf nur der Chef");
    let quelle = docs.nordfracht.join("save").join("1").join("game.sii");
    let new = new_truck_from_save(&quelle, "SPED 001", &suggestion).unwrap();
    assert_eq!(new.plate, "NOR 001");
    let truck_id = s.chef.add_truck(s.company, &new).unwrap();
    assert_eq!(s.chef.plate_suggestion(s.company).unwrap(), "NOR 002");
    // Ein zweiter Firmen-Lkw, dessen Kennzeichen das Spiel gleich vergeben wird
    let other = s
        .chef
        .add_truck(
            s.company,
            &new_truck_from_save(&quelle, "DD RI 772", "BZ PM 1411").unwrap(),
        )
        .unwrap();
    let trucks = s.fahrer.trucks(s.company).unwrap();
    let mine = trucks.iter().find(|t| t.id == truck_id).unwrap();
    assert_eq!(
        (mine.plate.as_str(), mine.model.as_str()),
        ("NOR 001", "scania.s_2016")
    );

    // Der Fahrer leiht ihn. Im Paket steht danach das Kennzeichen der Firma, mit Land.
    let checkout = s.fahrer.checkout(truck_id).unwrap();
    assert_eq!(checkout.truck.plate, "NOR 001");
    let package = package_from_checkout(&checkout.truck).unwrap();
    assert_eq!(package.license_plate, "NOR 001|_germany");
    // Ein Bauplan in der Ablage von einem älteren Launcher, noch mit SPED 001: bekommt es auch
    let mut old = package.clone();
    old.license_plate = "SPED 001|_germany".into();
    app.store_lease(truck_id, &old).unwrap();
    let (p, from) = lease_package(&s.fahrer, &app, truck_id, &checkout.truck.plate).unwrap();
    assert_eq!(from, PackageSource::Local);
    assert_eq!(p.license_plate, "NOR 001|_germany");

    // Einsetzen mit Geldabgleich: Im neuen Speicherplatz trägt der Lkw NOR 001
    let o = sync_and_write(
        &s.fahrer,
        s.company,
        &docs.nordfracht,
        Some(&p),
        &app.backups(),
        &FakeGame::closed(),
        SystemTime::now(),
    )
    .unwrap();
    assert_eq!(o.deployed.name, "SpedHub: NOR 001");
    let game_sii = o.deployed.slot_dir.join("game.sii");
    let (save, _) = Save::read(&game_sii).unwrap();
    let plates: Vec<String> = list_trucks(&save)
        .unwrap()
        .into_iter()
        .map(|t| t.plate)
        .collect();
    assert_eq!(plates, vec!["NOR 001|_germany".to_string()]);
    let back = read_for_return(&docs.nordfracht, "NOR 001", Some(&p))
        .unwrap()
        .unwrap();
    assert_eq!(back.found_by, spedhub_launcher::savegame::FoundBy::Plate);

    // Das Spiel gibt ihm das Kennzeichen des anderen Firmen-Lkw, wie im Test im Spiel
    let (mut save, _) = Save::read(&game_sii).unwrap();
    set_license_plate(&mut save, Some("NOR 001"), "BZ PM 1411", None).unwrap();
    let mut state = truck_state(&save, None).unwrap();
    state.odometer += 100;
    set_truck_state(&mut save, None, &state).unwrap();
    write_save(&game_sii, &save).unwrap();
    fs::File::options()
        .write(true)
        .open(&game_sii)
        .unwrap()
        .set_modified(SystemTime::now())
        .unwrap();
    let back = read_for_return(&docs.nordfracht, "NOR 001", Some(&p))
        .unwrap()
        .unwrap();
    assert_eq!(back.new_plate.as_deref(), Some("BZ PM 1411"));
    // Die Rückgabe geht trotzdem durch, der Lkw behält sein Kennzeichen
    let r = return_truck(
        &s.fahrer,
        truck_id,
        back.odometer_km,
        Some(&back.state),
        back.new_plate.as_deref(),
    )
    .unwrap();
    assert_eq!(r.plate_taken.as_deref(), Some("BZ PM 1411"));
    assert_eq!(r.checkin.plate, "NOR 001");
    assert_eq!(r.checkin.driven_km, 100);
    let trucks = s.chef.trucks(s.company).unwrap();
    let t = trucks.iter().find(|t| t.id == truck_id).unwrap();
    assert_eq!(t.plate, "NOR 001");
    assert!(t.old_plates.is_empty());
    assert!(t.leased_by.is_none());
    assert_eq!(
        trucks.iter().find(|t| t.id == other).unwrap().plate,
        "BZ PM 1411"
    );
    // Beim nächsten Einsetzen ist es wieder NOR 001
    let again = s.fahrer.checkout(truck_id).unwrap();
    assert_eq!(
        package_from_checkout(&again.truck).unwrap().license_plate,
        "NOR 001|_germany"
    );
    // Ein freies neues Kennzeichen vom Spiel übernimmt der Server wie bisher
    let r = return_truck(
        &s.fahrer,
        truck_id,
        back.odometer_km,
        None,
        Some("EF XU 577"),
    )
    .unwrap();
    assert_eq!(
        (r.plate_taken, r.checkin.plate.as_str()),
        (None, "EF XU 577")
    );
    assert_eq!(r.checkin.plate_ignored, None);
    // Ein Kennzeichen, das sich nicht als eigenes setzen lässt, übernimmt der Server nicht. Der
    // Lkw behält seins und lässt sich weiter einsetzen.
    s.fahrer.checkout(truck_id).unwrap();
    let r = return_truck(
        &s.fahrer,
        truck_id,
        back.odometer_km,
        None,
        Some("MÜ AB 123"),
    )
    .unwrap();
    assert_eq!(r.checkin.plate, "EF XU 577");
    let ignored = r.checkin.plate_ignored.expect("plate_ignored");
    assert_eq!(ignored.plate, "MÜ AB 123");
    assert!(!ignored.reason.is_empty());
    let again = s.fahrer.checkout(truck_id).unwrap();
    assert_eq!(
        package_from_checkout(&again.truck).unwrap().license_plate,
        "EF XU 577|_germany"
    );
    return_truck(&s.fahrer, truck_id, back.odometer_km, None, None).unwrap();
    // Andere Fehler bleiben Fehler
    let e = return_truck(&s.fahrer, truck_id, back.odometer_km, None, Some("XY 1")).unwrap_err();
    assert_eq!(e.to_string(), "Server: Der Lkw ist gar nicht verliehen");
    let _ = fs::remove_dir_all(dir);
}
