//! Updates gegen einen lokalen HTTP-Server, der die GitHub-API nachspielt. Kein Test greift
//! ins Internet.
mod common;

use common::*;
use serde_json::json;
use spedhub_launcher::update::{
    CHECKSUMS, Updater, download_and_start, parse_checksums, setup_name,
};
use spedhub_launcher::version::Version;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Routes = Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>;

/// Ein kleiner HTTP-Server: Pfad zu Status und Inhalt. Merkt sich die Köpfe der Anfragen.
struct FakeGitHub {
    url: String,
    routes: Routes,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeGitHub {
    fn start() -> FakeGitHub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let routes: Routes = Arc::default();
        let requests: Arc<Mutex<Vec<String>>> = Arc::default();
        let (r, q) = (routes.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (r, q) = (r.clone(), q.clone());
                std::thread::spawn(move || serve(stream, &r, &q));
            }
        });
        FakeGitHub {
            url,
            routes,
            requests,
        }
    }

    fn set(&self, path: &str, status: u16, body: impl Into<Vec<u8>>) {
        self.routes
            .lock()
            .unwrap()
            .insert(path.to_string(), (status, body.into()));
    }

    fn api(&self) -> String {
        format!("{}/repos/LSSippler/SpedHub", self.url)
    }

    /// Ein Release mit Setup und Prüfsummen. `sums` ist der Inhalt von SHA256SUMS.txt, `None`
    /// lässt die Datei weg, ebenso `setup` das Setup.
    fn release(&self, tag: &str, setup: Option<&[u8]>, sums: Option<String>) {
        let version = Version::parse(tag).unwrap();
        let name = setup_name(&version);
        let mut assets = Vec::new();
        if let Some(content) = setup {
            let path = format!("/download/{tag}/{name}");
            self.set(&path, 200, content.to_vec());
            assets.push(json!({
                "name": name,
                "browser_download_url": format!("{}{path}", self.url),
                "size": content.len(),
            }));
        }
        if let Some(text) = sums {
            let path = format!("/download/{tag}/{CHECKSUMS}");
            self.set(&path, 200, text);
            assets.push(json!({
                "name": CHECKSUMS,
                "browser_download_url": format!("{}{path}", self.url),
            }));
        }
        // Eine Datei, die mit dem Setup nichts zu tun hat
        assets.push(json!({
            "name": "spedhub-server-linux-x86_64",
            "browser_download_url": format!("{}/download/{tag}/server", self.url),
        }));
        let body = json!({
            "tag_name": tag,
            "name": format!("SpedHub {version}"),
            "html_url": format!("https://github.com/LSSippler/SpedHub/releases/tag/{tag}"),
            "draft": false,
            "prerelease": false,
            "assets": assets,
        });
        self.set(
            "/repos/LSSippler/SpedHub/releases/latest",
            200,
            body.to_string(),
        );
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn serve(mut s: TcpStream, routes: &Routes, requests: &Mutex<Vec<String>>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let path = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    requests.lock().unwrap().push(head);
    let (status, body) = routes
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap_or((404, br#"{"message":"Not Found"}"#.to_vec()));
    let reply = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/octet-stream\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(reply.as_bytes());
    let _ = s.write_all(&body);
}

fn sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

const SETUP: &[u8] = b"MZ dies ist ein nachgebautes Setup";

#[test]
fn neuer_gleich_aelter() {
    let gh = FakeGitHub::start();
    let updater = Updater::new(&gh.api());

    gh.release("v0.3.0", Some(SETUP), Some(String::new()));
    let r = updater.check("0.2.0").unwrap().expect("neuer");
    assert_eq!(r.tag, "v0.3.0");
    assert_eq!(r.version.to_string(), "0.3.0");
    assert_eq!(r.setup_name(), "SpedHub-Setup-0.3.0.exe");
    assert!(r.html_url.ends_with("/releases/tag/v0.3.0"));
    assert!(r.asset("SpedHub-Setup-0.3.0.exe").is_some());

    // Anfrage wie bei GitHub: User-Agent und Accept
    let head = gh.requests().last().unwrap().to_lowercase();
    assert!(
        head.starts_with("get /repos/lssippler/spedhub/releases/latest "),
        "{head}"
    );
    assert!(
        head.contains(&format!(
            "user-agent: spedhub-launcher/{}",
            env!("CARGO_PKG_VERSION")
        )),
        "{head}"
    );
    assert!(
        head.contains("accept: application/vnd.github+json"),
        "{head}"
    );

    assert_eq!(updater.check("0.3.0").unwrap(), None, "gleich");
    assert_eq!(updater.check("v0.3.0").unwrap(), None, "gleich mit v");
    assert_eq!(updater.check("0.4.1").unwrap(), None, "älter");
    gh.release("v0.2.1", Some(SETUP), None);
    assert!(updater.check("0.2.0").unwrap().is_some(), "Fehlerbehebung");
    assert!(updater.check_current().is_ok());
}

#[test]
fn fehler_bei_der_abfrage_stoeren_nicht() {
    let gh = FakeGitHub::start();
    let updater = Updater::new(&gh.api());
    // Noch kein Release
    let e = updater.check("0.2.0").unwrap_err();
    assert!(e.contains("kein veröffentlichtes Release"), "{e}");
    // Limit
    gh.set("/repos/LSSippler/SpedHub/releases/latest", 403, "{}");
    let e = updater.check("0.2.0").unwrap_err();
    assert!(e.contains("keine weiteren Abfragen"), "{e}");
    // Kaputte Antwort, Tag ohne Version
    gh.set("/repos/LSSippler/SpedHub/releases/latest", 200, "kein json");
    assert!(updater.check("0.2.0").unwrap_err().contains("nicht lesbar"));
    gh.set(
        "/repos/LSSippler/SpedHub/releases/latest",
        200,
        r#"{"tag_name":"nightly","assets":[]}"#,
    );
    let e = updater.check("0.2.0").unwrap_err();
    assert!(e.contains("keine Versionsnummer"), "{e}");
    // Offline
    let e = Updater::new("http://127.0.0.1:1/repos/x/y")
        .check("0.2.0")
        .unwrap_err();
    assert!(e.contains("nicht erreichbar"), "{e}");
}

#[test]
fn download_mit_richtiger_pruefsumme_startet_das_setup() {
    let gh = FakeGitHub::start();
    let name = "SpedHub-Setup-0.3.0.exe";
    // Binärmodus mit *, andere Dateien davor und Windows-Zeilenenden
    let sums = format!(
        "{}  spedhub-server-linux-x86_64\r\n{} *{name}\r\n",
        sha256(b"server"),
        sha256(SETUP).to_uppercase()
    );
    gh.release("v0.3.0", Some(SETUP), Some(sums));
    let updater = Updater::new(&gh.api());
    let release = updater.check("0.2.0").unwrap().unwrap();
    let dir = temp_dir("update-ok");

    let started = RefCell::new(Vec::<PathBuf>::new());
    let start = |p: &Path| -> Result<(), String> {
        started.borrow_mut().push(p.to_path_buf());
        Ok(())
    };
    let path = download_and_start(&updater, &release, &dir, &start).unwrap();
    assert_eq!(path, dir.join(name));
    assert_eq!(fs::read(&path).unwrap(), SETUP);
    assert_eq!(*started.borrow(), vec![path.clone()]);
    assert_eq!(files_in(&dir), vec![name.to_string()], "kein .part bleibt");

    // Ein zweites Mal überschreibt die alte Datei
    let path = updater.download(&release, &dir).unwrap();
    assert_eq!(fs::read(&path).unwrap(), SETUP);

    // Startet das Setup nicht, gibt es eine Meldung
    let e = download_and_start(&updater, &release, &dir, &|_| Err("kaputt".into())).unwrap_err();
    assert!(e.contains("ließ sich nicht starten"), "{e}");
}

#[test]
fn falsche_pruefsumme_loescht_und_startet_nichts() {
    let gh = FakeGitHub::start();
    let name = "SpedHub-Setup-0.3.0.exe";
    gh.release(
        "v0.3.0",
        Some(SETUP),
        Some(format!("{}  {name}\n", sha256(b"ein anderes Setup"))),
    );
    let updater = Updater::new(&gh.api());
    let release = updater.check("0.2.0").unwrap().unwrap();
    let dir = temp_dir("update-falsch");
    // Eine alte Datei gleichen Namens bleibt auch nicht liegen
    fs::write(dir.join(name), b"alt").unwrap();

    let started = RefCell::new(0);
    let start = |_: &Path| -> Result<(), String> {
        *started.borrow_mut() += 1;
        Ok(())
    };
    let e = download_and_start(&updater, &release, &dir, &start).unwrap_err();
    assert!(e.contains("Prüfsumme") && e.contains("stimmt nicht"), "{e}");
    assert!(e.contains("nichts gestartet"), "{e}");
    assert_eq!(*started.borrow(), 0);
    assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));
}

#[test]
fn fehlendes_setup_oder_fehlende_pruefsummen() {
    let gh = FakeGitHub::start();
    let updater = Updater::new(&gh.api());
    let dir = temp_dir("update-fehlt");
    let never = |_: &Path| -> Result<(), String> { panic!("darf nicht starten") };

    // Setup fehlt im Release
    gh.release("v0.3.0", None, Some(format!("{}  x\n", sha256(SETUP))));
    let release = updater.check("0.2.0").unwrap().unwrap();
    let e = download_and_start(&updater, &release, &dir, &never).unwrap_err();
    assert!(e.contains("fehlt SpedHub-Setup-0.3.0.exe"), "{e}");

    // SHA256SUMS.txt fehlt
    gh.release("v0.3.0", Some(SETUP), None);
    let release = updater.check("0.2.0").unwrap().unwrap();
    let e = download_and_start(&updater, &release, &dir, &never).unwrap_err();
    assert!(e.contains("fehlt SHA256SUMS.txt"), "{e}");
    assert!(e.contains("Ohne Prüfsumme"), "{e}");

    // SHA256SUMS.txt ohne Zeile für das Setup
    gh.release(
        "v0.3.0",
        Some(SETUP),
        Some(format!("{}  SpedHub-Setup-0.2.9.exe\n", sha256(SETUP))),
    );
    let release = updater.check("0.2.0").unwrap().unwrap();
    let e = download_and_start(&updater, &release, &dir, &never).unwrap_err();
    assert!(
        e.contains("keine Prüfsumme für SpedHub-Setup-0.3.0.exe"),
        "{e}"
    );

    // Asset steht im Release, der Download gibt aber 404
    gh.release(
        "v0.3.0",
        Some(SETUP),
        Some(format!("{}  SpedHub-Setup-0.3.0.exe\n", sha256(SETUP))),
    );
    gh.set("/download/v0.3.0/SpedHub-Setup-0.3.0.exe", 404, "weg");
    let release = updater.check("0.2.0").unwrap().unwrap();
    let e = download_and_start(&updater, &release, &dir, &never).unwrap_err();
    assert!(e.contains("Antwort 404"), "{e}");
    assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));
}

#[test]
fn pruefsummen_lesen() {
    let h = "a".repeat(64);
    let name = "SpedHub-Setup-0.3.0.exe";
    assert_eq!(
        parse_checksums(&format!("{h}  {name}"), name),
        Some(h.clone())
    );
    assert_eq!(
        parse_checksums(&format!("{h} *{name}\n"), name),
        Some(h.clone())
    );
    assert_eq!(
        parse_checksums(&format!("{}  {name}\r\n", h.to_uppercase()), name),
        Some(h.clone())
    );
    assert_eq!(parse_checksums(&format!("{h}  {name}.alt"), name), None);
    assert_eq!(parse_checksums(&format!("{h}  x/{name}"), name), None);
    assert_eq!(
        parse_checksums(&format!("{}  {name}", "a".repeat(63)), name),
        None
    );
    assert_eq!(
        parse_checksums(&format!("{}  {name}", "g".repeat(64)), name),
        None
    );
    assert_eq!(parse_checksums(name, name), None);
    assert_eq!(parse_checksums("", name), None);
}
