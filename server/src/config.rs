//! Start-Einstellungen des Servers: Datenbank und Adresse aus Kommandozeile und Umgebung.
//!
//! | Quelle | Datenbank | Adresse |
//! |---|---|---|
//! | `SPEDHUB_DB`, `SPEDHUB_ADDR` | gehen immer vor | gehen immer vor |
//! | `--local` | `%LOCALAPPDATA%\SpedHub\server\spedhub.db`, unter Linux `~/.local/share/SpedHub/server/spedhub.db` | `127.0.0.1:8787` |
//! | sonst | `spedhub.db` im aktuellen Ordner | `127.0.0.1:8787` |
//!
//! `--local` gibt es für die Verknüpfung aus dem Setup: Eine Verknüpfung kann keine
//! Umgebungsvariablen setzen, und ein fester Pfad im Setup wäre das Profil des Admins, der
//! installiert hat, nicht das des Benutzers, der den Server startet.

use std::{ffi::OsString, net::SocketAddr, path::PathBuf};

pub const DEFAULT_ADDR: &str = "127.0.0.1:8787";
pub const DEFAULT_DB: &str = "spedhub.db";

/// Was der Server tun soll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Server starten
    Run(Config),
    /// `--version`: nur die Version ausgeben
    Version,
    /// `--help`
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub db: PathBuf,
    pub addr: SocketAddr,
    /// Mit `--local`: Ordner der Datenbank, wird vor dem Start angelegt
    pub create_dir: Option<PathBuf>,
}

/// Umgebung, aus der die Einstellungen kommen. In Tests nachgebaut.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub spedhub_db: Option<OsString>,
    pub spedhub_addr: Option<String>,
    pub local_app_data: Option<OsString>,
    pub home: Option<OsString>,
}

impl Env {
    pub fn from_process() -> Env {
        let get = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        Env {
            spedhub_db: get("SPEDHUB_DB"),
            spedhub_addr: std::env::var("SPEDHUB_ADDR").ok().filter(|v| !v.is_empty()),
            local_app_data: get("LOCALAPPDATA"),
            home: get("HOME"),
        }
    }
}

pub const HELP: &str = "spedhub-server [--local] [--version]

  --local     Datenbank im Benutzerordner (%LOCALAPPDATA%\\SpedHub\\server, unter Linux
              ~/.local/share/SpedHub/server), Adresse 127.0.0.1:8787
  --version   Version ausgeben

SPEDHUB_DB und SPEDHUB_ADDR gehen immer vor.";

/// Ordner für die Datenbank mit `--local`: `%LOCALAPPDATA%\SpedHub\server`, ohne
/// `LOCALAPPDATA` `$HOME/.local/share/SpedHub/server`.
pub fn local_data_dir(env: &Env) -> Option<PathBuf> {
    let base = match (&env.local_app_data, &env.home) {
        (Some(l), _) => PathBuf::from(l),
        (None, Some(h)) => PathBuf::from(h).join(".local").join("share"),
        (None, None) => return None,
    };
    Some(base.join("SpedHub").join("server"))
}

/// Liest die Kommandozeile (ohne den Programmnamen).
pub fn parse<I, S>(args: I, env: &Env) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut local = false;
    for arg in args {
        match arg.as_ref() {
            "--local" => local = true,
            "--version" | "-V" => return Ok(Command::Version),
            "--help" | "-h" => return Ok(Command::Help),
            other => return Err(format!("Unbekannte Option {other}\n\n{HELP}")),
        }
    }
    let addr_text = env.spedhub_addr.as_deref().unwrap_or(DEFAULT_ADDR);
    let addr: SocketAddr = addr_text.parse().map_err(|_| {
        format!("SPEDHUB_ADDR muss wie 127.0.0.1:8787 aussehen, ist aber {addr_text}")
    })?;
    let (db, create_dir) = match (&env.spedhub_db, local) {
        (Some(db), _) => (PathBuf::from(db), None),
        (None, true) => {
            let dir = local_data_dir(env).ok_or_else(|| {
                "Für --local muss LOCALAPPDATA oder HOME gesetzt sein".to_string()
            })?;
            (dir.join(DEFAULT_DB), Some(dir))
        }
        (None, false) => (PathBuf::from(DEFAULT_DB), None),
    };
    Ok(Command::Run(Config {
        db,
        addr,
        create_dir,
    }))
}
