//! Logik ohne Server: Übersetzung, event_ids, Konfiguration, Log, Postausgang.
mod common;

use common::*;
use spedhub_plugin::event::{Draft, offence_label};
use spedhub_plugin::ids::{IdGenerator, StartNote};
use spedhub_plugin::log::{Logger, rotated};
use spedhub_plugin::outbox::{Entry, Outbox, read_entries};
use spedhub_plugin::{Agent, Booking, Config, ConfigError, GameplayData, Paths, translate};
use std::{collections::HashSet, fs};

fn data(id: &str) -> GameplayData {
    GameplayData {
        id: id.into(),
        ..Default::default()
    }
}

fn draft(kind: &'static str, amount: i64, note: &str) -> Option<Draft> {
    Some(Draft {
        kind,
        amount,
        distance_km: None,
        note: note.into(),
    })
}

// ------------------------------------------------------------------- Übersetzung

#[test]
fn alle_events_werden_uebersetzt() {
    let delivered = GameplayData {
        revenue: Some(12_480),
        distance_km: Some(523.4),
        cargo_damage: Some(0.025),
        ..data("job.delivered")
    };
    assert_eq!(
        translate(&delivered),
        Ok(Some(Draft {
            distance_km: Some(523),
            ..draft(
                "job_delivered",
                12_480,
                "Auftrag abgeliefert, 523 km, Frachtschaden 2,5 %"
            )
            .unwrap()
        }))
    );
    // Die Strecke geht als eigenes Feld an den Server, gerundet
    let booking = translate(&delivered)
        .unwrap()
        .unwrap()
        .with_id("a-1".into());
    let json = serde_json::to_value(&booking).unwrap();
    assert_eq!(json["distance_km"], 523);
    assert_eq!(json["type"], "job_delivered");

    let cancelled = GameplayData {
        cancel_penalty: Some(1_250),
        ..data("job.cancelled")
    };
    assert_eq!(
        translate(&cancelled),
        Ok(draft("job_cancelled", 1_250, "Auftrag abgebrochen"))
    );

    let fined = GameplayData {
        fine_amount: Some(400),
        fine_offence: Some("speeding_camera".into()),
        ..data("player.fined")
    };
    assert_eq!(
        translate(&fined),
        Ok(draft("fined", 400, "Strafe: Blitzer"))
    );

    let toll = GameplayData {
        pay_amount: Some(35),
        ..data("player.tollgate.paid")
    };
    assert_eq!(translate(&toll), Ok(draft("tollgate", 35, "Maut")));

    let ferry = GameplayData {
        pay_amount: Some(780),
        source_name: Some("Calais".into()),
        target_name: Some("Dover".into()),
        ..data("player.use.ferry")
    };
    assert_eq!(
        translate(&ferry),
        Ok(draft("ferry", 780, "Fähre Calais nach Dover"))
    );

    let train = GameplayData {
        pay_amount: Some(560),
        target_name: Some("Folkestone".into()),
        ..data("player.use.train")
    };
    assert_eq!(
        translate(&train),
        Ok(draft("train", 560, "Zug nach Folkestone"))
    );
}

#[test]
fn notizen_ohne_zusatzwerte_und_grenzfaelle() {
    // Ohne Strecke und Schaden
    let d = GameplayData {
        revenue: Some(1),
        ..data("job.delivered")
    };
    assert_eq!(translate(&d).unwrap().unwrap().note, "Auftrag abgeliefert");
    assert_eq!(translate(&d).unwrap().unwrap().distance_km, None);
    // Kein Schaden, krumme Werte, NaN
    let d = GameplayData {
        revenue: Some(1),
        distance_km: Some(f32::NAN),
        cargo_damage: Some(0.0),
        ..data("job.delivered")
    };
    assert_eq!(
        translate(&d).unwrap().unwrap().note,
        "Auftrag abgeliefert, Frachtschaden 0,0 %"
    );
    assert_eq!(translate(&d).unwrap().unwrap().distance_km, None);
    // Ohne Strecke fehlt das Feld ganz, alte Zeilen im Postausgang lesen sich weiter
    let b = translate(&d).unwrap().unwrap().with_id("a-2".into());
    let json = serde_json::to_string(&b).unwrap();
    assert!(!json.contains("distance_km"), "{json}");
    assert!(!json.contains("occurred_at"), "{json}");
    let old: Booking =
        serde_json::from_str(r#"{"event_id":"a-3","type":"tollgate","amount":5,"note":"Maut"}"#)
            .unwrap();
    assert_eq!(old.distance_km, None);
    assert_eq!(old.occurred_at, None);
    // Mit Zeitpunkt geht er als Unix-Sekunden mit
    let b = translate(&d)
        .unwrap()
        .unwrap()
        .with_id_at("a-4".into(), 1_795_000_000);
    assert_eq!(b.occurred_at, Some(1_795_000_000));
    let json = serde_json::to_string(&b).unwrap();
    assert!(json.contains(r#""occurred_at":1795000000"#), "{json}");
    // Unbekannte Strafe bleibt mit Originaltext, fehlende Strafe ist schlicht "Strafe"
    let f = GameplayData {
        fine_amount: Some(10),
        fine_offence: Some("future_thing".into()),
        ..data("player.fined")
    };
    assert_eq!(translate(&f).unwrap().unwrap().note, "Strafe: future_thing");
    let f = GameplayData {
        fine_amount: Some(10),
        ..data("player.fined")
    };
    assert_eq!(translate(&f).unwrap().unwrap().note, "Strafe");
    // Fähre ohne Orte
    let f = GameplayData {
        pay_amount: Some(10),
        source_name: Some("  ".into()),
        ..data("player.use.ferry")
    };
    assert_eq!(translate(&f).unwrap().unwrap().note, "Fähre");
    // Betrag 0 ist erlaubt
    let t = GameplayData {
        pay_amount: Some(0),
        ..data("player.tollgate.paid")
    };
    assert_eq!(translate(&t).unwrap().unwrap().amount, 0);
}

#[test]
fn alle_strafen_haben_eine_deutsche_bezeichnung() {
    for offence in [
        "crash",
        "avoid_sleeping",
        "wrong_way",
        "speeding_camera",
        "no_lights",
        "red_signal",
        "speeding",
        "avoid_weighing",
        "illegal_trailer",
        "avoid_inspection",
        "illegal_border_crossing",
        "hard_shoulder_violation",
        "damaged_vehicle_usage",
        "generic",
    ] {
        assert!(offence_label(offence).is_some(), "{offence}");
    }
    assert_eq!(offence_label("unbekannt"), None);
}

#[test]
fn fehlende_oder_negative_betraege_und_fremde_events() {
    for id in [
        "job.delivered",
        "job.cancelled",
        "player.fined",
        "player.tollgate.paid",
        "player.use.ferry",
        "player.use.train",
    ] {
        let e = translate(&data(id)).unwrap_err();
        assert!(e.contains("fehlt"), "{id}: {e}");
    }
    let negative = GameplayData {
        revenue: Some(-5),
        ..data("job.delivered")
    };
    assert!(translate(&negative).unwrap_err().contains("negativ"));
    assert_eq!(translate(&data("player.use.helicopter")), Ok(None));
    assert_eq!(translate(&data("")), Ok(None));
}

// ------------------------------------------------------------------- event_id

#[test]
fn ids_bleiben_ueber_neustarts_eindeutig() {
    let dir = temp_dir("ids");
    let state = dir.join("plugin-state.json");
    let mut all = HashSet::new();

    let (first, note) = IdGenerator::start(&state);
    assert_eq!(note, StartNote::Created);
    // Gleichzeitig aus mehreren Threads
    let ids: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|_| s.spawn(|| (0..250).map(|_| first.next()).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    assert_eq!(ids.len(), 1000);
    all.extend(ids);
    assert_eq!(all.len(), 1000, "keine Doppelten innerhalb eines Starts");

    // Neustart: derselbe Zustand, neue IDs
    for restart in 2..=5 {
        let (g, note) = IdGenerator::start(&state);
        assert_eq!(note, StartNote::Ok);
        for _ in 0..100 {
            let id = g.next();
            assert!(
                id.len() <= 100,
                "Server erlaubt höchstens 100 Zeichen: {id}"
            );
            assert!(all.insert(id), "doppelt nach Neustart {restart}");
        }
    }
    let first_id = first.next();
    let prefix: Vec<&str> = first_id.split('-').collect();
    assert_eq!(prefix[0].len(), 32);
    assert_eq!(prefix[1], "1");

    // Kaputter Zustand: neue Kennung, also auch keine Kollision mit den alten IDs
    fs::write(&state, "{ kaputt").unwrap();
    let (g, note) = IdGenerator::start(&state);
    assert!(matches!(note, StartNote::Replaced(_)));
    let id = g.next();
    assert!(all.insert(id.clone()));
    assert!(!id.starts_with(prefix[0]), "neue Kennung");

    // Zustand lässt sich nicht speichern: Zufall im Präfix, zwei Starts trotzdem verschieden
    let blocked = dir.join("ist-ein-ordner");
    fs::create_dir_all(&blocked).unwrap();
    let (a, note_a) = IdGenerator::start(&blocked);
    let (b, note_b) = IdGenerator::start(&blocked);
    assert!(matches!(note_a, StartNote::NotSaved(_)), "{note_a:?}");
    assert!(matches!(note_b, StartNote::NotSaved(_)));
    let (ia, ib) = (a.next(), b.next());
    assert_ne!(ia, ib);
    assert!(all.insert(ia) && all.insert(ib));
    let _ = fs::remove_dir_all(dir);
}

// ------------------------------------------------------------------- Konfiguration

#[test]
fn konfiguration_fehlt_oder_ist_kaputt() {
    let dir = temp_dir("config");
    let paths = Paths::new(&dir);

    // Fehlt: kein Agent, aber ein Eintrag im Log
    match Agent::start(&paths, fast()) {
        Err(ConfigError::Missing(p)) => assert_eq!(p, paths.config()),
        other => panic!("erwartet Missing, bekommen {:?}", other.err()),
    }
    assert!(read_log(&paths).contains("Keine Konfiguration"));
    assert!(
        !paths.outbox().exists(),
        "ohne Konfiguration passiert nichts"
    );
    assert!(!paths.state().exists());

    let cases = [
        ("{ kaputt", "kein JSON"),
        (
            r#"{"server_url": "http://x", "token": "t"}"#,
            "company_id fehlt",
        ),
        (
            r#"{"server_url": "ftp://x", "token": "t", "company_id": 1}"#,
            "falsches Schema",
        ),
        (
            r#"{"server_url": "http://", "token": "t", "company_id": 1}"#,
            "leere Adresse",
        ),
        (
            r#"{"server_url": "http://x", "token": "  ", "company_id": 1}"#,
            "leerer Token",
        ),
        (
            r#"{"server_url": "http://x", "token": "a b", "company_id": 1}"#,
            "Token mit Leerzeichen",
        ),
        (
            r#"{"server_url": "http://x", "token": "t", "company_id": 0}"#,
            "Firma 0",
        ),
    ];
    for (text, label) in cases {
        fs::write(paths.config(), text).unwrap();
        assert!(
            matches!(Config::load(&paths.config()), Err(ConfigError::Invalid(..))),
            "{label}"
        );
        assert!(Agent::start(&paths, fast()).is_err(), "{label}");
    }
    assert!(read_log(&paths).contains("ist ungültig"));

    // Gültig, mit BOM, Leerzeichen und / am Ende
    fs::write(
        paths.config(),
        "\u{feff}{\"server_url\": \" https://spedhub.example/ \", \"token\": \"abc123\", \"company_id\": 7}",
    )
    .unwrap();
    let c = Config::load(&paths.config()).unwrap();
    assert_eq!(c.server_url, "https://spedhub.example");
    assert_eq!(c.company_id, 7);
    // Der Token taucht in Debug-Ausgaben nicht auf
    assert!(!format!("{c:?}").contains("abc123"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn log_wird_rotiert_und_bleibt_klein() {
    let dir = temp_dir("log");
    let path = dir.join("plugin.log");
    let logger = Logger::new(&path, 2_000);
    for i in 0..500 {
        logger.write(&format!("Zeile {i} mit etwas Text\nund Umbruch"));
    }
    let current = fs::read_to_string(&path).unwrap();
    let old = fs::read_to_string(rotated(&path)).unwrap();
    assert!(current.len() <= 2_000 && old.len() <= 2_000);
    assert!(current.contains("Zeile 499 mit etwas Text und Umbruch"));
    assert!(
        current.lines().count() + old.lines().count() < 500,
        "alte Zeilen sind weg"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2, "nur zwei Dateien");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn postausgang_ueberlebt_kaputte_zeilen() {
    let dir = temp_dir("outbox");
    let paths = Paths::new(&dir);
    let entry = |n: u32| Entry {
        company_id: 1,
        booking: Booking {
            event_id: format!("x-1-{n}"),
            kind: "tollgate".into(),
            amount: 10,
            distance_km: None,
            note: "Maut".into(),
            occurred_at: Some(1_795_000_000 + i64::from(n)),
        },
    };
    let (mut outbox, notes) = Outbox::open(paths.outbox(), paths.rejected());
    assert!(notes.is_empty());
    outbox.push(entry(1)).unwrap();
    outbox.push(entry(2)).unwrap();
    // Halb geschriebene Zeile, zum Beispiel nach einem Absturz
    let mut text = fs::read_to_string(paths.outbox()).unwrap();
    text.push_str("{\"company_id\": 1, \"booking\": {\"event_\n");
    fs::write(paths.outbox(), text).unwrap();

    let (mut outbox, notes) = Outbox::open(paths.outbox(), paths.rejected());
    assert_eq!(outbox.entries(), &[entry(1), entry(2)]);
    assert!(notes.iter().any(|n| n.contains("kaputte Zeilen")));
    assert!(
        fs::read_to_string(paths.rejected())
            .unwrap()
            .contains("event_")
    );
    assert_eq!(read_entries(&paths.outbox()).len(), 2, "Datei bereinigt");

    outbox.remove_first(1).unwrap();
    assert_eq!(read_entries(&paths.outbox()), vec![entry(2)]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn zeitpunkt_wird_beim_annehmen_gesetzt_und_bleibt_im_postausgang() {
    let dir = temp_dir("zeitpunkt");
    let paths = Paths::new(&dir);
    // Ein Server, der nie erreichbar ist: alles bleibt im Postausgang
    write_config(&paths, "http://127.0.0.1:1", "token", 7);
    // Eine Zeile von einem älteren Plugin ohne Zeitpunkt liegt schon im Postausgang
    fs::write(
        paths.outbox(),
        r#"{"company_id":7,"booking":{"event_id":"alt-1-1","type":"tollgate","amount":5,"note":"Maut"}}"#
            .to_string()
            + "\n",
    )
    .unwrap();

    let before = spedhub_plugin::event::unix_now();
    let agent = Agent::start(&paths, fast()).unwrap();
    let toll = GameplayData {
        pay_amount: Some(40),
        ..data("player.tollgate.paid")
    };
    let id = match agent.submit(&toll) {
        spedhub_plugin::Submitted::Queued(id) => id,
        other => panic!("{other:?}"),
    };
    let after = spedhub_plugin::event::unix_now();
    wait_until("Event im Postausgang", || {
        read_entries(&paths.outbox()).len() == 2
    });
    agent.shutdown();

    // Neustart des Spiels: der Zeitpunkt ist noch da, die alte Zeile liest sich weiter
    let (outbox, _) = Outbox::open(paths.outbox(), paths.rejected());
    let entries = outbox.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].booking.event_id, "alt-1-1");
    assert_eq!(entries[0].booking.occurred_at, None);
    assert_eq!(entries[1].booking.event_id, id);
    let at = entries[1].booking.occurred_at.expect("Zeitpunkt fehlt");
    assert!(
        (before..=after).contains(&at),
        "{at} nicht in {before}..={after}"
    );
    let _ = fs::remove_dir_all(dir);
}
