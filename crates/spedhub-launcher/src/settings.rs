//! Einstellungen des Launchers und die Konfiguration für das Plugin.
//!
//! | Datei in `%LOCALAPPDATA%\SpedHub` | Inhalt |
//! |---|---|
//! | `launcher.json` | Server, Name, Firma, Profil, Spielordner, Token (unter Windows mit DPAPI verschlüsselt) |
//! | `plugin.json` | Server, Token, Firma für das Spiel-Plugin. Das Plugin liest den Token im Klartext |
//! | `backups\` | Kopien der Speicherplätze, bevor der Launcher etwas schreibt |

use crate::error::{Error, Result};
use crate::system;
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

pub const DEFAULT_SERVER: &str = "http://127.0.0.1:8787";

/// Die Ablageorte. In Tests zeigt `dir` auf einen leeren Ordner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub dir: PathBuf,
}

impl AppPaths {
    pub fn new(dir: impl Into<PathBuf>) -> AppPaths {
        AppPaths { dir: dir.into() }
    }

    pub fn from_env() -> Option<AppPaths> {
        system::app_dir().map(AppPaths::new)
    }

    pub fn settings(&self) -> PathBuf {
        self.dir.join("launcher.json")
    }

    pub fn plugin_config(&self) -> PathBuf {
        self.dir.join("plugin.json")
    }

    pub fn backups(&self) -> PathBuf {
        self.dir.join("backups")
    }
}

/// Inhalt von `launcher.json`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub server_url: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub company_id: Option<i64>,
    #[serde(default)]
    pub profile_dir: Option<PathBuf>,
    /// Ordner von Euro Truck Simulator 2, wenn er von Hand gewählt wurde. Sonst sucht der
    /// Launcher über Steam.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_dir: Option<PathBuf>,
    /// Verschlüsselter Token als Hex
    #[serde(default)]
    token: Option<String>,
    /// Verfahren, mit dem der Token verschlüsselt ist (`dpapi`, außerhalb von Windows `none`)
    #[serde(default)]
    token_method: Option<String>,
}

// Der Token gehört nicht in Debug-Ausgaben, auch nicht verschlüsselt.
impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("server_url", &self.server_url)
            .field("user_name", &self.user_name)
            .field("company_id", &self.company_id)
            .field("profile_dir", &self.profile_dir)
            .field("game_dir", &self.game_dir)
            .field("token", &self.token.as_ref().map(|_| "***"))
            .finish()
    }
}

impl Settings {
    /// Liest die Einstellungen. Fehlt die Datei oder ist sie kaputt, gibt es die Vorgaben.
    pub fn load(paths: &AppPaths) -> Settings {
        let loaded = fs::read_to_string(paths.settings())
            .ok()
            .and_then(|t| serde_json::from_str::<Settings>(&t).ok());
        let mut s = loaded.unwrap_or_default();
        if s.server_url.trim().is_empty() {
            s.server_url = DEFAULT_SERVER.into();
        }
        s
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Other(format!("Einstellungen: {e}")))?;
        write_atomic(&paths.settings(), text.as_bytes())
    }

    /// Speichert den Token verschlüsselt.
    pub fn set_token(&mut self, token: &str) -> Result<()> {
        let (data, method) = system::protect(token.as_bytes()).map_err(Error::Other)?;
        self.token = Some(to_hex(&data));
        self.token_method = Some(method.into());
        Ok(())
    }

    /// Der entschlüsselte Token, falls einer gespeichert ist und sich lesen lässt.
    pub fn token(&self) -> Option<String> {
        let data = from_hex(self.token.as_deref()?)?;
        let plain = system::unprotect(&data, self.token_method.as_deref()?).ok()?;
        String::from_utf8(plain).ok()
    }

    pub fn clear_token(&mut self) {
        self.token = None;
        self.token_method = None;
    }

    /// Steht der Token so in der Datei, dass man ihn ohne Entschlüsseln lesen könnte?
    pub fn token_method(&self) -> Option<&str> {
        self.token_method.as_deref()
    }
}

/// Inhalt von `plugin.json`, so wie `crates/spedhub-plugin` ihn liest.
#[derive(Serialize)]
struct PluginConfig<'a> {
    server_url: &'a str,
    token: &'a str,
    company_id: i64,
}

/// Schreibt `plugin.json` für das Spiel-Plugin.
pub fn write_plugin_config(
    paths: &AppPaths,
    server_url: &str,
    token: &str,
    company_id: i64,
) -> Result<()> {
    let config = PluginConfig {
        server_url: server_url.trim().trim_end_matches('/'),
        token,
        company_id,
    };
    let text = serde_json::to_string_pretty(&config)
        .map_err(|e| Error::Other(format!("Plugin-Konfiguration: {e}")))?;
    write_atomic(&paths.plugin_config(), text.as_bytes())
}

/// Entfernt `plugin.json`, zum Beispiel beim Abmelden. Das Plugin bleibt dann still.
pub fn remove_plugin_config(paths: &AppPaths) -> Result<()> {
    match fs::remove_file(paths.plugin_config()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(paths.plugin_config(), &e)),
    }
}

/// Schreibt über eine temporäre Datei, damit nie eine halbe Datei liegen bleibt.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data).map_err(|e| Error::io(&tmp, &e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(path, &e)
    })
}

fn to_hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

impl AppPaths {
    /// Ablage für Lkw-Pakete geliehener Lkw. Der Server liefert den Bauplan nur beim Ausleihen.
    /// Läuft in dem Moment das Spiel, schreibt der Launcher später aus dieser Ablage.
    pub fn leases(&self) -> PathBuf {
        self.dir.join("leases")
    }

    fn lease_file(&self, truck_id: i64) -> PathBuf {
        self.leases().join(format!("{truck_id}.json"))
    }

    /// Legt das Paket eines geliehenen Lkw ab.
    pub fn store_lease(&self, truck_id: i64, package: &spedhub_save::TruckPackage) -> Result<()> {
        write_atomic(&self.lease_file(truck_id), package.to_json()?.as_bytes())
    }

    /// Das abgelegte Paket, falls es eines gibt.
    pub fn load_lease(&self, truck_id: i64) -> Option<spedhub_save::TruckPackage> {
        let text = fs::read_to_string(self.lease_file(truck_id)).ok()?;
        spedhub_save::TruckPackage::from_json(&text).ok()
    }

    /// Nach der Rückgabe braucht es das Paket nicht mehr.
    pub fn remove_lease(&self, truck_id: i64) {
        let _ = fs::remove_file(self.lease_file(truck_id));
    }
}
