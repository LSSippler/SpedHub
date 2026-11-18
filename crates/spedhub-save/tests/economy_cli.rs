//! `spedhub-save economy-mod` über die Kommandozeile, mit der selbst geschriebenen
//! Beispieldatei `testdata/economy_beispiel.sii`.
mod common;

use common::*;
use spedhub_save::zip::read_stored;
use std::{fs, path::Path, process::Command};

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_spedhub-save"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// Die Anleitung nennt Extractor, Archiv, Pfad der Datei und die Doku.
fn ist_anleitung(text: &str) {
    for teil in [
        "SCS Extractor",
        "def.scs",
        "def\\economy_data.sii",
        "docs/wirtschafts-mod.md",
    ] {
        assert!(text.contains(teil), "{teil} fehlt in: {text}");
    }
}

#[test]
fn ohne_datei_kommt_die_anleitung() {
    let (code, out, err) = run(&["economy-mod"]);
    assert_eq!(code, 2);
    assert!(out.is_empty());
    ist_anleitung(&err);

    let dir = temp_dir("economy-cli-fehlt");
    let fehlt = dir.join("economy_data.sii");
    let ziel = dir.join("mod.scs");
    let (code, _, err) = run(&["economy-mod", s(&fehlt), "-o", s(&ziel)]);
    assert_eq!(code, 1);
    assert!(err.contains("gibt es nicht"), "{err}");
    ist_anleitung(&err);
    assert!(!ziel.exists());

    // Hilfe nennt den Befehl
    let (code, out, _) = run(&[]);
    assert_eq!(code, 0);
    assert!(out.contains("economy-mod"));
}

#[test]
fn mod_bauen_nicht_ueberschreiben_und_force() {
    let dir = temp_dir("economy-cli");
    let ziel = dir.join("spedhub_economy.scs");
    let beispiel = testdata("economy_beispiel.sii");

    let (code, out, err) = run(&["economy-mod", s(&beispiel), "-o", s(&ziel)]);
    assert_eq!(code, 0, "{err}");
    for zeile in [
        "Zeile 23: fixed_revenue: 450 -> 4.5",
        "Zeile 24: revenue_coef_per_km: 2.75 -> 0.0275",
        "Zeile 25: cargo_market_revenue_coef_per_km: &40200000 -> &3ccccccd",
        "Zeile 26: driver_revenue_coef_per_km: 1200 -> 12",
        "Zeile 29: driver_cargo_market_revenue_coef_per_km[0]: 1.5 -> 0.015",
        "Nicht verändert, bitte prüfen",
        "Zeile 32: job_bonus_per_km: 0.4",
        "Zeile 35: reward_scale: (1, 2.5, &3f800000)",
        "@include \"beispiel_zusatz.sui\"",
        "Erlösfaktor auf 100",
    ] {
        assert!(out.contains(zeile), "{zeile} fehlt in: {out}");
    }
    assert!(!out.contains("Warnung"), "{out}");
    let zip = fs::read(&ziel).unwrap();
    let entries = read_stored(&zip).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["manifest.sii", "description.txt", "def/economy_data.sii"]
    );
    assert_eq!(
        entries[2].data,
        fs::read(testdata("economy_beispiel_erwartet.sii")).unwrap()
    );

    // Noch einmal ohne --force: Abbruch, die Datei bleibt
    let (code, _, err) = run(&[
        "economy-mod",
        s(&beispiel),
        "-o",
        s(&ziel),
        "--factor",
        "0.5",
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("--force"), "{err}");
    assert_eq!(fs::read(&ziel).unwrap(), zip);

    // Mit --force, anderem Faktor, --also (mehrfach und mit Komma), Version und Bild
    let bild = dir.join("bild.jpg");
    fs::write(&bild, [0xff, 0xd8, 0xff, 0xe0, 0, 0x10]).unwrap();
    let (code, out, err) = run(&[
        "economy-mod",
        s(&beispiel),
        "--output",
        s(&ziel),
        "--factor",
        "0.5",
        "--also",
        "job_bonus_per_km,urgency_bonus",
        "--also",
        "reward_scale",
        "--version",
        "9.9.9",
        "--icon",
        s(&bild),
        "--force",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("fixed_revenue: 450 -> 225"), "{out}");
    assert!(out.contains("job_bonus_per_km: 0.4 -> 0.2"), "{out}");
    assert!(
        out.contains("reward_scale (Komponente 1): 2.5 -> 1.25"),
        "{out}"
    );
    assert!(!out.contains("Nicht verändert"), "{out}");
    assert!(out.contains("Erlösfaktor auf 2"), "{out}");
    let entries = read_stored(&fs::read(&ziel).unwrap()).unwrap();
    let manifest = String::from_utf8(entries[0].data.clone()).unwrap();
    assert!(manifest.contains("package_version: \"9.9.9\""));
    assert!(manifest.contains("icon: \"icon.jpg\""));
    assert!(entries.iter().any(|e| e.name == "icon.jpg"));
    // Nur die Mod liegt im Ordner, keine temporären Dateien
    let mut files: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["bild.jpg", "spedhub_economy.scs"]);
}

#[test]
fn falsche_eingaben_schreiben_nichts() {
    let dir = temp_dir("economy-cli-falsch");
    let ziel = dir.join("mod.scs");
    let beispiel = testdata("economy_beispiel.sii");

    // Keine bekannten Schlüssel: nicht die richtige Datei
    let falsch = dir.join("falsch.sii");
    fs::write(&falsch, "SiiNunit\n{\na : b\n{\n\tfuel_price: 1.9\n}\n}\n").unwrap();
    let (code, _, err) = run(&["economy-mod", s(&falsch), "-o", s(&ziel)]);
    assert_eq!(code, 1);
    assert!(err.contains("keiner der Schlüssel"), "{err}");

    // Verschlüsselt
    let scsc = dir.join("scsc.sii");
    fs::write(&scsc, b"ScsC\0\0\0\0").unwrap();
    let (code, _, err) = run(&["economy-mod", s(&scsc), "-o", s(&ziel)]);
    assert_eq!(code, 1);
    assert!(err.contains("verschlüsselt"), "{err}");

    // Unbekannter Schlüssel bei --also, Faktor außerhalb, Bild kein JPG
    let png = dir.join("bild.png");
    fs::write(&png, b"\x89PNG").unwrap();
    for extra in [
        vec!["--also", "gibt_es_nicht"],
        vec!["--factor", "2"],
        vec!["--factor", "0,01"],
        vec!["--icon", s(&png)],
    ] {
        let mut args = vec!["economy-mod", s(&beispiel), "-o", s(&ziel)];
        args.extend(extra.iter().copied());
        let (code, _, err) = run(&args);
        assert_eq!(code, 1, "{extra:?}: {err}");
    }

    // Zwei Dateien, unbekannte Option
    let (code, _, _) = run(&["economy-mod", s(&beispiel), s(&beispiel)]);
    assert_eq!(code, 2);
    let (code, _, _) = run(&["economy-mod", s(&beispiel), "--plate", "x"]);
    assert_eq!(code, 2);

    assert!(!ziel.exists());
}
