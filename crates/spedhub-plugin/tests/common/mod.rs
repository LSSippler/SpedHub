//! Hilfen für die Tests: echter spedhub-server, Firma anlegen, Kassenbuch lesen.
#![allow(dead_code)]

use serde_json::{Value, json};
use spedhub_plugin::{Paths, Settings};
use std::{
    fs,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

/// Ein frischer, leerer Ordner.
pub fn temp_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "spedhub-plugin-test-{}-{n}-{label}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Ein laufender spedhub-server. Beim Drop hört er auf.
pub struct TestServer {
    pub addr: SocketAddr,
    runtime: Option<tokio::runtime::Runtime>,
}

impl TestServer {
    /// Startet auf einem freien Port mit der Datenbank in `db`.
    pub fn start(db: &Path) -> TestServer {
        TestServer::start_on("127.0.0.1:0".parse().unwrap(), db)
    }

    /// Startet auf einer bestimmten Adresse, zum Beispiel nach einem simulierten Ausfall
    /// wieder auf dem alten Port.
    pub fn start_on(addr: SocketAddr, db: &Path) -> TestServer {
        let listener = bind_with_retry(addr);
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

    /// Hält den Server an und gibt den Port frei.
    pub fn stop(mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

fn bind_with_retry(addr: SocketAddr) -> TcpListener {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match TcpListener::bind(addr) {
            Ok(l) => return l,
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("Port {addr} bleibt belegt: {e}"),
        }
    }
}

/// Ein Port, auf dem gerade niemand lauscht.
pub fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into()
}

pub fn post(url: &str, token: Option<&str>, body: Value) -> Value {
    let mut req = agent().post(url);
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    let mut res = req.send_json(body).unwrap();
    assert!(res.status().is_success(), "{url}: {}", res.status());
    res.body_mut().read_json().unwrap()
}

/// Fahrer anlegen und Firma gründen. Liefert Token und Firmen-ID.
pub fn create_company(server_url: &str, name: &str) -> (String, i64) {
    let r = post(
        &format!("{server_url}/api/register"),
        None,
        json!({ "name": name, "password": "geheim1234" }),
    );
    let token = r["token"].as_str().unwrap().to_string();
    let c = post(
        &format!("{server_url}/api/companies"),
        Some(&token),
        json!({ "name": format!("Firma {name}") }),
    );
    (token, c["id"].as_i64().unwrap())
}

/// Firma mit Kassenstand und Lohnkonten.
pub fn company_info(server_url: &str, token: &str, id: i64) -> Value {
    let mut res = agent()
        .get(&format!("{server_url}/api/companies/{id}"))
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .unwrap();
    res.body_mut().read_json().unwrap()
}

/// Alle Buchungen der Firma, neueste zuerst.
pub fn ledger(server_url: &str, token: &str, id: i64) -> Vec<Value> {
    let mut res = agent()
        .get(&format!("{server_url}/api/companies/{id}/ledger?limit=500"))
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .unwrap();
    let v: Value = res.body_mut().read_json().unwrap();
    v["entries"].as_array().unwrap().clone()
}

/// Das Firmenprofil in den Tests.
pub const COMPANY_PROFILE: &str = "Nordfracht";

/// Wo in den Tests `game.log.txt` liegt.
pub fn game_log(paths: &Paths) -> PathBuf {
    paths.dir.join("game.log.txt")
}

/// `plugin.json` schreiben, wie es der Launcher tut, mit Firmenprofil. Dazu ein
/// `game.log.txt`, in dem das Spiel das Firmenprofil gewählt hat.
pub fn write_config(paths: &Paths, server_url: &str, token: &str, company_id: i64) {
    fs::create_dir_all(&paths.dir).unwrap();
    fs::write(
        paths.config(),
        json!({
            "server_url": server_url, "token": token, "company_id": company_id,
            "profile_name": COMPANY_PROFILE, "game_log": game_log(paths)
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        game_log(paths),
        "00:00:00.000 : log created\n00:00:12.345 : New profile selected: 'Nordfracht'\n",
    )
    .unwrap();
}

/// `plugin.json` von einem älteren Launcher: ohne Firmenprofil und ohne Pfad zum Log.
pub fn write_old_config(paths: &Paths, server_url: &str, token: &str, company_id: i64) {
    fs::create_dir_all(&paths.dir).unwrap();
    fs::write(
        paths.config(),
        json!({ "server_url": server_url, "token": token, "company_id": company_id }).to_string(),
    )
    .unwrap();
}

/// Das Spiel wählt ein Profil: eine Zeile mehr in `game.log.txt`.
pub fn select_profile(paths: &Paths, name: &str) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(game_log(paths))
        .unwrap();
    writeln!(f, "00:10:00.000 : New profile selected: '{name}'").unwrap();
}

/// Das Firmenprofil beim Server melden, wie es der Launcher tut.
pub fn report_profile(server_url: &str, token: &str, company_id: i64, name: &str) {
    let mut res = agent()
        .put(&format!("{server_url}/api/companies/{company_id}/profile"))
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(json!({ "profile_name": name }))
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    let _: Value = res.body_mut().read_json().unwrap();
}

/// Die abgelehnten Ereignisse der Firma, wie der Chef sie sieht.
pub fn rejected_events(server_url: &str, token: &str, id: i64) -> Vec<Value> {
    let mut res = agent()
        .get(&format!("{server_url}/api/companies/{id}/rejected-events"))
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .unwrap();
    let v: Value = res.body_mut().read_json().unwrap();
    v["entries"].as_array().unwrap().clone()
}

/// Kurze Pausen, damit die Tests schnell sind.
pub fn fast() -> Settings {
    Settings {
        retry_initial: Duration::from_millis(100),
        retry_max: Duration::from_millis(400),
        request_timeout: Duration::from_millis(1500),
        heartbeat_interval: Duration::from_millis(50),
    }
}

/// Wartet, bis die Bedingung gilt. Fehlschlag nach zehn Sekunden.
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "Zeitüberschreitung: {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub fn read_log(paths: &Paths) -> String {
    fs::read_to_string(paths.log()).unwrap_or_default()
}
