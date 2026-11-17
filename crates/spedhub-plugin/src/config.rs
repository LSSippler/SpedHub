//! Konfiguration und Ablageorte.
//!
//! Alles liegt in `%LOCALAPPDATA%\SpedHub`:
//!
//! | Datei | Inhalt |
//! |---|---|
//! | `plugin.json` | Server, Token, Firma. Schreibt der Launcher |
//! | `plugin.log` | Log, rotiert nach `plugin.log.1` |
//! | `plugin-outbox.jsonl` | Postausgang: Events, die noch nicht beim Server sind |
//! | `plugin-rejected.jsonl` | Events, die der Server dauerhaft abgelehnt hat |
//! | `plugin-state.json` | Kennung dieser Installation und Startzähler für die `event_id` |

use serde::Deserialize;
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

/// Die Ablageorte des Plugins. In Tests zeigt `dir` auf einen leeren Ordner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub dir: PathBuf,
}

impl Paths {
    pub fn new(dir: impl Into<PathBuf>) -> Paths {
        Paths { dir: dir.into() }
    }

    /// `%LOCALAPPDATA%\SpedHub`. `None`, wenn die Variable fehlt.
    pub fn from_env() -> Option<Paths> {
        let base = std::env::var_os("LOCALAPPDATA")?;
        if base.is_empty() {
            return None;
        }
        Some(Paths::new(Path::new(&base).join("SpedHub")))
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("plugin.json")
    }

    pub fn log(&self) -> PathBuf {
        self.dir.join("plugin.log")
    }

    pub fn outbox(&self) -> PathBuf {
        self.dir.join("plugin-outbox.jsonl")
    }

    pub fn rejected(&self) -> PathBuf {
        self.dir.join("plugin-rejected.jsonl")
    }

    pub fn state(&self) -> PathBuf {
        self.dir.join("plugin-state.json")
    }
}

/// Inhalt von `plugin.json`.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    /// Zum Beispiel `http://127.0.0.1:8787`, ohne `/` am Ende
    pub server_url: String,
    pub token: String,
    pub company_id: i64,
}

// Der Token gehört nie ins Log, auch nicht über Debug.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("server_url", &self.server_url)
            .field("token", &"***")
            .field("company_id", &self.company_id)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// Die Datei gibt es nicht. Normal, solange der Launcher sie nicht geschrieben hat.
    Missing(PathBuf),
    Unreadable(PathBuf, String),
    Invalid(PathBuf, String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Missing(p) => write!(
                f,
                "Keine Konfiguration unter {}. Das Plugin bleibt aus",
                p.display()
            ),
            ConfigError::Unreadable(p, e) => write!(
                f,
                "Konfiguration {} lässt sich nicht lesen: {e}. Das Plugin bleibt aus",
                p.display()
            ),
            ConfigError::Invalid(p, e) => write!(
                f,
                "Konfiguration {} ist ungültig: {e}. Das Plugin bleibt aus",
                p.display()
            ),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ConfigError::Missing(path.into()));
            }
            Err(e) => return Err(ConfigError::Unreadable(path.into(), e.to_string())),
        };
        // Ein BOM, wie ihn manche Windows-Editoren schreiben, stört serde_json.
        let text = text.trim_start_matches('\u{feff}');
        let mut config: Config = serde_json::from_str(text)
            .map_err(|e| ConfigError::Invalid(path.into(), e.to_string()))?;
        config
            .normalize()
            .map_err(|e| ConfigError::Invalid(path.into(), e))?;
        Ok(config)
    }

    fn normalize(&mut self) -> Result<(), String> {
        let url = self.server_url.trim().trim_end_matches('/').to_string();
        let rest = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .ok_or("server_url muss mit http:// oder https:// beginnen")?;
        if rest.is_empty() || rest.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("server_url ist keine gültige Adresse".into());
        }
        self.server_url = url;
        let token = self.token.trim();
        if token.is_empty() || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("token fehlt oder enthält Leerzeichen".into());
        }
        self.token = token.to_string();
        if self.company_id <= 0 {
            return Err("company_id muss eine positive Zahl sein".into());
        }
        Ok(())
    }
}
