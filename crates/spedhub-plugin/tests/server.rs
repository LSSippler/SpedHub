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
            ..Default::default()
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
            ..Default::default()
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

// ------------------------------------------------------------------- Herzschlag

use spedhub_plugin::JobInfo;
use std::{
    io::Write,
    net::TcpStream,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Das Spiel läuft mit einem Lkw: Kennzeichen bekannt, nicht pausiert.
fn driving(agent: &Agent, plate: &str) {
    agent.telemetry().update(|s| {
        s.set_truck(Some(plate), Some(800.0));
        s.odometer_km = Some(1234.5);
        s.speed_ms = Some(23.0);
        s.fuel_liters = Some(400.0);
        s.wear_engine = Some(0.1);
        s.paused = false;
    });
}

fn member(url: &str, token: &str, company: i64) -> Value {
    company_info(url, token, company)["members"][0].clone()
}

#[test]
fn herzschlag_kommt_beim_server_an_und_zeigt_online() {
    let dir = temp_dir("herzschlag");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let (token, company) = create_company(&url, "Emil");
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, &token, company);

    let agent = Agent::start(&paths, fast()).unwrap();
    assert_eq!(member(&url, &token, company)["online"], false);
    driving(&agent, "EF<img src=/lp.mat> XU 577|germany");
    agent.telemetry().update(|s| {
        s.set_job(Some(JobInfo::new(
            Some("Holz".into()),
            Some("Berlin".into()),
            Some("Hamburg".into()),
        )))
    });
    wait_until("online", || member(&url, &token, company)["online"] == true);
    let m = member(&url, &token, company);
    assert_eq!(m["plate"], "EF XU 577");
    let speed = m["speed_kmh"].as_f64().unwrap();
    assert!((speed - 82.8).abs() < 0.01, "{speed}");
    assert_eq!(
        m["job"],
        serde_json::json!({ "cargo": "Holz", "source_city": "Berlin", "target_city": "Hamburg" })
    );
    assert!(m["near_city"].is_null(), "gibt das SDK nicht her");
    assert!(agent.heartbeat_stats().delivered >= 1);

    // Herzschläge kommen nie in den Postausgang
    assert!(read_entries(&paths.outbox()).is_empty());
    agent.shutdown();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn pausiert_geht_kein_herzschlag_raus() {
    let dir = temp_dir("pause");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let (token, company) = create_company(&url, "Finn");
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, &token, company);

    let agent = Agent::start(&paths, fast()).unwrap();
    // Kennzeichen bekannt, aber das Spiel steht noch in der Pause (Anfangszustand)
    agent
        .telemetry()
        .update(|s| s.set_truck(Some("SPED 001"), None));
    wait_until("fünf Takte", || agent.heartbeat_stats().ticks >= 5);
    assert_eq!(agent.heartbeat_stats().attempts, 0);
    assert!(member(&url, &token, company)["last_seen"].is_null());

    // Weiterfahren: Herzschläge gehen raus
    agent.telemetry().set_paused(false);
    wait_until("online", || member(&url, &token, company)["online"] == true);

    // Wieder Pause: keine weiteren Versuche
    agent.telemetry().set_paused(true);
    let paused_at = agent.heartbeat_stats();
    wait_until("fünf weitere Takte", || {
        agent.heartbeat_stats().ticks >= paused_at.ticks + 5
    });
    // Ein Takt kann gerade gelaufen sein, als die Pause kam
    assert!(agent.heartbeat_stats().attempts <= paused_at.attempts + 1);
    agent.shutdown();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ohne_server_verfaellt_der_herzschlag_und_kommt_nicht_in_den_postausgang() {
    let dir = temp_dir("herzschlag-weg");
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &format!("http://{}", free_addr()), "token", 1);
    let agent = Agent::start(&paths, fast()).unwrap();
    driving(&agent, "SPED 001");
    wait_until("drei Versuche", || agent.heartbeat_stats().attempts >= 3);
    assert_eq!(agent.heartbeat_stats().delivered, 0);
    agent.shutdown();
    assert!(read_entries(&paths.outbox()).is_empty());
    assert!(!paths.outbox().exists(), "kein Postausgang angelegt");
    // Nur der erste Fehler steht im Log
    let log = read_log(&paths);
    assert_eq!(
        log.matches("Herzschlag nicht angekommen").count(),
        1,
        "{log}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// Ein Server, der Herzschläge annimmt und nie beantwortet, Buchungen aber sofort.
fn half_hanging_server() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let events = Arc::new(AtomicUsize::new(0));
    let beats = Arc::new(AtomicUsize::new(0));
    let (e, b) = (events.clone(), beats.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let (e, b) = (e.clone(), b.clone());
            std::thread::spawn(move || serve(stream, &e, &b));
        }
    });
    (url, events, beats)
}

fn serve(mut s: TcpStream, events: &AtomicUsize, beats: &AtomicUsize) {
    loop {
        // Kopf lesen
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match s.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => return,
            }
        }
        let head = String::from_utf8_lossy(&head).to_lowercase();
        let len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        if s.read_exact(&mut body).is_err() {
            return;
        }
        if head.contains("/heartbeat") {
            beats.fetch_add(1, Ordering::SeqCst);
            // Hängt, bis der Client aufgibt
            std::thread::sleep(Duration::from_secs(5));
            return;
        }
        events.fetch_add(1, Ordering::SeqCst);
        let answer = r#"{"duplicate":false}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{answer}",
            answer.len()
        );
        if s.write_all(reply.as_bytes()).is_err() {
            return;
        }
    }
}

#[test]
fn haengende_herzschlaege_bremsen_keine_buchungen() {
    let dir = temp_dir("herzschlag-haengt");
    let (url, events, beats) = half_hanging_server();
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, "token", 1);
    let settings = Settings {
        request_timeout: Duration::from_millis(2000),
        ..fast()
    };
    let agent = Agent::start(&paths, settings).unwrap();
    driving(&agent, "SPED 001");
    // Der Herzschlag-Thread hängt jetzt in einer Anfrage
    wait_until("ein Herzschlag unterwegs", || {
        beats.load(Ordering::SeqCst) >= 1
    });
    let started = Instant::now();
    for e in all_events() {
        queued(agent.submit(&e));
    }
    wait_until("sechs Buchungen beim Server", || {
        events.load(Ordering::SeqCst) == 6
    });
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "Buchungen warteten {:?}",
        started.elapsed()
    );
    wait_until("Postausgang leer", || {
        read_entries(&paths.outbox()).is_empty()
    });
    assert_eq!(agent.heartbeat_stats().delivered, 0);
    // Beenden wartet höchstens eine Anfrage lang
    let stopping = Instant::now();
    agent.shutdown();
    assert!(
        stopping.elapsed() < Duration::from_millis(3000),
        "Beenden dauerte {:?}",
        stopping.elapsed()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn abgelieferter_auftrag_traegt_fracht_und_staedte() {
    let dir = temp_dir("fracht");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let (token, company) = create_company(&url, "Gina");
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, &token, company);
    let agent = Agent::start(&paths, fast()).unwrap();
    let job = || {
        Some(JobInfo::new(
            Some("Holzbretter".into()),
            Some("Berlin".into()),
            Some("Hamburg".into()),
        ))
    };
    let delivered = all_events().remove(0);
    let cancelled = all_events().remove(1);

    // Das SDK leert die Job-Konfiguration vor dem Event: Der letzte Auftrag gilt trotzdem
    agent.telemetry().update(|s| s.set_job(job()));
    agent.telemetry().update(|s| s.set_job(None));
    let first = queued(agent.submit(&delivered));
    // Ein zweites Event ohne neue Job-Konfiguration hat keine Fracht
    let second = queued(agent.submit(&delivered));
    // Abgebrochen: Der Auftrag ist erledigt, das nächste abgelieferte Event hat keine Fracht
    agent.telemetry().update(|s| s.set_job(job()));
    queued(agent.submit(&cancelled));
    let third = queued(agent.submit(&delivered));
    // Und wenn die Konfiguration erst nach dem Event geleert wird
    agent.telemetry().update(|s| s.set_job(job()));
    let fourth = queued(agent.submit(&delivered));
    agent.telemetry().update(|s| s.set_job(None));

    wait_until("fünf Buchungen", || {
        ledger(&url, &token, company).len() == 5
    });
    agent.shutdown();
    let l = ledger(&url, &token, company);
    let by = |id: &str| l.iter().find(|e| e["event_id"] == id).unwrap().clone();
    for id in [&first, &fourth] {
        let e = by(id);
        assert_eq!(e["cargo"], "Holzbretter", "{e}");
        assert_eq!(e["source_city"], "Berlin");
        assert_eq!(e["target_city"], "Hamburg");
    }
    for id in [&second, &third] {
        let e = by(id);
        assert!(e["cargo"].is_null() && e["target_city"].is_null(), "{e}");
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn version_geht_im_header_mit() {
    // Ein Server, der nur den Kopf der Anfrage festhält und mit 200 antwortet
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") && s.read(&mut byte).unwrap_or(0) == 1 {
            head.push(byte[0]);
        }
        let answer = r#"{"duplicate":false}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{answer}",
            answer.len()
        );
        let _ = s.write_all(reply.as_bytes());
        tx.send(String::from_utf8_lossy(&head).to_lowercase())
            .unwrap();
    });
    let config = spedhub_plugin::Config {
        server_url: url,
        token: "t".repeat(64),
        company_id: 1,
        profile_name: Some(COMPANY_PROFILE.into()),
        game_log: None,
    };
    let booking = Booking {
        event_id: "v-1".into(),
        kind: "tollgate".into(),
        amount: 5,
        ..Booking::default()
    };
    let outcome =
        spedhub_plugin::send::Sender::new(Duration::from_secs(3)).send(&config, 1, &booking);
    assert_eq!(
        outcome,
        spedhub_plugin::send::Outcome::Sent { duplicate: false }
    );
    let head = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        head.contains(&format!("x-spedhub-version: {}", env!("CARGO_PKG_VERSION"))),
        "{head}"
    );
}

// ------------------------------------------------------------------- Nur das Firmenprofil

#[test]
fn ereignis_aus_dem_firmenprofil_geht_raus_und_traegt_das_profil() {
    let dir = temp_dir("firmenprofil");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let (token, company) = create_company(&url, "Hanna");
    // Der Launcher hat das Firmenprofil gemeldet: Der Server bucht nur noch daraus
    report_profile(&url, &token, company, COMPANY_PROFILE);
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, &token, company);

    let agent = Agent::start(&paths, fast()).unwrap();
    let toll = all_events().remove(3);
    let first = queued(agent.submit(&toll));
    wait_until("gebucht", || ledger(&url, &token, company).len() == 1);
    wait_until("Postausgang leer", || {
        read_entries(&paths.outbox()).is_empty()
    });

    // Profilwechsel im laufenden Spiel: nichts kommt an, nichts in den Postausgang
    select_profile(&paths, "Privat");
    for e in all_events() {
        assert_eq!(agent.submit(&e), Submitted::OtherProfile("Privat".into()));
    }
    assert!(read_entries(&paths.outbox()).is_empty());
    // Zurück ins Firmenprofil
    select_profile(&paths, COMPANY_PROFILE);
    let second = queued(agent.submit(&toll));
    wait_until("zwei Buchungen", || {
        ledger(&url, &token, company).len() == 2
    });
    wait_until("Postausgang leer", || {
        read_entries(&paths.outbox()).is_empty()
    });
    agent.shutdown();

    let l = ledger(&url, &token, company);
    for id in [&first, &second] {
        let e = l.iter().find(|e| e["event_id"] == id.as_str()).unwrap();
        assert_eq!(e["profile"], COMPANY_PROFILE, "{e}");
    }
    assert!(rejected_events(&url, &token, company).is_empty());
    let log = read_log(&paths);
    assert_eq!(log.matches("Ereignis nicht angenommen").count(), 6, "{log}");
    assert!(!log.contains(&token), "kein Token im Log");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bei_unbekanntem_profil_kommt_nichts_an() {
    let dir = temp_dir("unbekannt");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let (token, company) = create_company(&url, "Ida");
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, &token, company);
    // Das Spiel hat noch kein Profil gewählt
    fs::write(game_log(&paths), "00:00:00.000 : log created\n").unwrap();

    let agent = Agent::start(&paths, fast()).unwrap();
    for e in all_events() {
        assert_eq!(agent.submit(&e), Submitted::UnknownProfile);
    }
    driving(&agent, "SPED 001");
    wait_until("drei Takte ohne Herzschlag", || {
        agent.heartbeat_stats().blocked >= 3
    });
    assert_eq!(agent.heartbeat_stats().attempts, 0);
    agent.shutdown();
    assert!(ledger(&url, &token, company).is_empty());
    assert!(!paths.outbox().exists(), "kein Postausgang angelegt");
    assert!(member(&url, &token, company)["last_seen"].is_null());
    assert!(read_log(&paths).contains("Aktives Profil unbekannt"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn herzschlag_nur_aus_dem_firmenprofil() {
    let dir = temp_dir("herzschlag-profil");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let (token, company) = create_company(&url, "Jana");
    report_profile(&url, &token, company, COMPANY_PROFILE);
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, &token, company);
    select_profile(&paths, "Privat");

    let agent = Agent::start(&paths, fast()).unwrap();
    driving(&agent, "SPED 001");
    wait_until("drei verworfene Takte", || {
        agent.heartbeat_stats().blocked >= 3
    });
    assert_eq!(agent.heartbeat_stats().attempts, 0);
    assert!(member(&url, &token, company)["last_seen"].is_null());

    // Ins Firmenprofil gewechselt: Herzschläge kommen an, mit dem Profil
    select_profile(&paths, COMPANY_PROFILE);
    wait_until("online", || member(&url, &token, company)["online"] == true);
    assert!(agent.heartbeat_stats().delivered >= 1);
    agent.shutdown();
    let log = read_log(&paths);
    assert!(log.contains("Aktives Profil 'Privat' ist nicht das Firmenprofil"));
    assert!(log.contains("Aktives Profil 'Nordfracht' ist das Firmenprofil"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn herzschlag_den_der_server_verwirft_steht_im_log() {
    let dir = temp_dir("herzschlag-verworfen");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let (token, company) = create_company(&url, "Kai");
    // Der Server kennt ein anderes Firmenprofil als plugin.json, etwa weil der Launcher das
    // neue Profil noch nicht melden konnte
    report_profile(&url, &token, company, "Anderes Profil");
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, &token, company);
    let agent = Agent::start(&paths, fast()).unwrap();
    driving(&agent, "SPED 001");
    wait_until("drei Versuche", || agent.heartbeat_stats().attempts >= 3);
    assert_eq!(agent.heartbeat_stats().delivered, 0);
    agent.shutdown();
    assert!(member(&url, &token, company)["last_seen"].is_null());
    let log = read_log(&paths);
    assert_eq!(
        log.matches("Herzschlag nicht angekommen: Herzschlag verworfen")
            .count(),
        1,
        "{log}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn vom_server_wegen_profil_abgelehntes_ereignis_blockiert_den_postausgang_nicht() {
    let dir = temp_dir("abgelehnt-profil");
    let server = TestServer::start(&dir.join("server.db"));
    let url = server.url();
    let (token, company) = create_company(&url, "Lea");
    report_profile(&url, &token, company, COMPANY_PROFILE);
    let paths = Paths::new(dir.join("SpedHub"));
    write_config(&paths, &url, &token, company);

    // Im Postausgang liegen von einem älteren Plugin zwei Events ohne Profil, dazwischen eins
    // aus dem Firmenprofil
    let entry = |id: &str, profile: Option<&str>| Entry {
        company_id: company,
        booking: Booking {
            event_id: id.into(),
            kind: "tollgate".into(),
            amount: 40,
            note: "Maut".into(),
            profile: profile.map(str::to_string),
            ..Default::default()
        },
    };
    let lines: Vec<String> = [
        entry("alt-1", None),
        entry("neu-1", Some(COMPANY_PROFILE)),
        entry("alt-2", None),
    ]
    .iter()
    .map(|e| serde_json::to_string(e).unwrap())
    .collect();
    fs::write(paths.outbox(), lines.join("\n") + "\n").unwrap();

    let agent = Agent::start(&paths, fast()).unwrap();
    let id = queued(agent.submit(&all_events()[3]));
    wait_until("Postausgang leer", || {
        read_entries(&paths.outbox()).is_empty()
    });
    agent.shutdown();

    // Gebucht sind nur die aus dem Firmenprofil, die anderen liegen beim Server im Protokoll
    // und beim Plugin in den Ablehnungen, ohne erneut gesendet zu werden
    let l = ledger(&url, &token, company);
    let mut booked: Vec<&str> = l.iter().map(|e| e["event_id"].as_str().unwrap()).collect();
    booked.sort();
    let mut want = vec!["neu-1", id.as_str()];
    want.sort();
    assert_eq!(booked, want);
    let rejected = rejected_events(&url, &token, company);
    assert_eq!(rejected.len(), 2);
    let file = fs::read_to_string(paths.rejected()).unwrap();
    assert!(file.contains("alt-1") && file.contains("alt-2"), "{file}");
    assert!(file.contains("kein Profil"), "{file}");
    let log = read_log(&paths);
    assert_eq!(log.matches("Server lehnt alt-1 ab").count(), 1, "{log}");
    assert!(!log.contains("bleiben im Postausgang"), "{log}");
    let _ = fs::remove_dir_all(dir);
}
