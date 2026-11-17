//! Updates über die GitHub-Releases von SpedHub.
//!
//! Beim Start fragt der Launcher `GET /repos/LSSippler/SpedHub/releases/latest` ab (öffentliche
//! API, ohne Anmeldung). Ist `tag_name` neuer als die eigene Version, zeigt er einen Hinweis.
//! Erst auf Knopfdruck lädt er `SpedHub-Setup-<Version>.exe` und `SHA256SUMS.txt` aus dem
//! Release, prüft die SHA-256 und startet das Setup nur, wenn sie passt. Es gibt kein stilles
//! Update.

use crate::plugin_install::hex;
use crate::version::{VERSION, Version};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

/// Basis der GitHub-API für das Repo. In Tests zeigt sie auf einen lokalen Server.
pub const DEFAULT_API: &str = "https://api.github.com/repos/LSSippler/SpedHub";
/// Name der Datei mit den Prüfsummen im Release.
pub const CHECKSUMS: &str = "SHA256SUMS.txt";

/// Name des Setups für eine Version, so wie `installer/build.ps1` es baut.
pub fn setup_name(version: &Version) -> String {
    format!("SpedHub-Setup-{version}.exe")
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// Ein Release mit lesbarer Version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    /// Seite des Release auf GitHub
    pub html_url: String,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }

    pub fn setup_name(&self) -> String {
        setup_name(&self.version)
    }
}

/// Sucht die Prüfsumme einer Datei in einer `SHA256SUMS.txt`. Zeilen wie
/// `<hex>  SpedHub-Setup-0.3.0.exe` (Textmodus) oder `<hex> *SpedHub-Setup-0.3.0.exe`
/// (Binärmodus). Liefert die Prüfsumme in Kleinbuchstaben.
pub fn parse_checksums(text: &str, file: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim_end_matches('\r').trim_start_matches('\u{feff}');
        let (hash, rest) = line.split_once(char::is_whitespace)?;
        let name = rest.trim_start();
        let name = name.strip_prefix('*').unwrap_or(name);
        let valid = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        (valid && name == file).then(|| hash.to_ascii_lowercase())
    })
}

pub struct Updater {
    api: String,
    agent: ureq::Agent,
    download_agent: ureq::Agent,
}

fn agent(global: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(global)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .proxy(None)
        .user_agent(concat!("spedhub-launcher/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

impl Updater {
    /// `api` ist die Basis des Repos, normalerweise [`DEFAULT_API`].
    pub fn new(api: &str) -> Updater {
        Updater {
            api: api.trim().trim_end_matches('/').to_string(),
            agent: agent(Some(Duration::from_secs(15))),
            // Das Setup darf auch bei langsamer Leitung ein paar Minuten brauchen
            download_agent: agent(Some(Duration::from_secs(15 * 60))),
        }
    }

    /// Das neueste veröffentlichte Release.
    pub fn latest(&self) -> Result<Release, String> {
        let url = format!("{}/releases/latest", self.api);
        let mut res = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|e| format!("GitHub nicht erreichbar: {e}"))?;
        let status = res.status().as_u16();
        match status {
            200..=299 => {}
            404 => return Err("Es gibt noch kein veröffentlichtes Release".into()),
            403 | 429 => {
                return Err(
                    "GitHub lässt gerade keine weiteren Abfragen zu, später noch einmal".into(),
                );
            }
            s => return Err(format!("GitHub antwortet mit {s}")),
        }
        let body = res
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| format!("Antwort von GitHub unvollständig: {e}"))?;
        let json: ReleaseJson = serde_json::from_str(&body)
            .map_err(|e| format!("Antwort von GitHub nicht lesbar: {e}"))?;
        let version = Version::parse(&json.tag_name).ok_or_else(|| {
            format!(
                "Das neueste Release heißt {}, das ist keine Versionsnummer",
                json.tag_name
            )
        })?;
        Ok(Release {
            tag: json.tag_name,
            version,
            html_url: json.html_url,
            assets: json.assets,
        })
    }

    /// Das neueste Release, wenn es neuer ist als `current`, sonst `None`.
    pub fn check(&self, current: &str) -> Result<Option<Release>, String> {
        let current = Version::parse(current)
            .ok_or_else(|| format!("Die eigene Version {current} ist nicht lesbar"))?;
        let release = self.latest()?;
        Ok((release.version > current).then_some(release))
    }

    /// Das neueste Release, wenn es neuer ist als dieser Launcher.
    pub fn check_current(&self) -> Result<Option<Release>, String> {
        self.check(VERSION)
    }

    fn get(&self, url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
        let res = self
            .download_agent
            .get(url)
            .header("Accept", "application/octet-stream")
            .call()
            .map_err(|e| format!("Download von {url} fehlgeschlagen: {e}"))?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(format!(
                "Download von {url} fehlgeschlagen: Antwort {status}"
            ));
        }
        Ok(res)
    }

    /// Lädt das Setup des Release nach `dir` und prüft es gegen `SHA256SUMS.txt` aus demselben
    /// Release. Liefert den Pfad nur bei passender Prüfsumme. Fehlt etwas oder passt die
    /// Prüfsumme nicht, liegt danach keine Datei in `dir`.
    pub fn download(&self, release: &Release, dir: &Path) -> Result<PathBuf, String> {
        let name = release.setup_name();
        let setup = release
            .asset(&name)
            .ok_or_else(|| format!("Im Release {} fehlt {name}", release.tag))?;
        let sums = release.asset(CHECKSUMS).ok_or_else(|| {
            format!(
                "Im Release {} fehlt {CHECKSUMS}. Ohne Prüfsumme wird nichts installiert",
                release.tag
            )
        })?;
        let sums_text = self
            .get(&sums.browser_download_url)?
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_string()
            .map_err(|e| format!("{CHECKSUMS} ließ sich nicht laden: {e}"))?;
        let expected = parse_checksums(&sums_text, &name).ok_or_else(|| {
            format!("In {CHECKSUMS} steht keine Prüfsumme für {name}. Es wird nichts installiert")
        })?;

        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let target = dir.join(&name);
        let part = dir.join(format!("{name}.part"));
        let _ = fs::remove_file(&target);
        let result = self.fetch_to(&setup.browser_download_url, &part);
        let actual = match result {
            Ok(hash) => hash,
            Err(e) => {
                let _ = fs::remove_file(&part);
                return Err(e);
            }
        };
        if actual != expected {
            let _ = fs::remove_file(&part);
            return Err(format!(
                "Die Prüfsumme von {name} stimmt nicht (erwartet {expected}, geladen {actual}). Die Datei wurde gelöscht, es wurde nichts gestartet"
            ));
        }
        fs::rename(&part, &target).map_err(|e| {
            let _ = fs::remove_file(&part);
            format!("{}: {e}", target.display())
        })?;
        Ok(target)
    }

    /// Lädt `url` nach `path` und liefert die SHA-256 des Inhalts.
    fn fetch_to(&self, url: &str, path: &Path) -> Result<String, String> {
        let mut res = self.get(url)?;
        let mut reader = res
            .body_mut()
            .with_config()
            .limit(512 * 1024 * 1024)
            .reader();
        let mut file = fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("Download abgebrochen: {e}"))?;
            if n == 0 {
                break;
            }
            let chunk = buf.get(..n).unwrap_or_default();
            hasher.update(chunk);
            file.write_all(chunk)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        file.sync_all()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(hex(&hasher.finalize()))
    }
}

/// Lädt das Setup, prüft es und startet es mit `start` (in der Oberfläche
/// [`crate::system::open_path`], in Tests eine Attrappe). Bei falscher Prüfsumme wird nichts
/// gestartet.
pub fn download_and_start(
    updater: &Updater,
    release: &Release,
    dir: &Path,
    start: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let setup = updater.download(release, dir)?;
    start(&setup).map_err(|e| format!("Das Setup ließ sich nicht starten: {e}"))?;
    Ok(setup)
}

/// Ordner für das geladene Setup.
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("SpedHub-Update")
}
