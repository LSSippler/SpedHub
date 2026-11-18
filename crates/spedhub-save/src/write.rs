//! Spielstände sicher schreiben.
//!
//! Ablauf für jede Datei:
//! 1. Integritätsprüfung. Schlägt sie fehl, wird gar nichts geschrieben.
//! 2. Text erzeugen und probeweise wieder einlesen.
//! 3. In eine temporäre Datei daneben schreiben und zurücklesen.
//! 4. Erst wenn das für alle Dateien geklappt hat: bestehende Datei als Backup kopieren,
//!    temporäre Datei an ihren Platz verschieben.
//!
//! Scheitert Schritt 4 mittendrin, werden die schon ersetzten Dateien aus dem Backup
//! zurückgeholt.

use crate::check::check;
use crate::error::{Error, Result};
use crate::sii::Save;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Was beim Schreiben einer Datei passiert ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub path: PathBuf,
    /// Kopie der vorherigen Datei, falls es eine gab
    pub backup: Option<PathBuf>,
}

/// Schreibt einen Spielstand als Klartext. Siehe Modulbeschreibung.
pub fn write_save(path: impl AsRef<Path>, save: &Save) -> Result<Written> {
    let mut written = write_saves(&[(path.as_ref(), save)])?;
    written
        .pop()
        .ok_or_else(|| Error::Structure("nichts geschrieben".into()))
}

/// Schreibt mehrere Dateien gemeinsam, zum Beispiel `game.sii` und `info.sii`. Entweder
/// werden alle geschrieben oder keine.
pub fn write_saves(items: &[(&Path, &Save)]) -> Result<Vec<Written>> {
    // 1. und 2.: prüfen und Text erzeugen, noch ohne die Platte anzufassen
    let mut texts = Vec::with_capacity(items.len());
    for (path, save) in items {
        let report = check(save);
        if !report.is_ok() {
            return Err(Error::Integrity(report.problems));
        }
        let text = save.dump();
        let again = Save::parse(&text)?;
        if again != **save || again.dump() != text {
            return Err(Error::Structure(format!(
                "{} ließe sich nicht unverändert wieder einlesen",
                path.display()
            )));
        }
        texts.push(text);
    }

    // 3.: temporäre Dateien
    let mut temps: Vec<PathBuf> = Vec::with_capacity(items.len());
    for ((path, _), text) in items.iter().zip(&texts) {
        let tmp = sibling(path, "spedhub-tmp");
        let result = write_and_verify(&tmp, text.as_bytes());
        temps.push(tmp);
        if let Err(e) = result {
            remove_all(&temps);
            return Err(e);
        }
    }

    // 4.: Backup und ersetzen
    let mut done: Vec<Written> = Vec::with_capacity(items.len());
    for ((path, _), tmp) in items.iter().zip(&temps) {
        match replace(path, tmp) {
            Ok(w) => done.push(w),
            Err(e) => {
                roll_back(&done);
                remove_all(&temps);
                return Err(e);
            }
        }
    }
    Ok(done)
}

pub(crate) fn write_and_verify(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data).map_err(|e| Error::io(path, &e))?;
    let back = fs::read(path).map_err(|e| Error::io(path, &e))?;
    if back != data {
        return Err(Error::Io {
            path: path.into(),
            message: "die geschriebene Datei weicht vom Inhalt ab".into(),
        });
    }
    Ok(())
}

fn replace(path: &Path, tmp: &Path) -> Result<Written> {
    let backup = if path.exists() {
        let b = backup_path(path);
        fs::copy(path, &b).map_err(|e| Error::io(&b, &e))?;
        Some(b)
    } else {
        None
    };
    // Unter Windows ersetzt rename eine vorhandene Datei ebenfalls.
    fs::rename(tmp, path).map_err(|e| Error::io(path, &e))?;
    Ok(Written {
        path: path.into(),
        backup,
    })
}

fn roll_back(done: &[Written]) {
    for w in done {
        match &w.backup {
            Some(b) => {
                let _ = fs::copy(b, &w.path);
            }
            None => {
                let _ = fs::remove_file(&w.path);
            }
        }
    }
}

fn remove_all(paths: &[PathBuf]) {
    for p in paths {
        let _ = fs::remove_file(p);
    }
}

pub(crate) fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

/// `game.sii.spedhub-<zeit>.bak`, bei Bedarf mit Zähler, damit nie ein Backup überschrieben wird.
fn backup_path(path: &Path) -> PathBuf {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let first = sibling(path, &format!("spedhub-{secs}.bak"));
    if !first.exists() {
        return first;
    }
    (2u32..)
        .map(|n| sibling(path, &format!("spedhub-{secs}-{n}.bak")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}
