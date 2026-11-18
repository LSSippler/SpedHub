//! Welches ETS2-Profil gerade aktiv ist, aus `Dokumente\Euro Truck Simulator 2\game.log.txt`.
//!
//! Das Telemetry SDK nennt das Profil nicht. Das Spiel schreibt aber beim Wählen eines Profils
//! eine Zeile wie
//!
//! ```text
//! 00:00:12.345 : New profile selected: 'Nordfracht'
//! ```
//!
//! in sein Log. Die letzte solche Zeile gilt, auch nach einem Profilwechsel im laufenden Spiel.
//! Das Plugin liest die Datei nur, es greift nicht in das Spiel ein.
//!
//! - [`last_profile`] holt aus einem Text den letzten Profilnamen. Das Präfix vor
//!   `New profile selected:` ist egal, der Name steht in einfachen Anführungszeichen und darf
//!   Leerzeichen, Umlaute und selbst Anführungszeichen enthalten (es gilt das letzte `'` der
//!   Zeile).
//! - [`LogReader`] merkt sich, wie weit es die Datei gelesen hat, und liest bei jedem Aufruf
//!   nur die neuen Bytes. Das Spiel schreibt die Datei bei jedem Start neu. Ist sie kürzer als
//!   die gemerkte Stelle, hat sich ihr Anfang geändert oder ihre Erstellungszeit, beginnt der
//!   Leser wieder bei 0. Die Änderungszeit taugt dafür nicht, sie ändert sich mit jeder Zeile.
//! - [`ProfileGate`] vergleicht das aktive Profil mit dem Firmenprofil aus `plugin.json`.
//!
//! Die Datei ist UTF-8. Kaputte Bytes werden ersetzt, nie als Fehler behandelt.
//!
//! Namen werden nach dem Trimmen genau verglichen, also auch mit Groß- und Kleinschreibung.
//! Das Spiel unterscheidet Profile `Nordfracht` und `nordfracht` (die Ordnernamen sind die
//! Bytes des Namens als Hex), der Launcher schreibt den Namen so, wie er im Ordnernamen steht.
//! Sollte das Spiel im Log eine andere Schreibweise benutzen als im Ordnernamen, passt nichts
//! mehr zusammen. Das ist im Spiel noch nicht geprüft.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Der Teil der Zeile, nach dem der Profilname kommt.
pub const MARKER: &str = "New profile selected:";

/// So viele Bytes vom Anfang der Datei merkt sich der Leser, um eine neu geschriebene Datei
/// zu erkennen.
const HEAD_BYTES: usize = 64;

/// Der Profilname aus einer Zeile, falls sie eine Profilwahl ist.
pub fn profile_in_line(line: &str) -> Option<String> {
    let rest = line.get(line.find(MARKER)? + MARKER.len()..)?;
    let rest = rest.trim_start().strip_prefix('\'')?;
    let name = rest.get(..rest.rfind('\'')?)?;
    (!name.trim().is_empty()).then(|| name.to_string())
}

/// Der letzte Profilname in einem Text, `None`, wenn keine Zeile eine Profilwahl ist.
pub fn last_profile(text: &str) -> Option<String> {
    text.lines().rev().find_map(profile_in_line)
}

/// Wie [`last_profile`], für rohe Bytes. Kaputtes UTF-8 wird ersetzt.
pub fn last_profile_bytes(bytes: &[u8]) -> Option<String> {
    last_profile(&String::from_utf8_lossy(bytes))
}

/// Liest `game.log.txt` Stück für Stück und merkt sich das zuletzt gewählte Profil.
#[derive(Debug)]
pub struct LogReader {
    path: PathBuf,
    /// Bis hier ist gelesen, in Bytes
    pos: u64,
    /// Rest nach dem letzten Zeilenende. Wird beim nächsten Lesen vervollständigt.
    partial: Vec<u8>,
    /// Die ersten Bytes der Datei, so wie sie beim Lesen waren
    head: Vec<u8>,
    created: Option<SystemTime>,
    /// Letzte Profilwahl in vollständigen Zeilen
    current: Option<String>,
}

impl LogReader {
    pub fn new(path: impl Into<PathBuf>) -> LogReader {
        LogReader {
            path: path.into(),
            pos: 0,
            partial: Vec::new(),
            head: Vec::new(),
            created: None,
            current: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bis hier ist die Datei gelesen.
    pub fn position(&self) -> u64 {
        self.pos
    }

    fn reset(&mut self) {
        self.pos = 0;
        self.partial.clear();
        self.head.clear();
        self.created = None;
        self.current = None;
    }

    /// Liest, was seit dem letzten Aufruf dazugekommen ist, und liefert das aktive Profil.
    /// Fehlt die Datei oder lässt sie sich nicht lesen, ist das Profil unbekannt.
    pub fn poll(&mut self) -> Option<String> {
        if self.read_new().is_err() {
            self.reset();
            return None;
        }
        // Eine Zeile ohne Zeilenende zählt schon, bleibt aber für das nächste Mal liegen
        profile_in_line(&String::from_utf8_lossy(&self.partial)).or_else(|| self.current.clone())
    }

    fn read_new(&mut self) -> std::io::Result<()> {
        let mut file = File::open(&self.path)?;
        let meta = file.metadata()?;
        let len = meta.len();
        let created = meta.created().ok();
        let mut restarted = len < self.pos;
        if let (Some(old), Some(new)) = (self.created, created)
            && old != new
        {
            restarted = true;
        }
        if !restarted && self.pos > 0 && !self.head.is_empty() {
            let mut head = vec![0u8; self.head.len()];
            file.read_exact(&mut head)?;
            restarted = head != self.head;
        }
        if restarted {
            self.reset();
        }
        self.created = created;
        if len == self.pos {
            return Ok(());
        }
        file.seek(SeekFrom::Start(self.pos))?;
        let mut fresh = Vec::new();
        file.take(len - self.pos).read_to_end(&mut fresh)?;
        self.pos += fresh.len() as u64;
        if self.head.len() < HEAD_BYTES {
            let missing = HEAD_BYTES - self.head.len();
            self.head.extend(fresh.iter().take(missing).copied());
        }
        self.partial.extend_from_slice(&fresh);
        if let Some(end) = self.partial.iter().rposition(|b| *b == b'\n') {
            let rest = self.partial.split_off(end + 1);
            let complete = std::mem::replace(&mut self.partial, rest);
            if let Some(name) = last_profile_bytes(&complete) {
                self.current = Some(name);
            }
        }
        Ok(())
    }
}

/// Ob das aktive Profil das Firmenprofil ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileState {
    /// Das aktive Profil ist das Firmenprofil
    Company(String),
    /// Ein anderes Profil ist aktiv
    Other(String),
    /// Kein Profil bekannt: `game.log.txt` fehlt, ist nicht bekannt oder nennt noch keins
    Unknown,
    /// In `plugin.json` steht kein Firmenprofil, der Launcher muss sie neu schreiben
    NotConfigured,
}

impl ProfileState {
    /// Name des aktiven Profils, wenn es das Firmenprofil ist.
    pub fn company(&self) -> Option<&str> {
        match self {
            ProfileState::Company(name) => Some(name),
            _ => None,
        }
    }

    /// Kurz für das Log.
    pub fn describe(&self, company: Option<&str>) -> String {
        let company = company.unwrap_or_default();
        match self {
            ProfileState::Company(name) => format!("Aktives Profil '{name}' ist das Firmenprofil"),
            ProfileState::Other(name) => format!(
                "Aktives Profil '{name}' ist nicht das Firmenprofil '{company}'. Ereignisse und Herzschläge gehen nicht raus"
            ),
            ProfileState::Unknown => format!(
                "Aktives Profil unbekannt, Firmenprofil ist '{company}'. Ereignisse und Herzschläge gehen nicht raus, bis das Spiel ein Profil meldet"
            ),
            ProfileState::NotConfigured => "plugin.json nennt kein Firmenprofil (profile_name). Der Launcher muss die Konfiguration neu schreiben: Launcher öffnen, Firmenprofil wählen, Spiel neu starten. Bis dahin sendet das Plugin nichts".into(),
        }
    }
}

/// Vergleicht das aktive Profil mit dem Firmenprofil.
#[derive(Debug)]
pub struct ProfileGate {
    company: Option<String>,
    reader: Option<LogReader>,
    /// Zuletzt gemeldeter Zustand, damit das Log nur Wechsel zeigt
    last: Option<ProfileState>,
}

impl ProfileGate {
    /// `company` ist das Firmenprofil aus `plugin.json`, `game_log` der Pfad zu
    /// `game.log.txt`. Fehlt der Pfad, ist das Profil immer unbekannt.
    pub fn new(company: Option<&str>, game_log: Option<&Path>) -> ProfileGate {
        ProfileGate {
            company: company
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string),
            reader: game_log.map(LogReader::new),
            last: None,
        }
    }

    pub fn company(&self) -> Option<&str> {
        self.company.as_deref()
    }

    /// Liest neue Zeilen und liefert den Zustand. Der zweite Wert ist eine Zeile fürs Log,
    /// wenn sich der Zustand seit dem letzten Aufruf geändert hat.
    pub fn check(&mut self) -> (ProfileState, Option<String>) {
        let state = match (&self.company, &mut self.reader) {
            (None, _) => ProfileState::NotConfigured,
            (Some(_), None) => ProfileState::Unknown,
            (Some(company), Some(reader)) => match reader.poll() {
                None => ProfileState::Unknown,
                Some(active) if active.trim() == company => {
                    ProfileState::Company(active.trim().to_string())
                }
                Some(active) => ProfileState::Other(active.trim().to_string()),
            },
        };
        let note =
            (self.last.as_ref() != Some(&state)).then(|| state.describe(self.company.as_deref()));
        self.last = Some(state.clone());
        (state, note)
    }
}
