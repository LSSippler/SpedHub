//! Senden an einen echten spedhub-server: Buchungen, Ausfall, Postausgang, erneutes Senden.
mod common;

use common::*;
use serde_json::Value;
use spedhub_plugin::outbox::{Entry, read_entries};
use spedhub_plugin::{Agent, Booking, GameplayData, Paths, Settings, Submitted};
use std::{
    fs,
    io::Read,
    net::TcpListener,
    time::{Duration, Instant},
};

fn all_events() -> Vec<GameplayData> {
    let d = |id: &str| GameplayData {
        id: id.into(),
        ..Default::default()
    };
    vec![
        GameplayData {
            revenue: Some(10_000),
            distance_km: Some(412.0),
            cargo_damage: Some(0.031),
            ..d("job.delivered")
        },
        GameplayData {
            cancel_penalty: Some(500),
            ..d("job.cancelled")
        },
        GameplayData {
            fine_amount: Some(300),
            fine_offence: Some("speeding".into()),
            ..d("player.fined")
        },
        GameplayData {
            pay_amount: Some(40),
            ..d("player.tollgate.paid")
        },
        GameplayData {
            pay_amount: Some(700),
            source_name: Some("Calais".into()),
            target_name: Some("Dover".into()),
            ..d("player.use.ferry")
        },
        GameplayData {
            pay_amount: Some(600),
            ..d("player.use.train")
        },
    ]
}

fn queued(s: Submitted) -> String {
    match s {
        Submitted::Queued(id) => id,
        other => panic!("erwartet Queued, bekommen {other:?}"),
    }
}

fn amounts(ledger: &[Value]) -> Vec<(String, String, i64)> {
    let mut v: Vec<_> = ledger
        .iter()
        .map(|e| {
            (
                e["account"].as_str().unwrap().to_string(),
                e["kind"].as_str().unwrap().to_string(),
                e["amount"].as_i64().unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn alle_events_kommen_beim_server_an() {
    let dir = temp_dir("senden");
    let server = TestServer::start(&dir.join("server.db"));
    let (token, company) = create_company(&server.url(), "Alex");
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &server.url(), &token, company);

    let agent = Agent::start(&paths, fast()).unwrap();
    let before = spedhub_plugin::event::unix_now();
    let mut ids = Vec::new();
    for e in all_events() {
        ids.push(queued(agent.submit(&e)));
    }
    let after = spedhub_plugin::event::unix_now();
    // Ein Event, das SpedHub nicht bucht, und eins ohne Betrag
    let other = GameplayData {
        id: "player.use.helicopter".into(),
        ..Default::default()
    };
    assert_eq!(agent.submit(&other), Submitted::Ignored);
    let broken = GameplayData {
        id: "job.delivered".into(),
        ..Default::default()
    };
    assert_eq!(agent.submit(&broken), Submitted::Invalid);

    wait_until("sechs Buchungen", || {
        ledger(&server.url(), &token, company).len() == 6
    });
    wait_until("Postausgang leer", || {
        read_entries(&paths.outbox()).is_empty()
    });
    agent.shutdown();

    // Auftrag: der ganze Erlös an die Firma. Der Rest nach den Buchungsregeln.
    let l = ledger(&server.url(), &token, company);
    assert_eq!(
        amounts(&l),
        vec![
            ("fahrer".into(), "auftrag_abgebrochen".into(), -500),
            ("fahrer".into(), "strafe".into(), -300),
            ("firma".into(), "auftrag".into(), 10_000),
            ("firma".into(), "faehre".into(), -700),
            ("firma".into(), "maut".into(), -40),
            ("firma".into(), "zug".into(), -600),
        ]
    );
    // Die Strecke kommt als eigenes Feld an und zählt als Firmenkilometer
    let job = l.iter().find(|e| e["kind"] == "auftrag").unwrap();
    assert_eq!(job["distance_km"], 412);
    assert_eq!(job["game_amount"], 10_000);
    assert!(
        l.iter()
            .filter(|e| e["kind"] != "auftrag")
            .all(|e| e["distance_km"].is_null())
    );
    // Jedes Event kommt mit dem Zeitpunkt an, zu dem das Plugin es angenommen hat
    for e in &l {
        let at = e["occurred_at"].as_i64().unwrap();
        assert!((before..=after).contains(&at), "{e}");
    }
    let notes: Vec<&str> = l.iter().map(|e| e["note"].as_str().unwrap()).collect();
    assert!(notes.contains(&"Auftrag abgeliefert, 412 km, Frachtschaden 3,1 %"));
    assert!(notes.contains(&"Strafe: Zu schnell"));
    assert!(notes.contains(&"Fähre Calais nach Dover"));
    let c = company_info(&server.url(), &token, company);
    assert_eq!(c["balance"], 10_000 - 700 - 40 - 600);
    assert_eq!(c["members"][0]["wallet"], -500 - 300);
    assert_eq!(c["members"][0]["company_km"], 412);

    let log = read_log(&paths);
    assert!(log.contains("Gesendet: job_delivered 10000 Euro"));
    assert!(log.contains("revenue fehlt"));
    assert!(!log.contains(&token), "kein Token im Log");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ohne_server_landet_alles_im_postausgang_und_geht_spaeter_raus() {
    let dir = temp_dir("ausfall");
    let db = dir.join("server.db");
    let paths = Paths::new(dir.join("SpedHub"));

    // Firma anlegen, dann den Server anhalten
    let server = TestServer::start(&db);
    let addr = server.addr;
    let url = server.url();
    let (token, company) = create_company(&url, "Bora");
    server.stop();
    write_config(&paths, &url, &token, company);

    // Server weg: Events landen im Postausgang, submit bleibt sofort
    let agent = Agent::start(&paths, fast()).unwrap();
    let started = Instant::now();
    let mut ids = Vec::new();
    for e in all_events() {
        ids.push(queued(agent.submit(&e)));
    }
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "submit darf nicht warten"
    );
    wait_until("sechs Events im Postausgang", || {
        read_entries(&paths.outbox()).len() == 6
    });
    wait_until("Ausfall im Log", || {
        read_log(&paths).contains("Events bleiben im Postausgang")
    });

    // Server kommt auf demselben Port zurück: der Agent schickt von selbst nach
    let server = TestServer::start_on(addr, &db);
    wait_until("Postausgang leer", || {
        read_entries(&paths.outbox()).is_empty()
    });
    assert_eq!(ledger(&url, &token, company).len(), 6);
    assert!(read_log(&paths).contains("Server wieder erreichbar"));
    agent.shutdown();
    drop(server);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn postausgang_ueberlebt_einen_neustart_des_spiels() {
    let dir = temp_dir("neustart");
    let db = dir.join("server.db");
    let paths = Paths::new(dir.join("SpedHub"));
    let server = TestServer::start(&db);
    let (addr, url) = (server.addr, server.url());
    let (token, company) = create_company(&url, "Cem");
    server.stop();
    write_config(&paths, &url, &token, company);

    // Erste Spielsitzung ohne Server, dann Spiel beenden
    let first = Agent::start(&paths, fast()).unwrap();
    let mut first_ids = Vec::new();
    for e in all_events().into_iter().take(3) {
        first_ids.push(queued(first.submit(&e)));
    }
    wait_until("drei Events im Postausgang", || {
        read_entries(&paths.outbox()).len() == 3
    });
    first.shutdown();
    // Der Zeitpunkt aus der ersten Sitzung steht im Postausgang
    let before = spedhub_plugin::event::unix_now();
    let mut waiting = read_entries(&paths.outbox());
    assert_eq!(waiting.len(), 3);
    for e in &waiting {
        let at = e.booking.occurred_at.unwrap();
        assert!(before - at < 60, "{e:?}");
    }
    // Die erste Sitzung liegt eine Stunde zurück. So kann am Server nicht zufällig der
    // Empfang passen, ankommen muss genau der Zeitpunkt aus dem Postausgang.
    let mut text = String::new();
    for e in &mut waiting {
        e.booking.occurred_at = Some(e.booking.occurred_at.unwrap() - 3600);
        text.push_str(&serde_json::to_string(e).unwrap());
        text.push('\n');
    }
    fs::write(paths.outbox(), text).unwrap();

    // Zweite Sitzung mit Server: alter Postausgang und neue Events gehen raus
    let server = TestServer::start_on(addr, &db);
    let second = Agent::start(&paths, fast()).unwrap();
    let mut second_ids = Vec::new();
    for e in all_events().into_iter().skip(3) {
        second_ids.push(queued(second.submit(&e)));
    }
    wait_until("alle sechs Events gebucht", || {
        ledger(&url, &token, company).len() == 6
    });
    second.shutdown();
    for id in &second_ids {
        assert!(
            !first_ids.contains(id),
            "{id} nach Neustart doppelt vergeben"
        );
    }
    assert!(read_entries(&paths.outbox()).is_empty());
    // Am Server zählt der Zeitpunkt aus der ersten Sitzung, nicht der Empfang
    let l = ledger(&url, &token, company);
    for w in &waiting {
        let id = w.booking.event_id.as_str();
        let e = l.iter().find(|e| e["event_id"] == id).unwrap();
        assert_eq!(e["occurred_at"].as_i64(), w.booking.occurred_at, "{e}");
        assert!(e["created_at"].as_i64().unwrap() >= before, "{e}");
    }
    // Die Events der zweiten Sitzung haben ihren eigenen Zeitpunkt
    for id in &second_ids {
        let e = l.iter().find(|e| e["event_id"] == id.as_str()).unwrap();
        assert!(e["occurred_at"].as_i64().unwrap() >= before, "{e}");
    }
    drop(server);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn erneutes_senden_bucht_nicht_doppelt_und_abgelehntes_bleibt_liegen() {
    let dir = temp_dir("doppelt");
    let server = TestServer::start(&dir.join("server.db"));
    let (token, company) = create_company(&server.url(), "Dana");
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &server.url(), &token, company);

    let agent = Agent::start(&paths, fast()).unwrap();
    let toll = &all_events()[3];
    let id = queued(agent.submit(toll));
    wait_until("gebucht", || {
        ledger(&server.url(), &token, company).len() == 1
    });
    agent.shutdown();

    // Dasselbe Event liegt noch einmal im Postausgang, zum Beispiel weil das Spiel abstürzte,
    // bevor der Postausgang aufgeräumt war. Dazu ein Event, das der Server nie annimmt.
    let again = Entry {
        company_id: company,
        booking: Booking {
            event_id: id.clone(),
            kind: "tollgate".into(),
            amount: 40,
            distance_km: None,
            note: "Maut".into(),
            occurred_at: None,
        },
    };
    let bogus = Entry {
        company_id: company,
        booking: Booking {
            event_id: "kaputt-1".into(),
            kind: "geschenk".into(),
            amount: 5,
            distance_km: None,
            note: String::new(),
            occurred_at: None,
        },
    };
    let lines: Vec<String> = [&again, &bogus]
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect();
    fs::write(paths.outbox(), lines.join("\n") + "\n").unwrap();

    let agent = Agent::start(&paths, fast()).unwrap();
    wait_until("Postausgang leer", || {
        read_entries(&paths.outbox()).is_empty()
    });
    agent.shutdown();
    assert_eq!(
        ledger(&server.url(), &token, company).len(),
        1,
        "nicht doppelt"
    );
    let log = read_log(&paths);
    assert!(log.contains("war schon gebucht"));
    assert!(log.contains("Server lehnt kaputt-1 ab"));
    let rejected = fs::read_to_string(paths.rejected()).unwrap();
    assert!(rejected.contains("kaputt-1") && rejected.contains("Unbekannter Ereignistyp"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ein_haengender_server_bremst_weder_events_noch_das_beenden() {
    let dir = temp_dir("haengt");
    // Nimmt Verbindungen an und antwortet nie
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hold = std::thread::spawn(move || {
        let mut open = Vec::new();
        for stream in listener.incoming().take(3) {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            open.push(s);
        }
        std::thread::sleep(Duration::from_secs(3));
    });
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, "token", 1);
    let settings = Settings {
        request_timeout: Duration::from_millis(500),
        ..fast()
    };
    let agent = Agent::start(&paths, settings).unwrap();
    let started = Instant::now();
    for e in all_events() {
        queued(agent.submit(&e));
    }
    assert!(started.elapsed() < Duration::from_millis(200));
    wait_until("sechs Events im Postausgang", || {
        read_entries(&paths.outbox()).len() == 6
    });
    let stopping = Instant::now();
    agent.shutdown();
    assert!(
        stopping.elapsed() < Duration::from_secs(2),
        "Beenden dauerte {:?}",
        stopping.elapsed()
    );
    assert_eq!(read_entries(&paths.outbox()).len(), 6, "nichts verloren");
    drop(hold);
    let _ = fs::remove_dir_all(dir);
}
