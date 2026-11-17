//! Lkw exportieren, einsetzen, entfernen, Zustand und Kennzeichen.
mod common;

use common::*;
use spedhub_save::{
    Error, InsertOptions, Save, TruckPackage, check, custom_plate, export_truck, insert_truck,
    license_plate, list_trucks, plate_country, plate_text, remove_truck, set_license_plate,
    set_truck_state, truck_state, write_save,
};

const PLAYER: &str = "_nameless.206.3353.6c38";
const ASSIGNED: &str = "_nameless.206.3946.b128";
const MINE: &str = "_nameless.206.3946.acc8";
const SPED_001: &str = "_nameless.206.398b.b508";
const RI_772: &str = "_nameless.206.398b.d488";
const TN_190: &str = "_nameless.206.398b.f1c8";

fn package() -> TruckPackage {
    export_truck(&load(QUELLE), None).unwrap()
}

fn keep() -> InsertOptions {
    InsertOptions { keep_names: true }
}

/// Die Quelle mit einem zweiten, freien Stellplatz in der Heimatgarage München.
fn quelle_mit_freiem_platz() -> Save {
    let mut save = load(QUELLE);
    save.set_array(
        "garage.munchen",
        "vehicles",
        &[SPED_001.into(), "null".into()],
    )
    .unwrap();
    save.set_array(
        "garage.munchen",
        "drivers",
        &["driver.1".into(), "null".into()],
    )
    .unwrap();
    assert!(check(&save).is_ok());
    save
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ------------------------------------------------------------------- Export

#[test]
fn export_enthaelt_lkw_mit_platzhaltern() {
    let p = package();
    assert_eq!(p.version, 1);
    assert_eq!(p.model, "scania.s_2016");
    assert_eq!(p.license_plate, "SPED 001|_germany");
    assert_eq!(p.units.len(), 76);
    assert_eq!(p.units[0].kind, "vehicle");
    assert_eq!(p.units[0].source_name.as_deref(), Some(SPED_001));
    assert!(p.units[1..].iter().all(|u| u.kind.ends_with("_accessory")));
    for (i, u) in p.units.iter().enumerate() {
        assert_eq!(u.id, format!("@{i}"));
        for (_, value) in &u.fields {
            assert!(
                !value.starts_with("_nameless."),
                "kein Unit-Name im Bauplan: {value}"
            );
        }
    }
    let accessories: Vec<&str> = p.units[0]
        .fields
        .iter()
        .filter(|(k, _)| k.starts_with("accessories["))
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(accessories.len(), 75);
    assert!(accessories.iter().all(|v| v.starts_with('@')));

    assert_eq!(p.state.odometer, 3865);
    assert_eq!(p.state.fuel_relative.to_bits(), 0x3f367cdb);
    assert_eq!(p.state.chassis_wear.to_bits(), 0x3dd0332a);
    assert_eq!(p.state.wheels_wear.len(), 3);
    assert!(
        p.data_paths
            .contains(&"/def/vehicle/truck/scania.s_2016/data.sii".to_string())
    );
    assert!(
        p.data_paths.windows(2).all(|w| w[0] < w[1]),
        "sortiert, ohne Doppelte"
    );

    // Ein anderer Lkw per Kennzeichen, auch mit Spiel-Markup
    let other = export_truck(&load(QUELLE), Some("dd ri 772")).unwrap();
    assert_eq!(other.model, "mercedes.actros");
    assert_eq!(plate_text(&other.license_plate), "DD RI 772");
}

#[test]
fn paket_uebersteht_json() {
    let p = package();
    let json = p.to_json().unwrap();
    assert_eq!(TruckPackage::from_json(&json).unwrap(), p);
}

// ------------------------------------------------------------------- Referenztest

#[test]
fn einsetzen_mit_quellnamen_ist_byte_genau_wie_transplant_py() {
    let p = TruckPackage::from_json(&package().to_json().unwrap()).unwrap();
    let mut ziel = load(ZIEL);
    let report = insert_truck(&mut ziel, &p, &keep()).unwrap();
    assert!(report.removed_quick_job);
    assert_eq!(report.removed_units, 52);
    assert_eq!(report.vehicle, SPED_001);
    assert_eq!(report.profit_log, "_nameless.2ff.7a11.0001");
    assert_eq!(ziel.dump(), text(ERWARTET));
}

#[test]
fn einsetzen_mit_neuen_namen_entspricht_inhaltlich_der_referenz() {
    let mut ziel = load(ZIEL);
    let report = insert_truck(&mut ziel, &package(), &InsertOptions::default()).unwrap();
    assert!(report.vehicle.starts_with("_nameless.2ff.7a11."));
    assert!(!ziel.contains(SPED_001), "keine Namen aus der Quelle");
    assert_ne!(ziel.dump(), text(ERWARTET));
    assert_eq!(canonical(&ziel), canonical(&load(ERWARTET)));

    // Geschrieben und wieder gelesen bleibt alles gleich
    let dir = temp_dir("einsetzen");
    let path = dir.join("game.sii");
    write_save(&path, &ziel).unwrap();
    let (again, _) = Save::read(&path).unwrap();
    assert_eq!(again, ziel);
    let _ = std::fs::remove_dir_all(dir);
}

// ------------------------------------------------------------------- Einsetzen, weitere Fälle

#[test]
fn einsetzen_neben_eigene_lkw_mit_freiem_platz() {
    let mut p = package();
    p.license_plate = custom_plate("SPED 002", "germany").unwrap();
    let mut save = quelle_mit_freiem_platz();
    let units_before = save.units().len();

    let report = insert_truck(&mut save, &p, &InsertOptions::default()).unwrap();
    let new = report.vehicle.clone();
    assert!(!report.removed_quick_job);
    assert_eq!(report.moved_truck.as_deref(), Some(SPED_001));
    assert_eq!(
        save.units().len(),
        units_before + 77,
        "76 Units Lkw, 1 profit_log"
    );

    assert_eq!(
        save.array("garage.munchen", "vehicles").unwrap(),
        vec![new.clone(), SPED_001.into()]
    );
    assert_eq!(
        save.array("garage.munchen", "drivers").unwrap(),
        strings(&["driver.1", "null"])
    );
    let trucks = save.array(PLAYER, "trucks").unwrap();
    assert_eq!(
        trucks,
        vec![RI_772.into(), TN_190.into(), SPED_001.into(), new.clone()]
    );
    assert_eq!(
        save.array(PLAYER, "truck_profit_logs").unwrap()[3],
        report.profit_log
    );
    assert_eq!(save.get(ASSIGNED, "vehicle").unwrap(), new);
    assert_eq!(save.get(MINE, "vehicle").unwrap(), new);
    assert_eq!(
        save.get(MINE, "stored_vehicle_city").unwrap(),
        "city.munchen"
    );
    assert_eq!(
        save.get(MINE, "stored_vehicle_placement").unwrap(),
        save.get(ASSIGNED, "stored_vehicle_placement").unwrap()
    );
    // Scania war schon in der Liste, kein zweiter Eintrag
    assert_eq!(
        save.array("_nameless.206.3988.7fb8", "owned_trucks")
            .unwrap()
            .len(),
        3
    );

    let state = truck_state(&save, None).unwrap();
    assert_eq!(state, p.state);
    let trucks = list_trucks(&save).unwrap();
    let active: Vec<_> = trucks.iter().filter(|t| t.active).collect();
    assert_eq!(active.len(), 1);
    assert_eq!(plate_text(&active[0].plate), "SPED 002");
    assert!(check(&save).is_ok());
}

#[test]
fn ohne_freien_platz_ist_einsetzen_ein_fehler() {
    let mut p = package();
    p.license_plate = custom_plate("SPED 002", "germany").unwrap();

    let mut save = load(QUELLE);
    let before = save.clone();
    let e = insert_truck(&mut save, &p, &InsertOptions::default()).unwrap_err();
    assert_eq!(e, Error::NoFreeSlot("munchen".into()));
    assert!(e.to_string().contains("kein Stellplatz frei"));
    assert_eq!(save, before, "bei einem Fehler bleibt alles, wie es war");

    // Zweiter Lkw ins Ergebnis des Referenztests: Erfurt hat nur einen Platz
    let mut save = load(ERWARTET);
    let e = insert_truck(&mut save, &p, &InsertOptions::default()).unwrap_err();
    assert_eq!(e, Error::NoFreeSlot("erfurt".into()));
}

#[test]
fn fehlerfaelle_beim_einsetzen() {
    let p = package();

    // Kennzeichen gibt es schon
    let mut save = load(ERWARTET);
    assert_eq!(
        insert_truck(&mut save, &p, &InsertOptions::default()),
        Err(Error::PlateExists("SPED 001".into()))
    );

    // Quellnamen behalten, aber die Namen sind im Ziel schon vergeben
    let mut p2 = p.clone();
    p2.license_plate = custom_plate("SPED 002", "germany").unwrap();
    let mut save = quelle_mit_freiem_platz();
    assert_eq!(
        insert_truck(&mut save, &p2, &keep()),
        Err(Error::NameCollision(SPED_001.into()))
    );

    // Auftrag mit eigenem Lkw
    let mut save = load(ZIEL);
    save.set("_nameless.20e.3039.9ae8", "company_truck", "null")
        .unwrap();
    assert!(matches!(
        insert_truck(&mut save, &p, &InsertOptions::default()),
        Err(Error::Unsupported(_))
    ));

    // Weder eigener Lkw noch Auftrag
    let mut save = load(ZIEL);
    save.set("_nameless.20e.3150.57d8", "current_job", "null")
        .unwrap();
    assert!(matches!(
        insert_truck(&mut save, &p, &InsertOptions::default()),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn kaputte_pakete_werden_abgelehnt() {
    let base = package();
    let mut cases: Vec<(&str, TruckPackage)> = Vec::new();

    let mut p = base.clone();
    p.version = 2;
    cases.push(("Version", p));

    let mut p = base.clone();
    p.units.clear();
    cases.push(("leer", p));

    let mut p = base.clone();
    p.units.swap(0, 1);
    cases.push(("Reihenfolge", p));

    let mut p = base.clone();
    p.units[0].fields[0].1 = "_nameless.206.0000.0001".into();
    cases.push(("Unit-Name statt Platzhalter", p));

    let mut p = base.clone();
    p.units[0].fields.push(("extra".into(), "@999".into()));
    cases.push(("Platzhalter ins Leere", p));

    let mut p = base.clone();
    p.units[1].fields[0].1 = "x\ninjected : _nameless.1 {".into();
    cases.push(("Zeilenumbruch im Wert", p));

    let mut p = base.clone();
    p.units[1].fields[0].0 = "zwei worte".into();
    cases.push(("Leerzeichen im Schlüssel", p));

    let mut p = base.clone();
    p.units[1].kind = "profit_log".into();
    cases.push(("fremder Typ", p));

    let mut p = base.clone();
    p.model = "volvo.fh16".into();
    cases.push(("Modell passt nicht", p));

    let mut p = base.clone();
    p.license_plate = "SPED \"1\"|_germany".into();
    cases.push(("Kennzeichen", p));

    for (label, p) in cases {
        let mut save = load(ZIEL);
        match insert_truck(&mut save, &p, &InsertOptions::default()) {
            Err(Error::InvalidPackage(_)) => {}
            other => panic!("{label}: erwartet InvalidPackage, bekommen {other:?}"),
        }
        assert_eq!(save, load(ZIEL), "{label}: Ziel unverändert");
    }

    // Zustand außerhalb des Bereichs
    let mut p = base.clone();
    p.state.fuel_relative = 1.5;
    assert!(matches!(
        insert_truck(&mut load(ZIEL), &p, &InsertOptions::default()),
        Err(Error::InvalidInput(_))
    ));
    // Anzahl der Räder passt nicht zum Lkw
    let mut p = base.clone();
    p.state.wheels_wear.push(0.0);
    assert!(matches!(
        insert_truck(&mut load(ZIEL), &p, &InsertOptions::default()),
        Err(Error::InvalidInput(_))
    ));
    // Kein JSON
    assert!(matches!(
        TruckPackage::from_json("{ kaputt"),
        Err(Error::InvalidPackage(_))
    ));
    // Ohne Quellnamen geht keep_names nicht
    let mut p = base.clone();
    p.units[3].source_name = None;
    assert!(matches!(
        insert_truck(&mut load(ZIEL), &p, &keep()),
        Err(Error::InvalidPackage(_))
    ));
}

// ------------------------------------------------------------------- Entfernen

#[test]
fn entfernen_nimmt_zubehoer_und_statistik_mit() {
    let mut save = load(QUELLE);
    let before = save.units().len();
    let accessories = save.closure(RI_772, true).len();
    let log = "_nameless.206.3987.c018";
    let log_units = save.closure(log, true).len();
    assert_eq!(log_units, 9, "profit_log mit 8 Einträgen");

    let report = remove_truck(&mut save, "DD RI 772", None).unwrap();
    assert_eq!(report.vehicle, RI_772);
    assert_eq!(report.replacement, None);
    assert_eq!(report.removed_units, accessories + log_units);
    assert_eq!(save.units().len(), before - report.removed_units);
    assert!(!save.contains(RI_772) && !save.contains(log));
    assert_eq!(
        save.array(PLAYER, "trucks").unwrap(),
        strings(&[TN_190, SPED_001])
    );
    assert_eq!(save.array(PLAYER, "truck_profit_logs").unwrap().len(), 2);
    assert_eq!(save.get("garage.dresden", "vehicles[0]").unwrap(), "null");
    assert_eq!(
        save.get("garage.dresden", "drivers[0]").unwrap(),
        "driver.148"
    );
    assert_eq!(
        save.get(ASSIGNED, "vehicle").unwrap(),
        SPED_001,
        "aktiver Lkw bleibt"
    );
    assert!(check(&save).is_ok());
    Save::parse(&save.dump()).unwrap();
}

#[test]
fn aktiven_lkw_entfernen_braucht_ersatz() {
    let mut save = load(QUELLE);
    let before = save.clone();
    let e = remove_truck(&mut save, "SPED 001", None).unwrap_err();
    assert_eq!(e, Error::ReplacementRequired("SPED 001".into()));
    assert!(e.to_string().contains("Ersatz"));
    assert_eq!(save, before);

    let report = remove_truck(&mut save, "sped 001", Some("DD TN 190")).unwrap();
    assert_eq!(report.replacement.as_deref(), Some(TN_190));
    assert_eq!(save.get(ASSIGNED, "vehicle").unwrap(), TN_190);
    assert_eq!(save.get(MINE, "vehicle").unwrap(), TN_190);
    assert_eq!(save.get("garage.munchen", "vehicles[0]").unwrap(), TN_190);
    assert_eq!(save.get("garage.dresden", "vehicles[3]").unwrap(), "null");
    assert_eq!(
        save.array(PLAYER, "trucks").unwrap(),
        strings(&[RI_772, TN_190])
    );
    assert_eq!(
        license_plate(&save, None).unwrap(),
        plate_with_markup("TN 190")
    );
    assert!(check(&save).is_ok());
}

fn plate_with_markup(rest: &str) -> String {
    format!(
        "DD<offset hshift=4 vshift=-5><img src=/material/ui/lp/germany/b.mat color=FFFFFFFF><offset hshift=4 vshift=5>{rest}|germany"
    )
}

#[test]
fn fehlerfaelle_beim_entfernen() {
    let mut save = load(QUELLE);
    assert_eq!(
        remove_truck(&mut save, "XX 999", None),
        Err(Error::TruckNotFound("XX 999".into()))
    );
    assert_eq!(
        remove_truck(&mut save, "SPED 001", Some("XX 999")),
        Err(Error::TruckNotFound("XX 999".into()))
    );
    assert!(matches!(
        remove_truck(&mut save, "SPED 001", Some("SPED 001")),
        Err(Error::InvalidInput(_))
    ));
    // Der Firmen-Lkw im Schnellauftrag gehört dem Spieler nicht
    let mut ziel = load(ZIEL);
    assert!(matches!(
        remove_truck(&mut ziel, "E CT 8139", None),
        Err(Error::TruckNotFound(_))
    ));
    assert_eq!(save, load(QUELLE));
}

// ------------------------------------------------------------------- Zustand

#[test]
fn zustand_lesen_und_setzen() {
    let mut save = load(QUELLE);
    let state = truck_state(&save, None).unwrap();
    assert_eq!(state.odometer, 3865);
    assert_eq!(state.integrity_odometer, 3865);
    assert_eq!(state.wheels_wear.len(), 3);

    // Unveränderter Zustand ändert kein einziges Byte
    set_truck_state(&mut save, None, &state).unwrap();
    assert_eq!(save.dump(), text(QUELLE));

    let mut new = state.clone();
    new.odometer = 4200;
    new.fuel_relative = 0.5;
    new.engine_wear = 0.25;
    new.wheels_wear[1] = 0.0;
    set_truck_state(&mut save, Some("SPED 001"), &new).unwrap();
    assert_eq!(truck_state(&save, None).unwrap(), new);
    let unit = save.unit(SPED_001).unwrap();
    assert_eq!(unit.get("odometer"), Some("4200"));
    assert_eq!(unit.get("fuel_relative"), Some("&3f000000"));
    assert_eq!(unit.get("engine_wear"), Some("&3e800000"));
    assert_eq!(unit.get("wheels_wear[1]"), Some("0"));
    assert_eq!(unit.get("wheels_wear[0]"), Some("&3c95fff2"), "unverändert");
    assert!(check(&save).is_ok());

    // Ein anderer Lkw per Kennzeichen
    let other = truck_state(&save, Some("DD RI 772")).unwrap();
    assert_ne!(other, new);

    // Fehler: Bereich, Anzahl der Räder, unbekannter Lkw. Nichts wird geändert.
    let before = save.clone();
    let mut bad = new.clone();
    bad.cabin_wear = -0.1;
    assert!(matches!(
        set_truck_state(&mut save, None, &bad),
        Err(Error::InvalidInput(_))
    ));
    let mut bad = new.clone();
    bad.odometer_float_part = 1.0;
    assert!(matches!(
        set_truck_state(&mut save, None, &bad),
        Err(Error::InvalidInput(_))
    ));
    let mut bad = new.clone();
    bad.wheels_wear_unfixable.pop();
    assert!(matches!(
        set_truck_state(&mut save, None, &bad),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        set_truck_state(&mut save, Some("XX 1"), &new),
        Err(Error::TruckNotFound(_))
    ));
    assert_eq!(save, before);
}

// ------------------------------------------------------------------- Kennzeichen

#[test]
fn kennzeichen_beide_formen() {
    let game = plate_with_markup("RI 772");
    assert_eq!(plate_text(&game), "DD RI 772");
    assert_eq!(plate_country(&game), "germany");
    assert_eq!(plate_text("SPED 001|_germany"), "SPED 001");
    assert_eq!(plate_country("SPED 001|_germany"), "germany");
    assert!(spedhub_save::plate::is_custom_plate("SPED 001|_germany"));
    assert!(!spedhub_save::plate::is_custom_plate(&game));

    let mut save = load(QUELLE);
    assert_eq!(license_plate(&save, None).unwrap(), "SPED 001|_germany");

    // Selbst vergebenes Kennzeichen ändern, Land bleibt
    let new = set_license_plate(&mut save, Some("SPED 001"), "sped 777", None).unwrap();
    assert_eq!(new, "SPED 777|_germany");
    assert_eq!(
        save.get(SPED_001, "license_plate").unwrap(),
        "\"SPED 777|_germany\""
    );
    assert_eq!(license_plate(&save, Some("Sped 777")).unwrap(), new);

    // Kennzeichen vom Spiel durch ein eigenes ersetzen, mit anderem Land
    let new = set_license_plate(&mut save, Some("DD RI 772"), "WIEN 1", Some("austria")).unwrap();
    assert_eq!(new, "WIEN 1|_austria");
    assert_eq!(
        save.get(RI_772, "license_plate").unwrap(),
        "\"WIEN 1|_austria\""
    );
    assert!(check(&save).is_ok());

    // Fehler
    let before = save.clone();
    assert_eq!(
        set_license_plate(&mut save, None, "DD TN 190", None),
        Err(Error::PlateExists("DD TN 190".into()))
    );
    for bad in ["", "A\"B", "MÜNCHEN 1", "VIEL ZU LANGES KENNZEICHEN"] {
        assert!(
            matches!(
                set_license_plate(&mut save, None, bad, None),
                Err(Error::InvalidInput(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        set_license_plate(&mut save, None, "OK 1", Some("Germany")),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(save, before);
}
