//! Die Kommandozeile einmal mit allen Befehlen durchspielen.
mod common;

use common::*;
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

#[test]
fn alle_befehle() {
    let dir = temp_dir("cli");
    let quelle = testdata(QUELLE);
    let ziel = testdata(ZIEL);
    let paket = dir.join("paket.json");
    let ergebnis = dir.join("ergebnis.sii");

    // Hilfe
    let (code, out, _) = run(&[]);
    assert_eq!(code, 0);
    assert!(out.contains("insert-truck"));

    // check
    let (code, out, _) = run(&["check", s(&quelle)]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("325 Units") && out.contains("Keine Probleme"));

    // decode: Klartext rein, Klartext raus
    let decoded = dir.join("decoded.sii");
    let (code, _, err) = run(&["decode", s(&ziel), s(&decoded)]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(fs::read(&decoded).unwrap(), fs::read(&ziel).unwrap());

    // export und insert mit Quellnamen: byte-genau wie die Referenz
    let (code, out, err) = run(&["export-truck", s(&quelle), s(&paket)]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("SPED 001"));
    let (code, out, err) = run(&[
        "insert-truck",
        s(&ziel),
        s(&paket),
        s(&ergebnis),
        "--keep-names",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Schnellauftrag"));
    assert_eq!(
        fs::read(&ergebnis).unwrap(),
        fs::read(testdata(ERWARTET)).unwrap()
    );

    // Noch einmal in dieselbe Datei: das Kennzeichen gibt es schon, nichts wird geschrieben
    let (code, _, err) = run(&["insert-truck", s(&ergebnis), s(&paket), s(&ergebnis)]);
    assert_eq!(code, 1);
    assert!(err.contains("Kennzeichen SPED 001"), "{err}");
    assert_eq!(
        fs::read(&ergebnis).unwrap(),
        fs::read(testdata(ERWARTET)).unwrap()
    );

    // remove: der aktive Lkw braucht einen Ersatz
    let entfernt = dir.join("entfernt.sii");
    let (code, _, err) = run(&["remove-truck", s(&quelle), "SPED 001", s(&entfernt)]);
    assert_eq!(code, 1);
    assert!(err.contains("Ersatz"), "{err}");
    assert!(!entfernt.exists());
    let (code, out, err) = run(&[
        "remove-truck",
        s(&quelle),
        "SPED 001",
        s(&entfernt),
        "--replacement",
        "DD TN 190",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Aktiver Lkw"));
    let (code, _, _) = run(&["check", s(&entfernt)]);
    assert_eq!(code, 0);

    // money: anzeigen, dann in beiden Dateien setzen, mit Backup
    let game = dir.join("game.sii");
    let info = dir.join("info.sii");
    fs::copy(&ziel, &game).unwrap();
    fs::copy(testdata(INFO), &info).unwrap();
    let (code, out, _) = run(&["money", s(&game), "--info", s(&info)]);
    assert_eq!(code, 0);
    assert_eq!(out.matches(": 2000").count(), 2, "{out}");
    let (code, out, err) = run(&["money", s(&game), "--info", s(&info), "--set", "7500"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Backup"));
    assert!(
        fs::read_to_string(&game)
            .unwrap()
            .contains(" money_account: 7500\n")
    );
    assert!(
        fs::read_to_string(&info)
            .unwrap()
            .contains(" info_money_account: 7500\n")
    );
    let backups = fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".bak")
        })
        .count();
    assert_eq!(backups, 2);

    // check auf kaputter Datei
    let kaputt = dir.join("kaputt.sii");
    fs::write(
        &kaputt,
        text(ZIEL).replace(" bank: _nameless.20e.2752.4a68", " bank: null"),
    )
    .unwrap();
    let (code, out, _) = run(&["check", s(&kaputt)]);
    assert_eq!(code, 1);
    assert!(out.contains("nicht erreichbar"), "{out}");

    // Bedienfehler
    for args in [
        vec!["unbekannt"],
        vec!["check"],
        vec!["check", "a", "b"],
        vec!["export-truck", "a", "b", "--keep-names"],
        vec!["money", s(&game), "--set"],
        vec!["money", s(&game), "--set", "viel"],
        vec!["decode", "a", "b", "--falsch"],
    ] {
        let (code, _, err) = run(&args);
        assert_eq!(code, 2, "{args:?}: {err}");
    }
    let (code, _, err) = run(&["check", s(&dir.join("gibt-es-nicht.sii"))]);
    assert_eq!(code, 1);
    assert!(err.starts_with("Fehler:"));

    let _ = fs::remove_dir_all(dir);
}
