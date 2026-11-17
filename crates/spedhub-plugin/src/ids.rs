//! Eindeutige `event_id`s, auch über Neustarts des Spiels hinweg.
//!
//! Eine ID hat die Form `<installation>-<start>-<nummer>`:
//!
//! - `installation`: 32 Hex-Zeichen Zufall, einmal erzeugt und in `plugin-state.json` gemerkt.
//!   Damit kollidieren auch zwei Fahrer derselben Firma nicht.
//! - `start`: Zähler, der bei jedem Start des Plugins steigt und gespeichert wird, bevor die
//!   erste ID vergeben wird.
//! - `nummer`: zählt innerhalb eines Starts hoch.
//!
//! Lässt sich der Zustand nicht speichern, käme beim nächsten Start derselbe Zähler wieder.
//! Dann hängt an `start` zusätzlich eine Zufallszahl, zum Beispiel `7r1f3a...`.
//! Ist die Datei kaputt, gibt es eine neue Kennung, die alten IDs können also nicht
//! wiederkommen.

use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::RandomState,
    fs,
    hash::{BuildHasher, Hasher},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    client_id: String,
    starts: u64,
}

pub struct IdGenerator {
    prefix: String,
    next: AtomicU64,
}

/// Was beim Start passiert ist, für das Log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartNote {
    /// Alles normal
    Ok,
    /// Zustand fehlte, neue Kennung angelegt
    Created,
    /// Zustand war kaputt, neue Kennung angelegt
    Replaced(String),
    /// Zustand ließ sich nicht speichern, Zufall angehängt
    NotSaved(String),
}

impl IdGenerator {
    /// Liest den Zustand, zählt den Start hoch und speichert ihn.
    pub fn start(state_path: &Path) -> (IdGenerator, StartNote) {
        let (old, mut note) = match fs::read_to_string(state_path) {
            Ok(text) => match serde_json::from_str::<State>(&text) {
                Ok(s) if valid_client_id(&s.client_id) => (Some(s), StartNote::Ok),
                Ok(_) => (None, StartNote::Replaced("ungültige Kennung".into())),
                Err(e) => (None, StartNote::Replaced(e.to_string())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, StartNote::Created),
            Err(e) => (None, StartNote::Replaced(e.to_string())),
        };
        let state = State {
            client_id: old
                .as_ref()
                .map_or_else(new_client_id, |s| s.client_id.clone()),
            starts: old.map_or(1, |s| s.starts.saturating_add(1)),
        };
        let mut prefix = format!("{}-{}", state.client_id, state.starts);
        if let Err(e) = save(state_path, &state) {
            prefix.push_str(&format!("r{:016x}", random_u64()));
            note = StartNote::NotSaved(e);
        }
        (
            IdGenerator {
                prefix,
                next: AtomicU64::new(1),
            },
            note,
        )
    }

    /// Nächste ID. Thread-sicher und ohne Dateizugriff, darf also im Callback laufen.
    pub fn next(&self) -> String {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        format!("{}-{n}", self.prefix)
    }
}

fn save(path: &Path, state: &State) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string(state).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

fn valid_client_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn new_client_id() -> String {
    format!("{:016x}{:016x}", random_u64(), random_u64())
}

/// Zufall aus der Standardbibliothek: `RandomState` wird je Instanz mit Schlüsseln aus dem
/// Zufallsgenerator des Betriebssystems belegt. Für IDs reicht das, Kryptografie ist es nicht.
fn random_u64() -> u64 {
    let mut h = RandomState::new().build_hasher();
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    );
    h.finish()
}
