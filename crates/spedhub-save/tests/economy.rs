//! Wirtschafts-Mod: Erlös-Werte skalieren und die Mod bauen. Alle Tests arbeiten mit der
//! selbst geschriebenen Beispieldatei `testdata/economy_beispiel.sii`, nie mit dem Original
//! von SCS.
mod common;

use common::*;
use spedhub_save::{
    Error, Factor, ModOptions, ScaleOptions, build_mod, economy, scale_economy, write_mod,
    zip::read_stored,
};
use std::fs;

const BEISPIEL: &str = "economy_beispiel.sii";
/// Das Beispiel mit Faktor 0.01, von Hand geprüft
const ERWARTET: &str = "economy_beispiel_erwartet.sii";

fn beispiel() -> Vec<u8> {
    fs::read(testdata(BEISPIEL)).unwrap()
}

fn erwartet() -> Vec<u8> {
    fs::read(testdata(ERWARTET)).unwrap()
}

fn options(also: &[&str]) -> ScaleOptions {
    ScaleOptions {
        factor: Factor::parse("0.01").unwrap(),
        also: also.iter().map(|s| s.to_string()).collect(),
    }
}

fn crlf(data: &[u8]) -> Vec<u8> {
    String::from_utf8(data.to_vec())
        .unwrap()
        .replace('\n', "\r\n")
        .into_bytes()
}

/// Die Änderungen als `schluessel: alt -> neu`, Vektoren mit Komponente.
fn liste(edit: &spedhub_save::EconomyEdit) -> Vec<String> {
    edit.changes
        .iter()
        .map(|c| match c.component {
            Some(n) => format!("{}#{n}: {} -> {}", c.key, c.old, c.new),
            None => format!("{}: {} -> {}", c.key, c.old, c.new),
        })
        .collect()
}

#[test]
fn bekannte_schluessel_werden_skaliert() {
    let edit = scale_economy(&beispiel(), &options(&[])).unwrap();
    assert_eq!(
        liste(&edit),
        [
            // ganze Zahl, Ergebnis nicht ganz
            "fixed_revenue: 450 -> 4.5",
            // Dezimalzahl, exakt gerechnet
            "revenue_coef_per_km: 2.75 -> 0.0275",
            // Hex bleibt Hex: 2.5 mal 0.01 als 32-Bit-Float
            "cargo_market_revenue_coef_per_km: &40200000 -> &3ccccccd",
            // ganze Zahl, Ergebnis ganz
            "driver_revenue_coef_per_km: 1200 -> 12",
            // Liste: die Einträge, nicht die Zählzeile, auch mit Exponent
            "driver_cargo_market_revenue_coef_per_km[0]: 1.5 -> 0.015",
            "driver_cargo_market_revenue_coef_per_km[1]: 3e1 -> 0.3",
        ]
    );
    assert_eq!(f32::from_bits(0x3ccc_cccd), 0.025);
    let fractions: Vec<&str> = edit
        .changes
        .iter()
        .filter(|c| c.became_fraction)
        .map(|c| c.key.as_str())
        .collect();
    assert_eq!(fractions, ["fixed_revenue"]);
    assert!(
        edit.changes
            .iter()
            .all(|c| c.unit == "economy_data : economy.data")
    );
    assert_eq!(edit.changes[0].line, 23);
    assert!(edit.missing.is_empty());
    assert!(edit.skipped.is_empty());
    assert_eq!(edit.includes.len(), 1);
    assert_eq!(edit.includes[0].text, "@include \"beispiel_zusatz.sui\"");
    assert_eq!(edit.text, erwartet());
}

#[test]
fn ausserhalb_der_zahlen_bleibt_alles_gleich() {
    let vorher = String::from_utf8(beispiel()).unwrap();
    let edit = scale_economy(vorher.as_bytes(), &options(&[])).unwrap();
    let nachher = String::from_utf8(edit.text).unwrap();
    let a: Vec<&str> = vorher.split('\n').collect();
    let b: Vec<&str> = nachher.split('\n').collect();
    assert_eq!(a.len(), b.len());
    let geaendert: Vec<usize> = edit.changes.iter().map(|c| c.line).collect();
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        if geaendert.contains(&(i + 1)) {
            // Nur die Zahl ist anders: Schlüssel, Einrückung und Kommentar bleiben
            let (kx, _) = x.split_once(": ").unwrap();
            let (ky, _) = y.split_once(": ").unwrap();
            assert_eq!(kx, ky);
            assert_ne!(x, y);
        } else {
            assert_eq!(x, y, "Zeile {}", i + 1);
        }
    }
    // Kommentare, Zeichenketten und die Zählzeile sind nicht angefasst
    for unveraendert in [
        "// Auskommentiert, bleibt unverändert: fixed_revenue: 999",
        "mehrere Zeilen mit revenue_coef_per_km: 77",
        "fixed_revenue: 4.5 # Kommentar mit fixed_revenue: 999",
        "\tdriver_cargo_market_revenue_coef_per_km: 2\n",
        "notiz: \"revenue_coef_per_km: 5\"",
        "loan_interest: &3c23d70a",
        "fuel_price: 1.9",
    ] {
        assert!(nachher.contains(unveraendert), "{unveraendert}");
    }
}

#[test]
fn crlf_bleibt_crlf() {
    let edit = scale_economy(&crlf(&beispiel()), &options(&[])).unwrap();
    assert_eq!(edit.text, crlf(&erwartet()));
    assert_eq!(edit.changes.len(), 6);
    // Mit BOM am Anfang geht es auch, der BOM bleibt
    let mut bom = b"\xEF\xBB\xBF".to_vec();
    bom.extend(beispiel());
    let edit = scale_economy(&bom, &options(&[])).unwrap();
    assert!(edit.text.starts_with(b"\xEF\xBB\xBFSiiNunit"));
    assert_eq!(&edit.text[3..], erwartet().as_slice());
}

#[test]
fn kandidaten_werden_gemeldet_und_mit_also_skaliert() {
    let edit = scale_economy(&beispiel(), &options(&[])).unwrap();
    let kandidaten: Vec<String> = edit
        .candidates
        .iter()
        .map(|c| format!("{}: {}", c.key, c.value))
        .collect();
    assert_eq!(
        kandidaten,
        [
            "job_bonus_per_km: 0.4",
            "urgency_bonus[]: 0.1",
            "urgency_bonus[]: 0.2",
            "reward_scale: (1, 2.5, &3f800000)",
        ]
    );

    let edit = scale_economy(
        &beispiel(),
        &options(&["job_bonus_per_km", "urgency_bonus", "reward_scale"]),
    )
    .unwrap();
    assert!(edit.candidates.is_empty());
    let liste = liste(&edit);
    assert_eq!(liste.len(), 6 + 1 + 2 + 3);
    for erwartet in [
        "job_bonus_per_km: 0.4 -> 0.004",
        "urgency_bonus[]: 0.1 -> 0.001",
        "urgency_bonus[]: 0.2 -> 0.002",
        "reward_scale#0: 1 -> 0.01",
        "reward_scale#1: 2.5 -> 0.025",
    ] {
        assert!(
            liste.contains(&erwartet.to_string()),
            "{erwartet}: {liste:?}"
        );
    }
    let text = String::from_utf8(edit.text).unwrap();
    let hex = format!("&{:08x}", 0.01f32.to_bits());
    assert!(text.contains(&format!("\treward_scale: (0.01, 0.025, {hex})\n")));
    assert!(text.contains("\turgency_bonus[]: 0.001\n\turgency_bonus[]: 0.002\n"));
    // experience_per_km klingt nicht nach Geld und bleibt
    assert!(text.contains("\texperience_per_km: 3\n"));
}

#[test]
fn also_mit_unbekanntem_schluessel_bricht_ab() {
    let err = scale_economy(&beispiel(), &options(&["gibt_es_nicht"])).unwrap_err();
    assert!(err.to_string().contains("gibt_es_nicht"), "{err}");
    let err = scale_economy(&beispiel(), &options(&["a[0]"])).unwrap_err();
    assert!(err.to_string().contains("ohne Index"), "{err}");
}

#[test]
fn ohne_bekannte_schluessel_bricht_ab() {
    let text =
        b"SiiNunit\n{\nirgendwas : etwas.anderes\n{\n\tfuel_price: 1.9\n\tjob_bonus: 2\n}\n}\n";
    let err = scale_economy(text, &options(&[])).unwrap_err();
    assert!(err.to_string().contains("keiner der Schlüssel"), "{err}");
    // Auch dann nicht, wenn sie nur in Kommentaren und Zeichenketten stehen
    let text = b"SiiNunit\n{\na : b\n{\n\t# fixed_revenue: 1\n\tx: \"fixed_revenue: 1\"\n}\n}\n";
    assert!(scale_economy(text, &options(&[])).is_err());
}

#[test]
fn fehlende_bekannte_schluessel_werden_gemeldet() {
    let text = b"SiiNunit\n{\neconomy_data : economy.data\n{\n\tfixed_revenue: 300\n}\n}\n";
    let edit = scale_economy(text, &options(&[])).unwrap();
    assert_eq!(liste(&edit), ["fixed_revenue: 300 -> 3"]);
    assert_eq!(
        edit.missing,
        [
            "revenue_coef_per_km",
            "cargo_market_revenue_coef_per_km",
            "driver_revenue_coef_per_km",
            "driver_cargo_market_revenue_coef_per_km",
        ]
    );
}

#[test]
fn verschluesselt_binaer_und_kaputt_brechen_ab() {
    for (data, wort) in [
        (b"ScsC\x01\x02\x03".to_vec(), "ScsC"),
        (b"BSII\x01\x02\x03".to_vec(), "BSII"),
        (b"3nK\x01\x02".to_vec(), "3nK"),
        (b"economy_data : x {}".to_vec(), "SiiNunit"),
    ] {
        let err = scale_economy(&data, &options(&[])).unwrap_err();
        assert!(err.to_string().contains(wort), "{err}");
    }
    // Klammern, die nicht aufgehen
    let text = b"SiiNunit\n{\neconomy_data : economy.data\n{\n\tfixed_revenue: 300\n}\n";
    assert!(matches!(
        scale_economy(text, &options(&[])),
        Err(Error::Parse { .. })
    ));
}

#[test]
fn wert_ohne_zahl_bricht_ab() {
    for wert in ["\"600\"", "nil", "(1, x)", "&3f80", "1,5"] {
        let text = format!(
            "SiiNunit\n{{\neconomy_data : economy.data\n{{\n\tfixed_revenue: {wert}\n}}\n}}\n"
        );
        let err = scale_economy(text.as_bytes(), &options(&[])).unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }), "{wert}: {err}");
    }
}

#[test]
fn anderer_faktor() {
    let factor = Factor::parse("0.5").unwrap();
    let edit = scale_economy(
        &beispiel(),
        &ScaleOptions {
            factor,
            also: Vec::new(),
        },
    )
    .unwrap();
    assert!(liste(&edit).contains(&"fixed_revenue: 450 -> 225".to_string()));
    let desc = economy::description(&edit, &ModOptions::default());
    assert!(desc.contains("50 Prozent"), "{desc}");
    assert!(desc.contains("Erlösfaktor 2 zurück"), "{desc}");
}

#[test]
fn mod_enthaelt_manifest_beschreibung_und_datei() {
    let edit = scale_economy(&beispiel(), &options(&[])).unwrap();
    let zip = build_mod(&edit, &ModOptions::default()).unwrap();
    let entries = read_stored(&zip).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["manifest.sii", "description.txt", "def/economy_data.sii"]
    );
    assert_eq!(entries[2].data, erwartet());

    let manifest = String::from_utf8(entries[0].data.clone()).unwrap();
    assert!(manifest.starts_with("SiiNunit\n{\nmod_package : .package_name\n{\n"));
    for zeile in [
        format!("\tpackage_version: \"{}\"\n", env!("CARGO_PKG_VERSION")),
        "\tdisplay_name: \"SpedHub Wirtschaft\"\n".into(),
        "\tauthor: \"SpedHub\"\n".into(),
        "\tcategory[]: \"other\"\n".into(),
        "\tcompatible_versions[]: \"1.62.*\"\n".into(),
        "\tdescription_file: \"description.txt\"\n".into(),
        "\tmp_mod_optional: false\n".into(),
    ] {
        assert!(manifest.contains(&zeile), "{zeile}");
    }
    assert!(!manifest.contains("icon"));
    // Das Manifest ist selbst eine gültige SII-Datei mit ausgeglichenen Klammern
    assert_eq!(manifest.matches('{').count(), manifest.matches('}').count());

    let desc = String::from_utf8(entries[1].data.clone()).unwrap();
    for teil in [
        "1 Prozent",
        "Erlösfaktor 100",
        "Pflicht",
        "https://github.com/LSSippler/SpedHub",
        "fixed_revenue, revenue_coef_per_km",
    ] {
        assert!(desc.contains(teil), "{teil}");
    }
    assert!(!desc.contains(" – ") && !desc.contains(" — "));
}

#[test]
fn mod_mit_vorschaubild_und_eigener_version() {
    let edit = scale_economy(&beispiel(), &options(&[])).unwrap();
    let icon = vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
    let options = ModOptions {
        version: "1.2.3".into(),
        compatible: "1.63.*".into(),
        icon: Some(icon.clone()),
    };
    let entries = read_stored(&build_mod(&edit, &options).unwrap()).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "manifest.sii",
            "description.txt",
            "icon.jpg",
            "def/economy_data.sii"
        ]
    );
    assert_eq!(entries[2].data, icon);
    let manifest = String::from_utf8(entries[0].data.clone()).unwrap();
    assert!(manifest.contains("\ticon: \"icon.jpg\"\n"));
    assert!(manifest.contains("\tpackage_version: \"1.2.3\"\n"));
    assert!(manifest.contains("\tcompatible_versions[]: \"1.63.*\"\n"));

    for falsch in ["", "1.2 3", "\"x\"", "a\\b"] {
        let o = ModOptions {
            version: falsch.into(),
            ..ModOptions::default()
        };
        assert!(economy::manifest(&o).is_err(), "{falsch:?}");
    }
}

#[test]
fn ziel_wird_ohne_force_nicht_ueberschrieben() {
    let dir = temp_dir("economy-ziel");
    let ziel = dir.join("spedhub_economy.scs");
    fs::write(&ziel, b"alt").unwrap();
    let err = write_mod(&ziel, b"neu", false).unwrap_err();
    assert!(err.to_string().contains("--force"), "{err}");
    assert_eq!(fs::read(&ziel).unwrap(), b"alt");
    write_mod(&ziel, b"neu", true).unwrap();
    assert_eq!(fs::read(&ziel).unwrap(), b"neu");
    // Keine temporären Dateien übrig
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    // Ein Ordner als Ziel geht nicht
    assert!(write_mod(&dir, b"x", true).is_err());
}
