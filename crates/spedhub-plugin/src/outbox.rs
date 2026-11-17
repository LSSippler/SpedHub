//! Postausgang: Jedes Event landet hier, bevor es gesendet wird, und bleibt, bis der Server
//! es angenommen hat. Eine Zeile JSON je Event. So geht auch bei einem Absturz des Spiels
//! nichts verloren. Doppelt senden schadet nicht, der Server bucht jede `event_id` nur einmal.
//!
//! Lehnt der Server ein Event dauerhaft ab, kommt es mit Grund in eine zweite Datei.
//! Dort bleibt es zur Fehlersuche und wird nicht wieder gesendet.

use crate::event::Booking;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Die Firma zum Zeitpunkt des Events. Wechselt der Fahrer die Firma, bleibt das Event
    /// bei der alten.
    pub company_id: i64,
    pub booking: Booking,
}

#[derive(Debug, Serialize)]
struct RejectedLine<'a> {
    reason: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry: Option<&'a Entry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<&'a str>,
}

pub struct Outbox {
    path: PathBuf,
    rejected_path: PathBuf,
    entries: Vec<Entry>,
}

impl Outbox {
    /// Liest den Postausgang. Kaputte Zeilen wandern in die Ablehnungen. Gibt Meldungen für
    /// das Log zurück.
    pub fn open(
        path: impl Into<PathBuf>,
        rejected_path: impl Into<PathBuf>,
    ) -> (Outbox, Vec<String>) {
        let mut outbox = Outbox {
            path: path.into(),
            rejected_path: rejected_path.into(),
            entries: Vec::new(),
        };
        let mut notes = Vec::new();
        let text = match fs::read_to_string(&outbox.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (outbox, notes),
            Err(e) => {
                notes.push(format!(
                    "Postausgang {} lässt sich nicht lesen: {e}",
                    outbox.path.display()
                ));
                return (outbox, notes);
            }
        };
        let mut broken = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Entry>(line) {
                Ok(entry) => outbox.entries.push(entry),
                Err(e) => {
                    broken += 1;
                    outbox.write_rejected(&RejectedLine {
                        reason: &format!("kaputte Zeile im Postausgang: {e}"),
                        entry: None,
                        raw: Some(line),
                    });
                }
            }
        }
        if broken > 0 {
            notes.push(format!(
                "{broken} kaputte Zeilen aus dem Postausgang nach {} verschoben",
                outbox.rejected_path.display()
            ));
            if let Err(e) = outbox.rewrite() {
                notes.push(e);
            }
        }
        if !outbox.entries.is_empty() {
            notes.push(format!(
                "{} Events aus dem Postausgang warten auf den Versand",
                outbox.entries.len()
            ));
        }
        (outbox, notes)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hängt ein Event an, zuerst in der Datei, dann im Speicher. Scheitert die Datei, bleibt
    /// es trotzdem im Speicher und der Fehler kommt zurück.
    pub fn push(&mut self, entry: Entry) -> Result<(), String> {
        let result = append_line(&self.path, &entry);
        self.entries.push(entry);
        result
    }

    /// Nimmt die ersten `count` Events heraus, nachdem sie erledigt sind.
    pub fn remove_first(&mut self, count: usize) -> Result<(), String> {
        let count = count.min(self.entries.len());
        if count == 0 {
            return Ok(());
        }
        self.entries.drain(..count);
        self.rewrite()
    }

    /// Legt ein Event mit Grund in die Ablehnungen.
    pub fn reject(&self, entry: &Entry, reason: &str) {
        self.write_rejected(&RejectedLine {
            reason,
            entry: Some(entry),
            raw: None,
        });
    }

    fn write_rejected(&self, line: &RejectedLine<'_>) {
        let _ = append_line(&self.rejected_path, line);
    }

    /// Schreibt den Postausgang neu, über eine temporäre Datei.
    fn rewrite(&self) -> Result<(), String> {
        let mut text = String::new();
        for e in &self.entries {
            if let Ok(line) = serde_json::to_string(e) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        fs::write(&tmp, text)
            .and_then(|()| fs::rename(&tmp, &self.path))
            .map_err(|e| {
                let _ = fs::remove_file(&tmp);
                format!(
                    "Postausgang {} lässt sich nicht schreiben: {e}",
                    self.path.display()
                )
            })
    }
}

fn append_line(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut line = serde_json::to_string(value).map_err(|e| e.to_string())?;
    line.push('\n');
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| format!("{} lässt sich nicht schreiben: {e}", path.display()))
}

/// Liest einen Postausgang nur zum Nachsehen, zum Beispiel in Tests.
pub fn read_entries(path: &Path) -> Vec<Entry> {
    fs::read_to_string(path)
        .map(|t| {
            t.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}
