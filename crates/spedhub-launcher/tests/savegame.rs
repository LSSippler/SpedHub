//! Lkw übernehmen, als neuen Speicherplatz einsetzen, für die Rückgabe lesen, Schreibsperre.
mod common;

use common::*;
use serde_json::json;
use spedhub_launcher::Error;
use spedhub_launcher::client::LeasedTruck;
use spedhub_launcher::profiles::list_slots;
use spedhub_launcher::savegame::{
    FoundBy, deploy_truck, new_truck_from_save, package_from_checkout, read_for_return,
    set_company_plate, trucks_in_save,
};
use spedhub_save::{Save, TruckPackage, check, export_truck, list_trucks, plate_text};
use std::{
    fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn package() -> TruckPackage {
    let (save, _) = Save::read(testdata(QUELLE)).unwrap();
    export_truck(&save, None).unwrap()
}

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

#[test]
fn lkw_aus_dem_spielstand_fuer_den_fuhrpark() {
    let trucks = trucks_in_save(&testdata(QUELLE)).unwrap();
    let plates: Vec<&str> = trucks.iter().map(|t| t.plate.as_str()).collect();
    assert_eq!(plates, vec!["DD RI 772", "DD TN 190", "SPED 001"]);
    assert!(
        trucks
            .iter()
            .find(|t| t.plate == "SPED 001")
            .unwrap()
            .active
    );

    let t = new_truck_from_save(&testdata(QUELLE), "sped 001", "SPED 001").unwrap();
    assert_eq!(t.plate, "SPED 001");
    assert_eq!(t.model, "scania.s_2016");
    assert_eq!(t.odometer_km, 3865);
    assert_eq!(t.state["odometer"], 3865);
    let p = TruckPackage::from_json(&t.package).unwrap();
    assert_eq!(p.units.len(), 76);

    // Ein Lkw mit Kennzeichen vom Spiel
    let t = new_truck_from_save(&testdata(QUELLE), "DD RI 772", "DD RI 772").unwrap();
    assert_eq!(t.plate, "DD RI 772");

    assert!(matches!(
        new_truck_from_save(&testdata(QUELLE), "XX 1", "NOR 001"),
        Err(Error::Save(spedhub_save::Error::TruckNotFound(_)))
    ));
}

#[test]
fn paket_vom_server_bekommt_dessen_zustand() {
    let base = package();
    let mut state = base.state.clone();
    state.fuel_relative = 0.25;
    let leased = LeasedTruck {
        id: 1,
        plate: "SPED 001".into(),
        model: "scania.s_2016".into(),
        package: base.to_json().unwrap(),
        odometer_km: 4200,
        rev: 2,
        state: serde_json::to_value(&state).unwrap(),
    };
    let p = package_from_checkout(&leased).unwrap();
    assert_eq!(p.state.fuel_relative, 0.25);
    assert_eq!(p.state.odometer, 4200, "Kilometerstand vom Server");

    // Zustand, den der Server nicht als Lkw-Zustand kennt: der aus dem Bauplan bleibt
    let other = LeasedTruck {
        state: json!({ "chassis_wear": 0.1 }),
        ..leased.clone()
    };
    let p = package_from_checkout(&other).unwrap();
    assert_eq!(p.state.fuel_relative, base.state.fuel_relative);
    assert_eq!(p.state.odometer, 4200);

    // Lkw ohne Bauplan, zum Beispiel im Dashboard angelegt
    let empty = LeasedTruck {
        package: String::new(),
        ..leased
    };
    let e = package_from_checkout(&empty).unwrap_err().to_string();
    assert!(e.contains("keinen Bauplan"), "{e}");
}

#[test]
fn einsetzen_als_neuer_speicherplatz() {
    let docs = documents("einsetzen");
    let backups = docs.ets2.parent().unwrap().join("backups");
    let source_before = snapshot(&docs.nordfracht.join("save").join("autosave"));
    let p = package();

    let d = deploy_truck(&docs.nordfracht, &p, &backups, &FakeGame::closed(), now()).unwrap();
    assert_eq!(d.slot_id, "2", "1 ist schon belegt");
    assert_eq!(d.source_slot, "autosave");
    assert_eq!(d.name, "SpedHub: SPED 001");

    // Der neue Speicherplatz: Lkw drin, aktiv, Spielstand intakt
    let (game, _) = Save::read(d.slot_dir.join("game.sii")).unwrap();
    assert!(check(&game).is_ok());
    let trucks = list_trucks(&game).unwrap();
    assert_eq!(trucks.len(), 1);
    assert!(trucks[0].active && trucks[0].own);
    assert_eq!(plate_text(&trucks[0].plate), "SPED 001");
    assert_eq!(spedhub_save::truck_state(&game, None).unwrap(), p.state);

    // info.sii: CRLF wie beim Spiel, Name und Zeit gesetzt, sonst unverändert
    let info = fs::read_to_string(d.slot_dir.join("info.sii")).unwrap();
    let original = fs::read_to_string(testdata(INFO)).unwrap();
    assert_eq!(
        info,
        original
            .replace(" name: \"\"\r\n", " name: \"SpedHub: SPED 001\"\r\n")
            .replace(" file_time: 1794131316\r\n", " file_time: 1800000000\r\n")
    );
    assert!(info.ends_with("}\r\n\r\n}\r\n"));

    // In der Liste ist der neue Platz der neueste und trägt den Namen
    let slots = list_slots(&docs.nordfracht);
    assert_eq!(slots[0].id, "2");
    assert_eq!(slots[0].label(), "SpedHub: SPED 001");

    // Ausgangs-Speicherplatz unverändert, Backup davon vorhanden
    assert_eq!(
        snapshot(&docs.nordfracht.join("save").join("autosave")),
        source_before
    );
    assert!(d.backup_dir.starts_with(&backups));
    assert_eq!(
        fs::read(d.backup_dir.join("game.sii")).unwrap(),
        fs::read(testdata(ZIEL)).unwrap()
    );
    assert_eq!(
        fs::read(d.backup_dir.join("info.sii")).unwrap(),
        fs::read(testdata(INFO)).unwrap()
    );
    // Keine Reste im Speicherplatz
    let files: Vec<String> = fs::read_dir(&d.slot_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
}

#[test]
fn schreibsperre_bei_laufendem_spiel() {
    let docs = documents("sperre");
    let backups = docs.ets2.parent().unwrap().join("backups");
    let before = snapshot(&docs.ets2);
    let p = package();

    // Spiel läuft
    let e = deploy_truck(&docs.nordfracht, &p, &backups, &FakeGame::running(), now()).unwrap_err();
    assert_eq!(e, Error::GameRunning);
    assert!(e.to_string().contains("läuft gerade"));
    assert_eq!(snapshot(&docs.ets2), before);
    assert!(!backups.exists(), "nicht einmal ein Backup");

    // Prüfung nicht möglich: ebenfalls nichts schreiben
    let failing = FakeGame::sequence(vec![Err("Zugriff verweigert".into())]);
    let e = deploy_truck(&docs.nordfracht, &p, &backups, &failing, now()).unwrap_err();
    assert!(e.to_string().contains("nichts geschrieben"), "{e}");
    assert_eq!(snapshot(&docs.ets2), before);

    // Spiel startet zwischen Lesen und Schreiben: zweite Prüfung greift
    let late = FakeGame::sequence(vec![Ok(false), Ok(true)]);
    let e = deploy_truck(&docs.nordfracht, &p, &backups, &late, now()).unwrap_err();
    assert_eq!(e, Error::GameRunning);
    assert_eq!(snapshot(&docs.ets2), before, "kein neuer Speicherplatz");
}

#[test]
fn einsetzen_scheitert_ohne_spuren() {
    let docs = documents("scheitert");
    let backups = docs.ets2.parent().unwrap().join("backups");
    // Neuester Spielstand hat den Lkw schon
    add_slot(&docs.nordfracht, "autosave", ERWARTET, Some(INFO), 0);
    let before = snapshot(&docs.ets2);
    let e = deploy_truck(
        &docs.nordfracht,
        &package(),
        &backups,
        &FakeGame::closed(),
        now(),
    )
    .unwrap_err();
    assert!(
        e.to_string().contains("steht schon im neuesten Spielstand")
            && e.to_string().contains("SPED 001"),
        "{e}"
    );
    assert_eq!(snapshot(&docs.ets2), before);

    // Dasselbe, nachdem das Spiel Namen und Kennzeichen ersetzt hat: über die Teile erkannt,
    // obwohl die Prüfung auf gleiches Kennzeichen nicht mehr greifen würde
    add_slot(&docs.nordfracht, "autosave", NACH_SPIEL, Some(INFO), 0);
    let before = snapshot(&docs.ets2);
    let e = deploy_truck(
        &docs.nordfracht,
        &package(),
        &backups,
        &FakeGame::closed(),
        now(),
    )
    .unwrap_err();
    assert!(
        e.to_string().contains("unter dem Kennzeichen EF XU 577"),
        "{e}"
    );
    assert_eq!(snapshot(&docs.ets2), before);

    // Profil ohne Spielstand
    let e = deploy_truck(
        &docs.mueller,
        &package(),
        &backups,
        &FakeGame::closed(),
        now(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("noch keinen Spielstand"));
}

#[test]
fn rueckgabe_liest_den_neuesten_spielstand() {
    let docs = documents("rueckgabe");
    // Neuester Spielstand ohne den Lkw: von Hand eingeben
    assert_eq!(
        read_for_return(&docs.nordfracht, "SPED 001", None).unwrap(),
        None
    );

    // Nach dem Einsetzen ist er da
    add_slot(&docs.nordfracht, "2", ERWARTET, Some(INFO), 0);
    let r = read_for_return(&docs.nordfracht, "sped 001", None)
        .unwrap()
        .unwrap();
    assert_eq!(r.odometer_km, 3865);
    assert_eq!(r.slot, "2");
    assert_eq!(r.state["odometer"], 3865);
    assert!(r.state["wheels_wear"].is_array());
}

#[test]
fn rueckgabe_findet_den_lkw_nach_neuem_kennzeichen() {
    let docs = documents("neues-kennzeichen");
    // Das Spiel hat gespeichert: neue Namen, neues Kennzeichen, 425 km mehr
    add_slot(&docs.nordfracht, "autosave", NACH_SPIEL, Some(INFO), 0);
    let p = package();

    // Ohne Paket nur über das Kennzeichen: nicht zu finden
    assert_eq!(
        read_for_return(&docs.nordfracht, "SPED 001", None).unwrap(),
        None
    );

    // Mit Paket über Modell und Teile, das neue Kennzeichen kommt mit
    let r = read_for_return(&docs.nordfracht, "SPED 001", Some(&p))
        .unwrap()
        .unwrap();
    assert_eq!(r.odometer_km, 4290);
    assert_eq!(r.found_by, FoundBy::Parts { share: 1.0 });
    assert_eq!(r.new_plate.as_deref(), Some("EF XU 577"));
    assert_eq!(r.state["fuel_relative"], 0.5);

    // Meldet der Server schon das neue Kennzeichen, reicht das Kennzeichen
    let r = read_for_return(&docs.nordfracht, "EF XU 577", Some(&p))
        .unwrap()
        .unwrap();
    assert_eq!(r.found_by, FoundBy::Plate);
    assert_eq!(r.new_plate, None);
}

#[test]
fn rueckgabe_raet_nicht_bei_zwei_gleichen_lkw() {
    let docs = documents("zwei-gleiche");
    // Ein Spielstand, in dem derselbe Scania zweimal steht, keiner mehr als SPED 001
    let (mut save, _) = Save::read(testdata(NACH_SPIEL)).unwrap();
    let mut second = package();
    second.license_plate = spedhub_save::custom_plate("EF ZZ 999", "germany").unwrap();
    let garage = "garage.erfurt";
    let player = save.names_of_kind("player").remove(0);
    let driver = save.array(&player, "drivers").unwrap().remove(0);
    let first = save.get(garage, "vehicles[0]").unwrap().to_string();
    save.set_array(garage, "vehicles", &[first, "null".into()])
        .unwrap();
    save.set_array(garage, "drivers", &[driver, "null".into()])
        .unwrap();
    spedhub_save::insert_truck(&mut save, &second, &spedhub_save::InsertOptions::default())
        .unwrap();
    let dir = add_slot(&docs.nordfracht, "autosave", NACH_SPIEL, Some(INFO), 0);
    spedhub_save::write_save(dir.join("game.sii"), &save).unwrap();

    let e = read_for_return(&docs.nordfracht, "SPED 001", Some(&package())).unwrap_err();
    assert!(
        matches!(e, Error::Save(spedhub_save::Error::AmbiguousTruck(_))),
        "{e:?}"
    );
    assert!(e.to_string().contains("gleich gut"));
}

/// Was der Server beim Ausleihen zu einem Paket liefert, mit seinem Kennzeichen.
fn leased(package: &TruckPackage, plate: &str) -> LeasedTruck {
    LeasedTruck {
        id: 7,
        plate: plate.into(),
        model: package.model.clone(),
        package: package.to_json().unwrap(),
        odometer_km: i64::from(package.state.odometer),
        rev: 1,
        state: serde_json::to_value(&package.state).unwrap(),
    }
}

#[test]
fn einsetzen_setzt_das_kennzeichen_vom_server() {
    let docs = documents("server-kennzeichen");
    let backups = docs.ets2.parent().unwrap().join("backups");
    // Im Bauplan steht das Kennzeichen aus dem Spielstand des Chefs, der Server sagt NOR 007
    let base = package();
    assert_eq!(base.license_plate, "SPED 001|_germany");
    let p = package_from_checkout(&leased(&base, "nor  007")).unwrap();
    assert_eq!(
        p.license_plate, "NOR 007|_germany",
        "selbst vergeben, Land aus dem Bauplan"
    );
    assert_eq!(p.units, base.units, "der Bauplan selbst bleibt");

    let d = deploy_truck(&docs.nordfracht, &p, &backups, &FakeGame::closed(), now()).unwrap();
    assert_eq!(d.name, "SpedHub: NOR 007");
    let (game, _) = Save::read(d.slot_dir.join("game.sii")).unwrap();
    assert!(check(&game).is_ok());
    let trucks = list_trucks(&game).unwrap();
    assert_eq!(trucks.len(), 1);
    assert_eq!(trucks[0].plate, "NOR 007|_germany");
    assert!(spedhub_save::plate::is_custom_plate(&trucks[0].plate));
    assert!(!trucks.iter().any(|t| plate_text(&t.plate) == "SPED 001"));

    // Die Rückgabe findet ihn über das Kennzeichen vom Server
    let r = read_for_return(&docs.nordfracht, "NOR 007", Some(&p))
        .unwrap()
        .unwrap();
    assert_eq!(r.found_by, FoundBy::Plate);
    assert_eq!(r.new_plate, None);
    assert_eq!(r.odometer_km, 3865);

    // Ein Lkw mit Kennzeichen vom Spiel: Das Land kommt ohne Unterstrich, gesetzt wird es mit
    let (quelle, _) = Save::read(testdata(QUELLE)).unwrap();
    let actros = export_truck(&quelle, Some("DD RI 772")).unwrap();
    assert!(actros.license_plate.ends_with("RI 772|germany"));
    let mut a = actros.clone();
    set_company_plate(&mut a, "NOR 008").unwrap();
    assert_eq!(a.license_plate, "NOR 008|_germany");
}

#[test]
fn kennzeichen_vom_server_das_sich_nicht_setzen_laesst() {
    let base = package();
    // Ein ausgemusterter Lkw heißt etwa "SPED 001 (Lkw 7)", ein Kennzeichen vom Spiel kann
    // Zeichen haben, die sich nicht selbst vergeben lassen
    for bad in [
        "SPED 001 (Lkw 7)",
        "MÜNCHEN 1",
        "VIEL ZU LANGES KENNZEICHEN",
    ] {
        let e = package_from_checkout(&leased(&base, bad))
            .unwrap_err()
            .to_string();
        assert!(e.contains("Dashboard ändern"), "{bad}: {e}");
    }
    // Ohne Land im Bauplan
    let mut p = base.clone();
    p.license_plate = "SPED 001".into();
    let e = set_company_plate(&mut p, "NOR 1").unwrap_err().to_string();
    assert!(e.contains("kein Land"), "{e}");
    assert_eq!(p.license_plate, "SPED 001", "unverändert");
}

#[test]
fn uebernehmen_mit_kennzeichen_der_firma() {
    // Das Kennzeichen aus dem Spielstand wird nicht das Firmen-Kennzeichen
    let t = new_truck_from_save(&testdata(QUELLE), "SPED 001", " nor 003 ").unwrap();
    assert_eq!(t.plate, "NOR 003");
    assert_eq!(t.model, "scania.s_2016");
    // Der Bauplan bleibt, wie er im Spielstand war
    let p = TruckPackage::from_json(&t.package).unwrap();
    assert_eq!(p.license_plate, "SPED 001|_germany");
    // Ein Kennzeichen, das sich nicht einsetzen ließe, gar nicht erst anlegen
    let e = new_truck_from_save(&testdata(QUELLE), "SPED 001", "MÜNCHEN 1")
        .unwrap_err()
        .to_string();
    assert!(e.contains("MÜNCHEN 1"), "{e}");
}

#[test]
fn rueckgabe_nimmt_kein_kennzeichen_mit_fremdem_modell() {
    let docs = documents("fremdes-modell");
    // Neuester Spielstand: die Quelle mit Actros DD RI 772, DAF DD TN 190 und Scania SPED 001
    add_slot(&docs.nordfracht, "autosave", QUELLE, Some(INFO), 0);
    let scania = package();

    // Der Server kennt den Scania als DD RI 772: Den Actros mit diesem Kennzeichen nimmt die
    // Rückgabe nicht, über die Teile findet sie den Scania
    let r = read_for_return(&docs.nordfracht, "DD RI 772", Some(&scania))
        .unwrap()
        .unwrap();
    assert_eq!(r.found_by, FoundBy::Parts { share: 1.0 });
    assert_eq!(r.new_plate.as_deref(), Some("SPED 001"));
    assert_eq!(r.odometer_km, 3865);
    // Ohne Paket gibt es nur das Kennzeichen, dann wie bisher der Lkw mit dem Kennzeichen
    let r = read_for_return(&docs.nordfracht, "DD RI 772", None)
        .unwrap()
        .unwrap();
    assert_eq!(r.found_by, FoundBy::Plate);
    assert_ne!(r.odometer_km, 3865, "der Actros");

    // Zwei Lkw mit demselben Kennzeichen, einer davon das Modell aus dem Paket: über die Teile
    let (mut save, _) = Save::read(testdata(QUELLE)).unwrap();
    let actros = "_nameless.206.398b.d488";
    save.set(actros, "license_plate", "\"SPED 001|_germany\"")
        .unwrap();
    let dir = add_slot(&docs.nordfracht, "autosave", QUELLE, Some(INFO), 0);
    spedhub_save::write_save(dir.join("game.sii"), &save).unwrap();
    let r = read_for_return(&docs.nordfracht, "SPED 001", Some(&scania))
        .unwrap()
        .unwrap();
    assert_eq!(r.found_by, FoundBy::Parts { share: 1.0 });
    assert_eq!(r.new_plate, None);
    assert_eq!(r.odometer_km, 3865);
    let e = read_for_return(&docs.nordfracht, "SPED 001", None).unwrap_err();
    assert!(
        matches!(e, Error::Save(spedhub_save::Error::AmbiguousPlate(_))),
        "{e:?}"
    );
}
