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
            ..Default::default()
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

// ------------------------------------------------------------------- Herzschlag

use spedhub_plugin::telemetry::{JobInfo, Snapshot, readable_plate};

#[test]
fn kennzeichen_aus_dem_spiel_lesbar_machen() {
    assert_eq!(readable_plate("SPED 001"), "SPED 001");
    assert_eq!(readable_plate("SPED 001|_germany"), "SPED 001");
    assert_eq!(
        readable_plate(
            "DD<offset hshift=4 vshift=-5><img src=/material/ui/lp/germany/dresden.mat color=FFFFFFFF><offset hshift=4 vshift=5>RI 772"
        ),
        "DD RI 772"
    );
    assert_eq!(readable_plate("<img src=/a.mat>"), "");
    assert_eq!(readable_plate(&"A".repeat(100)).len(), 64);
}

#[test]
fn herzschlag_nur_wenn_das_spiel_laeuft_und_ein_kennzeichen_bekannt_ist() {
    // Am Anfang pausiert, bis das SDK `started` meldet
    let mut s = Snapshot::default();
    assert!(s.paused);
    s.set_truck(Some("SPED 001"), Some(800.0));
    assert_eq!(s.heartbeat(), None, "pausiert");
    s.paused = false;
    s.set_truck(Some("  "), None);
    assert_eq!(s.heartbeat(), None, "ohne Kennzeichen");

    s.set_truck(Some("EF<img src=/lp.mat> XU 577|germany"), Some(800.0));
    s.odometer_km = Some(12_345.5);
    s.wear_engine = Some(0.05);
    s.wear_transmission = Some(1.2);
    s.wear_cabin = Some(f32::NAN);
    s.wear_wheels = Some(-0.1);
    s.fuel_liters = Some(200.0);
    s.speed_ms = Some(-10.0);
    let hb = s.heartbeat().unwrap();
    assert_eq!(hb.plate, "EF XU 577");
    assert_eq!(hb.odometer_km, Some(12_345.5));
    let wear = hb.wear.clone().unwrap();
    assert!((wear.engine.unwrap() - 0.05).abs() < 1e-6);
    assert_eq!(wear.transmission, Some(1.0), "auf 1 begrenzt");
    assert_eq!(wear.cabin, None, "NaN fällt weg");
    assert_eq!(wear.chassis, None);
    assert_eq!(wear.wheels, Some(0.0));
    assert_eq!(hb.fuel_liters, Some(200.0));
    assert_eq!(hb.fuel_relative, Some(0.25));
    assert!(
        (hb.speed_kmh.unwrap() - 36.0).abs() < 1e-4,
        "rückwärts zählt positiv"
    );
    assert_eq!(hb.near_city, None, "gibt das SDK nicht her");
    // So geht er an den Server: ohne Stadt, Auftrag ausdrücklich null
    let v = serde_json::to_value(&hb).unwrap();
    assert!(v.get("near_city").is_none());
    assert!(v["job"].is_null());
    assert!(v["wear"].get("cabin").is_none());

    // Ohne Tankgröße kein relativer Tank, ohne Werte kein Verschleiß
    let mut t = Snapshot {
        paused: false,
        ..Default::default()
    };
    t.set_truck(Some("SPED 001"), Some(0.0));
    t.fuel_liters = Some(100.0);
    let hb = t.heartbeat().unwrap();
    assert_eq!(
        (hb.fuel_relative, hb.wear, hb.speed_kmh),
        (None, None, None)
    );

    // Pause und Weiterfahren
    s.paused = true;
    assert_eq!(s.heartbeat(), None);
}

#[test]
fn letzter_auftrag_bleibt_bis_zur_buchung() {
    let mut s = Snapshot::default();
    let job = JobInfo::new(
        Some(" Holz\nbretter ".into()),
        Some("Berlin".into()),
        Some("Hamburg".into()),
    );
    assert_eq!(job.cargo.as_deref(), Some("Holz bretter"));
    s.set_job(Some(job.clone()));
    assert_eq!(s.job, Some(job.clone()));
    // Das SDK leert die Job-Konfiguration, zum Beispiel vor dem Event job.delivered
    s.set_job(None);
    assert_eq!(s.job, None);
    assert_eq!(
        s.last_job,
        Some(job.clone()),
        "der letzte nicht leere bleibt"
    );
    // Ein Auftrag ohne Angaben zählt als keiner
    s.set_job(Some(JobInfo::new(Some(" ".into()), None, None)));
    assert_eq!(s.job, None);
    assert_eq!(s.last_job, Some(job));
}

#[test]
fn alte_und_neue_zeilen_im_postausgang_mit_fracht() {
    // Eine Zeile von vor dem Herzschlag: ohne Fracht und Städte
    let old: Entry = serde_json::from_str(
        r#"{"company_id":7,"booking":{"event_id":"alt-1-1","type":"job_delivered","amount":5,"distance_km":12,"note":"x","occurred_at":1795000000}}"#,
    )
    .unwrap();
    assert_eq!(old.booking.cargo, None);
    assert_eq!(old.booking.target_city, None);
    // Ohne Fracht wird auch nichts davon geschrieben
    let line = serde_json::to_string(&old).unwrap();
    assert!(!line.contains("cargo") && !line.contains("city"), "{line}");

    // Mit Fracht: hin und zurück
    let new = Booking {
        event_id: "neu-1-1".into(),
        kind: "job_delivered".into(),
        amount: 100,
        ..Default::default()
    }
    .with_job(Some(JobInfo::new(
        Some("Holz".into()),
        Some("Berlin".into()),
        None,
    )));
    let line = serde_json::to_string(&new).unwrap();
    assert!(line.contains(r#""cargo":"Holz""#), "{line}");
    assert!(!line.contains("target_city"), "{line}");
    let back: Booking = serde_json::from_str(&line).unwrap();
    assert_eq!(back, new);
    assert_eq!(back.source_city.as_deref(), Some("Berlin"));
}

// ------------------------------------------------------------------- Profil aus game.log.txt

use spedhub_plugin::profile::{LogReader, last_profile, last_profile_bytes, profile_in_line};
use spedhub_plugin::{ProfileGate, ProfileState, Submitted};

#[test]
fn profilname_aus_dem_logtext() {
    // So sieht die Zeile im Spiel aus
    assert_eq!(
        profile_in_line("00:00:12.345 : New profile selected: 'Nordfracht'").as_deref(),
        Some("Nordfracht")
    );
    // Mehrere Zeilen, Profilwechsel: die letzte zählt
    let text = "00:00:00.000 : log created\n\
                00:00:12.345 : New profile selected: 'Privat'\n\
                00:01:00.000 : [sys] loading save\n\
                00:15:00.000 : New profile selected: 'Nordfracht'\n\
                00:16:00.000 : [sys] autosave\n";
    assert_eq!(last_profile(text).as_deref(), Some("Nordfracht"));
    let back = format!("{text}00:30:00.000 : New profile selected: 'Privat'\n");
    assert_eq!(last_profile(&back).as_deref(), Some("Privat"));
    // Umlaute, Leerzeichen, Anführungszeichen im Namen, Zeilenende von Windows
    assert_eq!(
        last_profile("x : New profile selected: 'Müller Logistik GmbH'\r\n").as_deref(),
        Some("Müller Logistik GmbH")
    );
    assert_eq!(
        last_profile("New profile selected: 'Bob's Trucks'").as_deref(),
        Some("Bob's Trucks")
    );
    // Beliebiges Präfix, auch keins, Leerraum vor dem Namen
    assert_eq!(
        last_profile("[2026-11-18] <info> New profile selected:   'A B'").as_deref(),
        Some("A B")
    );
    // Keine Zeile, kaputte Zeilen, leerer Name: kein Profil
    assert_eq!(last_profile(""), None);
    assert_eq!(last_profile("00:00:00.000 : log created\n"), None);
    assert_eq!(last_profile("New profile selected: Nordfracht\n"), None);
    assert_eq!(last_profile("New profile selected: 'Nordfracht\n"), None);
    assert_eq!(last_profile("New profile selected: ''\n"), None);
    assert_eq!(last_profile("New profile selected: '  '\n"), None);
    // Eine kaputte Zeile danach ändert nichts an der letzten gültigen
    assert_eq!(
        last_profile("New profile selected: 'A'\nNew profile selected: 'B\n").as_deref(),
        Some("A")
    );
    // Kaputte Bytes: nicht panisch, der Rest liest sich weiter
    let mut bytes = b"\xff\xfe00:00 : New profile selected: 'Nordfracht'\n\xc3".to_vec();
    assert_eq!(last_profile_bytes(&bytes).as_deref(), Some("Nordfracht"));
    bytes.extend_from_slice(b"New profile selected: 'M\xc3\xbcller \xff'\n");
    assert_eq!(
        last_profile_bytes(&bytes).as_deref(),
        Some("Müller \u{fffd}")
    );
}

#[test]
fn log_wird_stueckweise_gelesen_und_nach_neuem_spielstart_von_vorn() {
    let dir = temp_dir("gamelog");
    let path = dir.join("game.log.txt");
    let mut reader = LogReader::new(&path);
    // Noch keine Datei: unbekannt
    assert_eq!(reader.poll(), None);

    let append = |text: &[u8]| {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .unwrap();
        f.write_all(text).unwrap();
    };
    append(b"00:00:00.000 : log created on Wednesday\n00:00:01.000 : [sys] init\n");
    assert_eq!(reader.poll(), None);
    let first = fs::metadata(&path).unwrap().len();
    assert_eq!(reader.position(), first);

    append(b"00:00:12.345 : New profile selected: 'Privat'\n");
    assert_eq!(reader.poll().as_deref(), Some("Privat"));
    // Nichts Neues: dasselbe, ohne neu zu lesen
    let pos = reader.position();
    assert_eq!(reader.poll().as_deref(), Some("Privat"));
    assert_eq!(reader.position(), pos);

    // Eine halbe Zeile: zählt, sobald der Name vollständig ist
    append(b"00:20:00.000 : New profile selected: 'Nord");
    assert_eq!(reader.poll().as_deref(), Some("Privat"));
    append(b"fracht'");
    assert_eq!(reader.poll().as_deref(), Some("Nordfracht"));
    append(b"\n00:21:00.000 : [sys] autosave\n");
    assert_eq!(reader.poll().as_deref(), Some("Nordfracht"));
    // Profilwechsel im laufenden Spiel
    append("00:40:00.000 : New profile selected: 'Müller'\n".as_bytes());
    assert_eq!(reader.poll().as_deref(), Some("Müller"));
    let long = reader.position();

    // Neuer Spielstart: Das Spiel schreibt die Datei neu, sie ist erst einmal kürzer
    fs::write(&path, b"00:00:00.000 : log created on Thursday\n").unwrap();
    assert_eq!(reader.poll(), None, "das alte Profil gilt nicht mehr");
    assert!(reader.position() < long);
    append(b"00:00:09.000 : New profile selected: 'Nordfracht'\n");
    assert_eq!(reader.poll().as_deref(), Some("Nordfracht"));

    // Neu geschrieben und schon länger als die gemerkte Stelle: der Anfang hat sich geändert
    let mut text = b"00:00:00.000 : log created on Friday!\n".to_vec();
    text.extend_from_slice(&vec![b'.'; 300]);
    text.extend_from_slice(b"\n00:00:05.000 : [sys] no profile yet\n");
    fs::write(&path, &text).unwrap();
    assert_eq!(reader.poll(), None);
    assert_eq!(reader.position(), text.len() as u64);

    // Datei weg: unbekannt, und danach geht es wieder von vorn
    fs::remove_file(&path).unwrap();
    assert_eq!(reader.poll(), None);
    append(b"00:00:01.000 : New profile selected: 'Nordfracht'\n");
    assert_eq!(reader.poll().as_deref(), Some("Nordfracht"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn firmenprofil_mit_dem_aktiven_profil_vergleichen() {
    let dir = temp_dir("tor");
    let log = dir.join("game.log.txt");
    // Ohne Firmenprofil in plugin.json: nichts geht raus
    let (state, note) = ProfileGate::new(None, Some(&log)).check();
    assert_eq!(state, ProfileState::NotConfigured);
    assert!(
        note.unwrap()
            .contains("Launcher muss die Konfiguration neu schreiben")
    );
    assert_eq!(
        ProfileGate::new(Some("  "), Some(&log)).check().0,
        ProfileState::NotConfigured
    );
    // Ohne Pfad zum Log oder ohne Log: unbekannt
    assert_eq!(
        ProfileGate::new(Some("Nordfracht"), None).check().0,
        ProfileState::Unknown
    );
    let mut gate = ProfileGate::new(Some(" Nordfracht "), Some(&log));
    assert_eq!(gate.company(), Some("Nordfracht"));
    let (state, note) = gate.check();
    assert_eq!(state, ProfileState::Unknown);
    assert!(note.unwrap().contains("unbekannt"));
    // Gleicher Zustand: keine neue Zeile fürs Log
    assert_eq!(gate.check(), (ProfileState::Unknown, None));

    fs::write(&log, "00:00 : New profile selected: 'Nordfracht'\n").unwrap();
    let (state, note) = gate.check();
    assert_eq!(state, ProfileState::Company("Nordfracht".into()));
    assert_eq!(state.company(), Some("Nordfracht"));
    assert!(note.unwrap().contains("ist das Firmenprofil"));

    // Groß- und Kleinschreibung zählt: anderes Profil
    let mut f = fs::OpenOptions::new().append(true).open(&log).unwrap();
    use std::io::Write;
    writeln!(f, "00:10 : New profile selected: 'nordfracht'").unwrap();
    let (state, note) = gate.check();
    assert_eq!(state, ProfileState::Other("nordfracht".into()));
    assert_eq!(state.company(), None);
    let note = note.unwrap();
    assert!(
        note.contains("'nordfracht'") && note.contains("'Nordfracht'"),
        "{note}"
    );
    writeln!(f, "00:20 : New profile selected: 'Nordfracht'").unwrap();
    assert_eq!(gate.check().0, ProfileState::Company("Nordfracht".into()));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn konfiguration_mit_firmenprofil_und_alte_ohne() {
    let dir = temp_dir("config-profil");
    let paths = Paths::new(&dir);
    write_config(&paths, "http://127.0.0.1:1", "abc", 3);
    let c = Config::load(&paths.config()).unwrap();
    assert_eq!(c.profile_name.as_deref(), Some(COMPANY_PROFILE));
    assert_eq!(c.game_log.as_deref(), Some(game_log(&paths).as_path()));
    // Leere Werte zählen wie fehlende
    fs::write(
        paths.config(),
        r#"{"server_url":"http://x","token":"t","company_id":1,"profile_name":" ","game_log":""}"#,
    )
    .unwrap();
    let c = Config::load(&paths.config()).unwrap();
    assert_eq!((c.profile_name, c.game_log), (None, None));
    // Von einem älteren Launcher: lädt, aber ohne Firmenprofil
    write_old_config(&paths, "http://127.0.0.1:1", "abc", 3);
    let c = Config::load(&paths.config()).unwrap();
    assert_eq!((c.profile_name, c.game_log), (None, None));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ereignis_aus_anderem_oder_unbekanntem_profil_kommt_nicht_in_den_postausgang() {
    let dir = temp_dir("fremdes-profil");
    let paths = Paths::new(&dir);
    // Ein Server, der nie erreichbar ist: Angenommenes bleibt im Postausgang
    write_config(&paths, "http://127.0.0.1:1", "token", 7);
    let agent = Agent::start(&paths, fast()).unwrap();
    let toll = GameplayData {
        pay_amount: Some(40),
        ..data("player.tollgate.paid")
    };
    // Firmenprofil: angenommen, mit Profil
    assert!(matches!(agent.submit(&toll), Submitted::Queued(_)));
    wait_until("ein Event im Postausgang", || {
        read_entries(&paths.outbox()).len() == 1
    });
    // Privates Profil: nicht angenommen
    select_profile(&paths, "Privat");
    assert_eq!(
        agent.submit(&toll),
        Submitted::OtherProfile("Privat".into())
    );
    // Ein Event, das SpedHub nicht bucht, bleibt einfach ignoriert
    assert_eq!(
        agent.submit(&data("player.use.helicopter")),
        Submitted::Ignored
    );
    // Log weg (zum Beispiel falscher Pfad): Profil unbekannt
    fs::remove_file(game_log(&paths)).unwrap();
    assert_eq!(agent.submit(&toll), Submitted::UnknownProfile);
    // Log da, aber noch kein Profil gewählt
    fs::write(game_log(&paths), "00:00:00.000 : log created\n").unwrap();
    assert_eq!(agent.submit(&toll), Submitted::UnknownProfile);
    // Zurück ins Firmenprofil
    select_profile(&paths, COMPANY_PROFILE);
    assert!(matches!(agent.submit(&toll), Submitted::Queued(_)));
    wait_until("zwei Events im Postausgang", || {
        read_entries(&paths.outbox()).len() == 2
    });
    wait_until("Log", || {
        read_log(&paths).contains("aktives Profil unbekannt")
    });
    agent.shutdown();

    let entries = read_entries(&paths.outbox());
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|e| e.booking.profile.as_deref() == Some(COMPANY_PROFILE))
    );
    let log = read_log(&paths);
    assert!(
        log.contains("Ereignis nicht angenommen: tollgate 40 Euro, aus Profil 'Privat', Firmenprofil ist 'Nordfracht'"),
        "{log}"
    );
    assert!(log.contains("Aktives Profil 'Privat' ist nicht das Firmenprofil"));
    assert!(!log.contains("token\""), "kein Token im Log");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn alte_plugin_json_ohne_firmenprofil_sendet_nichts() {
    let dir = temp_dir("alte-config");
    let paths = Paths::new(&dir);
    write_old_config(&paths, "http://127.0.0.1:1", "token", 7);
    // Selbst wenn das Spiel ein Profil meldet: ohne Firmenprofil weiß das Plugin nicht, ob es
    // das richtige ist
    fs::write(
        dir.join("game.log.txt"),
        "New profile selected: 'Nordfracht'\n",
    )
    .unwrap();
    // Ein Event von früher wartet im Postausgang
    fs::write(
        paths.outbox(),
        r#"{"company_id":7,"booking":{"event_id":"alt-1-1","type":"tollgate","amount":5,"note":"Maut"}}"#
            .to_string()
            + "\n",
    )
    .unwrap();
    let agent = Agent::start(&paths, fast()).unwrap();
    let toll = GameplayData {
        pay_amount: Some(40),
        ..data("player.tollgate.paid")
    };
    assert_eq!(agent.submit(&toll), Submitted::NoCompanyProfile);
    agent.telemetry().update(|s| {
        s.set_truck(Some("SPED 001"), None);
        s.paused = false;
    });
    wait_until("drei Takte", || agent.heartbeat_stats().blocked >= 3);
    assert_eq!(agent.heartbeat_stats().attempts, 0);
    agent.shutdown();
    // Der Postausgang ist unverändert, es wurde nichts versucht
    assert_eq!(read_entries(&paths.outbox()).len(), 1);
    let log = read_log(&paths);
    assert!(
        log.contains("Der Launcher muss die Konfiguration neu schreiben"),
        "{log}"
    );
    assert!(log.contains("plugin.json nennt kein Firmenprofil"));
    assert!(!log.contains("Server nicht erreichbar"), "{log}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn alte_zeilen_im_postausgang_ohne_profil() {
    let old: Entry = serde_json::from_str(
        r#"{"company_id":7,"booking":{"event_id":"alt-1-1","type":"tollgate","amount":5,"note":"Maut","occurred_at":1795000000}}"#,
    )
    .unwrap();
    assert_eq!(old.booking.profile, None);
    let line = serde_json::to_string(&old).unwrap();
    assert!(!line.contains("profile"), "{line}");
    // Mit Profil: hin und zurück
    let new = Booking {
        event_id: "neu-1-1".into(),
        kind: "tollgate".into(),
        amount: 5,
        ..Default::default()
    }
    .with_profile("Müller Logistik");
    let line = serde_json::to_string(&new).unwrap();
    assert!(line.contains(r#""profile":"Müller Logistik""#), "{line}");
    assert_eq!(serde_json::from_str::<Booking>(&line).unwrap(), new);
}

// ------------------------------------------------------------------- Marke und Modell

#[test]
fn marke_und_modell_gehen_mit_dem_herzschlag() {
    let mut s = Snapshot {
        paused: false,
        ..Default::default()
    };
    s.set_truck(Some("SPED 001|_germany"), Some(800.0));
    // Noch keine Marke und kein Modell: Die Felder fehlen im Herzschlag
    let hb = s.heartbeat().unwrap();
    assert_eq!(
        (hb.truck_brand.as_deref(), hb.truck_model.as_deref()),
        (None, None)
    );
    let v = serde_json::to_value(&hb).unwrap();
    assert!(
        v.get("truck_brand").is_none() && v.get("truck_model").is_none(),
        "{v}"
    );

    // Wie aus der Konfiguration `truck`: brand_id und id, unverändert bis auf Leerraum
    s.set_truck_model(Some(" scania "), Some("vehicle.scania.s_2016"));
    let hb = s.heartbeat().unwrap();
    assert_eq!(hb.truck_brand.as_deref(), Some("scania"));
    assert_eq!(hb.truck_model.as_deref(), Some("vehicle.scania.s_2016"));
    let v = serde_json::to_value(&hb).unwrap();
    assert_eq!(v["truck_brand"], "scania");
    assert_eq!(v["truck_model"], "vehicle.scania.s_2016");
    assert_eq!(
        serde_json::from_value::<spedhub_plugin::Heartbeat>(v).unwrap(),
        hb
    );

    // Leere Konfiguration (kein Lkw): beides weg
    s.set_truck_model(Some(""), None);
    assert_eq!((s.truck_brand.clone(), s.truck_model.clone()), (None, None));
}

#[test]
fn marke_und_modell_gehen_mit_der_buchung_und_alte_zeilen_lesen_sich() {
    let dir = temp_dir("modell");
    let paths = Paths::new(&dir);
    // Server nie erreichbar: alles bleibt im Postausgang
    write_config(&paths, "http://127.0.0.1:1", "token", 7);
    // Eine Zeile von einem älteren Plugin ohne Marke und Modell
    fs::write(
        paths.outbox(),
        r#"{"company_id":7,"booking":{"event_id":"alt-1-1","type":"tollgate","amount":5,"note":"Maut","occurred_at":1795000000,"profile":"Nordfracht"}}"#
            .to_string()
            + "\n",
    )
    .unwrap();
    let agent = Agent::start(&paths, fast()).unwrap();
    agent
        .telemetry()
        .update(|s| s.set_truck_model(Some("scania"), Some("s_2016")));
    let toll = GameplayData {
        pay_amount: Some(40),
        ..data("player.tollgate.paid")
    };
    assert!(matches!(
        agent.submit(&toll),
        spedhub_plugin::Submitted::Queued(_)
    ));
    // Ein anderer Lkw: die nächste Buchung trägt dessen Modell
    agent
        .telemetry()
        .update(|s| s.set_truck_model(Some("volvo"), Some("volvo.fh16_2012")));
    assert!(matches!(
        agent.submit(&toll),
        spedhub_plugin::Submitted::Queued(_)
    ));
    wait_until("Events im Postausgang", || {
        read_entries(&paths.outbox()).len() == 3
    });
    agent.shutdown();

    let (outbox, _) = Outbox::open(paths.outbox(), paths.rejected());
    let e = outbox.entries();
    assert_eq!(e[0].booking.event_id, "alt-1-1");
    assert_eq!(
        (
            e[0].booking.truck_brand.clone(),
            e[0].booking.truck_model.clone()
        ),
        (None, None)
    );
    assert_eq!(e[1].booking.truck_brand.as_deref(), Some("scania"));
    assert_eq!(e[1].booking.truck_model.as_deref(), Some("s_2016"));
    assert_eq!(e[2].booking.truck_model.as_deref(), Some("volvo.fh16_2012"));
    // Die alte Zeile schreibt sich ohne die Felder, die neue mit
    let line = serde_json::to_string(&e[0]).unwrap();
    assert!(!line.contains("truck_"), "{line}");
    let line = serde_json::to_string(&e[1]).unwrap();
    assert!(line.contains(r#""truck_model":"s_2016""#), "{line}");
    assert_eq!(serde_json::from_str::<Entry>(&line).unwrap(), e[1]);
    let _ = fs::remove_dir_all(dir);
}
