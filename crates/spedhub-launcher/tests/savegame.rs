//! Lkw übernehmen, als neuen Speicherplatz einsetzen, für die Rückgabe lesen, Schreibsperre.
mod common;

use common::*;
use serde_json::json;
use spedhub_launcher::Error;
use spedhub_launcher::client::LeasedTruck;
use spedhub_launcher::profiles::list_slots;
use spedhub_launcher::savegame::{
    deploy_truck, new_truck_from_save, package_from_checkout, read_for_return, trucks_in_save,
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

    let t = new_truck_from_save(&testdata(QUELLE), "sped 001").unwrap();
    assert_eq!(t.plate, "SPED 001");
    assert_eq!(t.model, "scania.s_2016");
    assert_eq!(t.odometer_km, 3865);
    assert_eq!(t.state["odometer"], 3865);
    let p = TruckPackage::from_json(&t.package).unwrap();
    assert_eq!(p.units.len(), 76);

    // Ein Lkw mit Kennzeichen vom Spiel
    let t = new_truck_from_save(&testdata(QUELLE), "DD RI 772").unwrap();
    assert_eq!(t.plate, "DD RI 772");

    assert!(matches!(
        new_truck_from_save(&testdata(QUELLE), "XX 1"),
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
        matches!(e, Error::Save(spedhub_save::Error::PlateExists(_))),
        "{e:?}"
    );
    assert!(e.to_string().contains("SPED 001"));
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
    assert_eq!(read_for_return(&docs.nordfracht, "SPED 001").unwrap(), None);

    // Nach dem Einsetzen ist er da
    add_slot(&docs.nordfracht, "2", ERWARTET, Some(INFO), 0);
    let r = read_for_return(&docs.nordfracht, "sped 001")
        .unwrap()
        .unwrap();
    assert_eq!(r.odometer_km, 3865);
    assert_eq!(r.slot, "2");
    assert_eq!(r.state["odometer"], 3865);
    assert!(r.state["wheels_wear"].is_array());
}
