//! Der Client gegen einen echten spedhub-server, dazu der ganze Ablauf: Lkw übernehmen,
//! ausleihen, in den Spielstand schreiben, fahren, zurückgeben.
mod common;

use common::*;
use serde_json::json;
use spedhub_launcher::Error;
use spedhub_launcher::client::Client;
use spedhub_launcher::flows::{PackageSource, describe, lease_package, sync_and_write};
use spedhub_launcher::savegame::{
    SYNC_SLOT_NAME, deploy_truck, new_truck_from_save, package_from_checkout, read_for_return,
};
use spedhub_launcher::settings::AppPaths;
use spedhub_save::{
    Save, info_money, info_name, money, set_money, set_truck_state, truck_state, write_save,
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
            &new_truck_from_save(&quelle, "SPED 001").unwrap(),
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
            &new_truck_from_save(&quelle, "SPED 001").unwrap(),
        )
        .unwrap();
    let checkout = s.fahrer.checkout(truck_id).unwrap();
    let expected = package_from_checkout(&checkout.truck).unwrap();

    // Ein anderer Rechner: leere Ablage, der Bauplan kommt vom Server und wird abgelegt
    let other_pc = AppPaths::new(dir.join("anderer-rechner"));
    assert!(other_pc.load_lease(truck_id).is_none());
    let (p, from) = lease_package(&s.fahrer, &other_pc, truck_id).unwrap();
    assert_eq!(from, PackageSource::Server);
    assert_eq!(p, expected);
    assert_eq!(other_pc.load_lease(truck_id), Some(expected.clone()));
    let (_, from) = lease_package(&s.fahrer, &other_pc, truck_id).unwrap();
    assert_eq!(from, PackageSource::Local);

    // Die Chefin darf auch, ein Kollege nicht
    let (p, _) = lease_package(&s.chef, &AppPaths::new(dir.join("chef")), truck_id).unwrap();
    assert_eq!(p, expected);
    let mut kollege = Client::new(&url);
    let t = kollege.register("Kollege", "geheim1234").unwrap().token;
    post_raw(
        &format!("{url}/api/companies/join"),
        &t,
        json!({ "invite_code": s.invite }),
    );
    let e = lease_package(&kollege, &AppPaths::new(dir.join("kollege")), truck_id).unwrap_err();
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
