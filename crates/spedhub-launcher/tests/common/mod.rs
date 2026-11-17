//! Hilfen für die Tests: Profilordner mit echter Struktur, Prozessprüfung zum Austauschen,
//! echter spedhub-server.
#![allow(dead_code)]

use spedhub_launcher::profiles::encode_hex_name;
use spedhub_launcher::system::GameCheck;
use std::{
    fs,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

pub const QUELLE: &str = "quelle_mit_lkw.sii";
pub const ZIEL: &str = "ziel_schnellauftrag.sii";
pub const ERWARTET: &str = "erwartet_nach_einsetzen.sii";
pub const INFO: &str = "ziel_info.sii";
/// `erwartet_nach_einsetzen.sii`, nachdem das Spiel gespeichert hat: neue Unit-Namen, neues
/// Kennzeichen mit Markup, 425 km mehr.
pub const NACH_SPIEL: &str = "nach_spiel_neues_kennzeichen.sii";

pub fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(name)
}

pub fn temp_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "spedhub-launcher-test-{}-{n}-{label}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Legt einen Speicherplatz an: `game.sii` und `info.sii` aus testdata, mit Änderungszeit
/// `age` Sekunden vor jetzt.
pub fn add_slot(profile_dir: &Path, id: &str, game: &str, info: Option<&str>, age: u64) -> PathBuf {
    let dir = profile_dir.join("save").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::copy(testdata(game), dir.join("game.sii")).unwrap();
    if let Some(i) = info {
        fs::copy(testdata(i), dir.join("info.sii")).unwrap();
    }
    let when = SystemTime::now() - Duration::from_secs(age);
    let f = fs::File::options()
        .write(true)
        .open(dir.join("game.sii"))
        .unwrap();
    f.set_modified(when).unwrap();
    dir
}

/// Ein Dokumente-Ordner wie unter Windows:
///
/// ```text
/// Euro Truck Simulator 2\
///   profiles\
///     <Nordfracht>\save\autosave (Ziel im Schnellauftrag, neuester)
///                       \1        (Quelle mit drei Lkw, älter)
///                       \quicksave (nur info.sii, kein game.sii)
///     <Müller Logistik>\        (ohne Spielstand)
///     Kein Hex!\                (Ordnername, der kein Hex ist)
///     liesmich.txt              (Datei, kein Profil)
///   steam_profiles\
///     <Alex Steam>\save\autosave
/// ```
pub struct Documents {
    pub ets2: PathBuf,
    pub nordfracht: PathBuf,
    pub mueller: PathBuf,
    pub steam: PathBuf,
}

pub fn documents(label: &str) -> Documents {
    let root = temp_dir(label);
    let ets2 = root.join("Euro Truck Simulator 2");
    let profiles = ets2.join("profiles");
    let nordfracht = profiles.join(encode_hex_name("Nordfracht"));
    add_slot(&nordfracht, "autosave", ZIEL, Some(INFO), 60);
    let mut info = fs::read_to_string(testdata(INFO)).unwrap();
    info = info.replace(" name: \"\"", " name: \"Mein Scania\"");
    let one = add_slot(&nordfracht, "1", QUELLE, None, 3600);
    fs::write(one.join("info.sii"), info).unwrap();
    let quick = nordfracht.join("save").join("quicksave");
    fs::create_dir_all(&quick).unwrap();
    fs::copy(testdata(INFO), quick.join("info.sii")).unwrap();

    let mueller = profiles.join(encode_hex_name("Müller Logistik"));
    fs::create_dir_all(&mueller).unwrap();
    fs::create_dir_all(profiles.join("Kein Hex!")).unwrap();
    fs::write(profiles.join("liesmich.txt"), "kein Profil").unwrap();

    let steam = ets2
        .join("steam_profiles")
        .join(encode_hex_name("Alex Steam"));
    add_slot(&steam, "autosave", ZIEL, Some(INFO), 10);
    Documents {
        ets2,
        nordfracht,
        mueller,
        steam,
    }
}

/// Prozessprüfung zum Austauschen. Liefert die Antworten der Reihe nach, die letzte bleibt.
pub struct FakeGame {
    answers: Mutex<Vec<Result<bool, String>>>,
}

impl FakeGame {
    pub fn closed() -> FakeGame {
        FakeGame::sequence(vec![Ok(false)])
    }

    pub fn running() -> FakeGame {
        FakeGame::sequence(vec![Ok(true)])
    }

    pub fn sequence(answers: Vec<Result<bool, String>>) -> FakeGame {
        FakeGame {
            answers: Mutex::new(answers),
        }
    }
}

impl GameCheck for FakeGame {
    fn is_running(&self) -> Result<bool, String> {
        let mut a = self.answers.lock().unwrap();
        if a.len() > 1 {
            a.remove(0)
        } else {
            a[0].clone()
        }
    }
}

/// Alle Dateien unterhalb eines Ordners mit Inhalt, zum Vergleichen vorher und nachher.
pub fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

// ------------------------------------------------------------------- echter Server

pub struct TestServer {
    pub addr: SocketAddr,
    runtime: Option<tokio::runtime::Runtime>,
}

impl TestServer {
    pub fn start(db: &Path) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let db = spedhub_server::db::open(db.to_str().unwrap()).unwrap();
        let app = spedhub_server::api::router(db);
        runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let _ = axum::serve(listener, app).await;
        });
        TestServer {
            addr,
            runtime: Some(runtime),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

/// Direkte Anfrage an den Server für Dinge, die der Launcher (noch) nicht kann.
pub fn post_raw(url: &str, token: &str, body: serde_json::Value) -> serde_json::Value {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    let mut res = agent
        .post(url)
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .unwrap();
    assert!(res.status().is_success(), "{url}: {}", res.status());
    res.body_mut().read_json().unwrap()
}
