//! Lesen in allen Formaten, Integritätsprüfung, sicheres Schreiben, kaputte Eingaben.
mod common;

use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use common::*;
use flate2::{Compression, write::ZlibEncoder};
use spedhub_save::sii::{LineEnding, TextStyle};
use spedhub_save::{
    Error, Format, InsertOptions, Problem, Save, check, export_truck, insert_truck, money,
    remove_truck, set_money, truck_state, write_save, write_saves,
};
use std::{fs, io::Write, path::Path};

/// Baut eine verschlüsselte Datei (ScsC) wie das Spiel: zlib, dann AES-256-CBC.
/// Der Schlüssel ist öffentlich bekannt und steht genauso in `decrypt_truck` (MIT).
fn encrypt(text: &[u8]) -> Vec<u8> {
    const KEY: [u8; 32] = [
        0x2a, 0x5f, 0xcb, 0x17, 0x91, 0xd2, 0x2f, 0xb6, 0x02, 0x45, 0xb3, 0xd8, 0x36, 0x9e, 0xd0,
        0xb2, 0xc2, 0x73, 0x71, 0x56, 0x3f, 0xbf, 0x1f, 0x3c, 0x9e, 0xdf, 0x6b, 0x11, 0x82, 0x5a,
        0x5d, 0x0a,
    ];
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(text).unwrap();
    let compressed = z.finish().unwrap();
    let iv = [7u8; 16];
    let len = compressed.len();
    let mut buf = compressed;
    buf.resize(len + 16, 0);
    let cipher = cbc::Encryptor::<aes::Aes256>::new_from_slices(&KEY, &iv).unwrap();
    let ct = cipher.encrypt_padded_mut::<Pkcs7>(&mut buf, len).unwrap();
    let mut out = b"ScsC".to_vec();
    out.extend([0u8; 32]); // HMAC, wird beim Lesen nicht geprüft
    out.extend(iv);
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(ct.iter());
    out
}

#[test]
fn alle_testdaten_bleiben_byte_genau() {
    let mut count = 0;
    for entry in fs::read_dir(testdata("")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "sii") {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let (save, format) = Save::from_bytes(&bytes).unwrap();
        assert_eq!(format, Format::Text);
        assert_eq!(save.dump().as_bytes(), bytes, "{}", path.display());
        assert!(
            check(&save).is_ok(),
            "{}: {:?}",
            path.display(),
            check(&save)
        );
        count += 1;
    }
    assert_eq!(count, 4, "alle vier Testdateien gefunden");
}

#[test]
fn verschluesselte_datei_wird_gelesen() {
    for name in [ZIEL, INFO] {
        let plain = fs::read(testdata(name)).unwrap();
        let (save, format) = Save::from_bytes(&encrypt(&plain)).unwrap();
        assert_eq!(format, Format::Encrypted);
        assert_eq!(save.dump().as_bytes(), plain);
    }
}

#[test]
fn kaputte_formate_sind_fehler() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("leer", vec![]),
        ("zu kurz", b"Si".to_vec()),
        ("unbekannt", b"PK\x03\x04 kein Spielstand".to_vec()),
        ("ScsC nur Kopf", b"ScsC".to_vec()),
        ("ScsC Müll", [b"ScsC".as_slice(), &[0x55; 200]].concat()),
        ("BSII Müll", [b"BSII".as_slice(), &[0xAA; 64]].concat()),
        (
            "BSII Kopf",
            [b"BSII".as_slice(), &2u32.to_le_bytes()].concat(),
        ),
        ("kein UTF-8", b"SiiNunit\n{\n\xff\xfe}".to_vec()),
    ];
    for (label, bytes) in cases {
        match Save::from_bytes(&bytes) {
            Err(Error::Decode(_)) => {}
            other => panic!("{label}: erwartet Decode-Fehler, bekommen {other:?}"),
        }
    }
    // Verschlüsselt, aber mit kaputtem Inhalt
    let mut broken = encrypt(text(ZIEL).as_bytes());
    let n = broken.len();
    broken.truncate(n - 20);
    assert!(Save::from_bytes(&broken).is_err());
}

#[test]
fn kaputter_text_meldet_die_zeile() {
    let ok = "SiiNunit\n{\nbank : _nameless.1 {\n money_account: 5\n}\n\n}";
    assert!(Save::parse(ok).is_ok());
    let cases = [
        ("CR ohne LF", ok.replace("account: 5\n", "account: 5\r"), 4),
        ("ohne Kopf", ok.replace("SiiNunit\n", ""), 1),
        ("zwei Zeilenumbrüche am Ende", format!("{ok}\n\n"), 9),
        ("ohne Leerzeile", ok.replace("}\n\n}", "}\n}"), 6),
        ("zwei Leerzeilen", ok.replace("}\n\n}", "}\n\n\n}"), 7),
        ("Feld ohne Einrückung", ok.replace(" money", "money"), 4),
        (
            "Feld ohne Doppelpunkt",
            ok.replace("account: 5", "account 5"),
            4,
        ),
        (
            "Kopf kaputt",
            ok.replace("bank : _nameless.1 {", "bank _nameless.1 {"),
            3,
        ),
        (
            "doppelter Name",
            ok.replace("\n\n}", "\n\nbank : _nameless.1 {\n}\n\n}"),
            7,
        ),
        ("Leerzeile in der Unit", ok.replace("}\n\n}", "\n}\n\n}"), 5),
        ("nicht geschlossen", ok.replace("}\n\n}", "}"), 3),
    ];
    for (label, input, line) in cases {
        match Save::parse(&input) {
            Err(Error::Parse { line: l, message }) => {
                assert_eq!(l, line, "{label}: {message}");
                assert!(!message.is_empty());
            }
            other => panic!("{label}: erwartet Parse-Fehler, bekommen {other:?}"),
        }
    }
}

#[test]
fn crlf_und_zeilenumbruch_am_ende_bleiben_erhalten() {
    // info.sii, wie das Spiel sie schreibt: CRLF und ein Zeilenumbruch nach der letzten }
    let raw = fs::read(testdata(INFO)).unwrap();
    assert!(raw.ends_with(b"&3758cbf7\r\n}\r\n\r\n}\r\n"));
    let (save, _) = Save::from_bytes(&raw).unwrap();
    assert_eq!(
        save.style(),
        TextStyle {
            line_ending: LineEnding::CrLf,
            final_newline: true
        }
    );
    assert_eq!(save.dump().as_bytes(), raw);

    // Alle vier Kombinationen mit dem großen Ziel, auch nach einer Änderung
    let lf = text(ZIEL);
    for ending in [LineEnding::Lf, LineEnding::CrLf] {
        for final_newline in [false, true] {
            let nl = ending.as_str();
            let mut input = lf.replace('\n', nl);
            if final_newline {
                input.push_str(nl);
            }
            let mut save = Save::parse(&input).unwrap();
            assert_eq!(save.style().line_ending, ending);
            assert_eq!(save.style().final_newline, final_newline);
            assert_eq!(save.dump(), input);
            assert_eq!(save, load(ZIEL), "dieselben Units");
            set_money(&mut save, 1).unwrap();
            assert_eq!(
                save.dump(),
                input.replace(" money_account: 2000", " money_account: 1")
            );
        }
    }

    // Leerer Spielstand mit CRLF
    let empty = "SiiNunit\r\n{\r\n}\r\n";
    assert_eq!(Save::parse(empty).unwrap().dump(), empty);
}

#[test]
fn gemischte_zeilenenden_werden_vereinheitlicht() {
    let lf = text(ZIEL);
    let lines: Vec<&str> = lf.split('\n').collect();
    let join = |minority: &str, majority: &str| {
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            out.push_str(line);
            if i + 1 < lines.len() {
                out.push_str(if i % 10 == 3 { minority } else { majority });
            }
        }
        out
    };
    let mostly_crlf = join("\n", "\r\n");
    let save = Save::parse(&mostly_crlf).unwrap();
    assert_eq!(save.style().line_ending, LineEnding::CrLf);
    assert_eq!(save.dump(), lf.replace('\n', "\r\n"));

    let mostly_lf = join("\r\n", "\n");
    let save = Save::parse(&mostly_lf).unwrap();
    assert_eq!(save.style().line_ending, LineEnding::Lf);
    assert_eq!(save.dump(), lf);
}

#[test]
fn einrueckung_mit_tabs_und_mehreren_leerzeichen() {
    let lf = text(ZIEL);
    let variants = ["\t", "  ", "\t ", " ", "\t\t", "    "];
    let mut n = 0;
    let mixed: Vec<String> = lf
        .split('\n')
        .map(|line| match line.strip_prefix(' ') {
            Some(rest) => {
                n += 1;
                format!("{}{rest}", variants[n % variants.len()])
            }
            None => line.to_string(),
        })
        .collect();
    let mixed = mixed.join("\n");
    let save = Save::parse(&mixed).unwrap();
    assert_eq!(save.dump(), mixed, "jede Einrückung bleibt, wie sie war");
    // Gleiche Units, Schlüssel und Werte wie das Original, nur die Einrückung ist anders
    let orig = load(ZIEL);
    let pairs = |u: &spedhub_save::Unit| -> Vec<(String, String)> {
        u.fields
            .iter()
            .map(|f| (f.key.clone(), f.value.clone()))
            .collect()
    };
    assert_eq!(orig.units().len(), save.units().len());
    for (a, b) in orig.units().iter().zip(save.units()) {
        assert_eq!(a.name, b.name);
        assert_eq!(pairs(a), pairs(b));
    }
    assert_eq!(money(&save).unwrap(), 2000);
    assert!(check(&save).is_ok());

    // Nur Tabs: Neue Felder beim Einsetzen bekommen ebenfalls einen Tab
    let tabs = lf.replace("\n ", "\n\t");
    let mut save = Save::parse(&tabs).unwrap();
    assert_eq!(save.indent(), "\t");
    let package = export_truck(&load(QUELLE), None).unwrap();
    insert_truck(&mut save, &package, &InsertOptions { keep_names: true }).unwrap();
    assert_eq!(save.dump(), text(ERWARTET).replace("\n ", "\n\t"));
}

#[test]
fn pruefung_findet_verwaiste_und_kaputte_verweise() {
    let mut save = load(ZIEL);
    assert!(check(&save).is_ok());

    // Verweis ins Leere
    save.set("_nameless.20e.298c.1238", "bank", "_nameless.dead.beef")
        .unwrap();
    let report = check(&save);
    assert!(
        report.problems.iter().any(
            |p| matches!(p, Problem::Dangling { target, .. } if target == "_nameless.dead.beef")
        )
    );
    // Die Bank hängt jetzt an nichts mehr
    assert!(
        report.problems.iter().any(
            |p| matches!(p, Problem::Orphan { unit, .. } if unit == "_nameless.20e.2752.4a68")
        )
    );

    // Liste mit falscher Zählzeile
    let mut save = load(ZIEL);
    save.set("garage.erfurt", "vehicles", "2").unwrap();
    assert!(
        check(&save)
            .problems
            .iter()
            .any(|p| matches!(p, Problem::BrokenList { field, .. } if field == "vehicles"))
    );

    // Leerer Spielstand
    let empty = Save::parse("SiiNunit\n{\n}").unwrap();
    assert_eq!(check(&empty).problems, vec![Problem::NoUnits]);
}

#[test]
fn schreiben_mit_backup_und_ohne_bei_fehler() {
    let dir = temp_dir("schreiben");
    let path = dir.join("game.sii");
    fs::write(&path, b"alt").unwrap();

    // Kaputter Spielstand: nichts wird geschrieben, die alte Datei bleibt
    let mut broken = load(ZIEL);
    broken
        .set("_nameless.20e.298c.1238", "player", "_nameless.weg")
        .unwrap();
    assert!(matches!(
        write_save(&path, &broken),
        Err(Error::Integrity(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), b"alt");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "keine Reste");

    // Intakter Spielstand: Backup der alten Datei, neue Datei byte-genau
    let save = load(ZIEL);
    let written = write_save(&path, &save).unwrap();
    let backup = written.backup.expect("Backup angelegt");
    assert_eq!(fs::read(&backup).unwrap(), b"alt");
    assert_eq!(fs::read_to_string(&path).unwrap(), text(ZIEL));

    // Ein zweites Mal: neues Backup, das alte bleibt
    let second = write_save(&path, &save).unwrap().backup.unwrap();
    assert_ne!(second, backup);
    assert!(backup.exists());

    // Mehrere Dateien: ist eine kaputt, wird keine geschrieben
    let other = dir.join("info.sii");
    let info = load(INFO);
    let items: Vec<(&Path, &Save)> = vec![(&other, &info), (&path, &broken)];
    assert!(write_saves(&items).is_err());
    assert!(!other.exists());
    let _ = fs::remove_dir_all(dir);
}

/// Lässt alle Operationen auf einem möglicherweise kaputten Spielstand laufen. Ergebnis egal,
/// es darf nur keine Panic geben.
fn exercise(save: &Save, package: &spedhub_save::TruckPackage) {
    let _ = check(save);
    let _ = money(save);
    let _ = truck_state(save, None);
    let _ = export_truck(save, None);
    let _ = spedhub_save::list_trucks(save);
    let mut a = save.clone();
    let _ = insert_truck(&mut a, package, &InsertOptions::default());
    let mut b = save.clone();
    let _ = remove_truck(&mut b, "SPED 001", Some("DD TN 190"));
    let mut c = save.clone();
    let _ = spedhub_save::set_license_plate(&mut c, None, "X 1", None);
}

#[test]
fn kaputte_eingaben_fuehren_nie_zur_panic() {
    let package = export_truck(&load(QUELLE), None).unwrap();

    // Jede einzelne Zeile des Ziels weglassen oder halbieren
    let ziel = text(ZIEL);
    let lines: Vec<&str> = ziel.split('\n').collect();
    for i in 0..lines.len() {
        for variant in 0..2 {
            let mut l = lines.clone();
            let half;
            if variant == 0 {
                l.remove(i);
            } else {
                half = &lines[i][..lines[i].len() / 2];
                l[i] = half;
            }
            if let Ok(save) = Save::parse(&l.join("\n")) {
                exercise(&save, &package);
            }
        }
    }

    // Jede 17. Zeile der Quelle weglassen
    let quelle = text(QUELLE);
    let lines: Vec<&str> = quelle.split('\n').collect();
    for i in (0..lines.len()).step_by(17) {
        let mut l = lines.clone();
        l.remove(i);
        if let Ok(save) = Save::parse(&l.join("\n")) {
            exercise(&save, &package);
        }
    }

    // Zufällige Bytes verändern
    let mut rng = Lcg(42);
    let bytes = ziel.as_bytes();
    for _ in 0..300 {
        let mut b = bytes.to_vec();
        for _ in 0..1 + rng.below(4) {
            let pos = rng.below(b.len());
            b[pos] = rng.next() as u8;
        }
        if let Ok((save, _)) = Save::from_bytes(&b) {
            exercise(&save, &package);
        }
    }
}
