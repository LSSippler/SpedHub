//! Kleines Log in eine Datei mit Rotation. Wird die Datei zu groß, wird sie zu `<name>.1`
//! und eine neue beginnt. So bleiben höchstens zwei Dateien mit zusammen etwa 2 x `max_bytes`.
//!
//! Schreibfehler werden geschluckt: Ein Log darf das Spiel nie stören.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// Standardgröße, ab der rotiert wird.
pub const DEFAULT_MAX_BYTES: u64 = 256 * 1024;

pub struct Logger {
    path: PathBuf,
    max_bytes: u64,
    state: Mutex<Option<(File, u64)>>,
}

impl Logger {
    pub fn new(path: impl Into<PathBuf>, max_bytes: u64) -> Logger {
        Logger {
            path: path.into(),
            max_bytes,
            state: Mutex::new(None),
        }
    }

    /// Ein Logger, der nichts schreibt.
    pub fn disabled() -> Logger {
        Logger::new(PathBuf::new(), 0)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Schreibt eine Zeile mit Zeitstempel (UTC).
    pub fn write(&self, message: &str) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        let line = format!("{} {}\n", timestamp(SystemTime::now()), one_line(message));
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        if state.is_none() {
            *state = self.open();
        }
        let needs_rotation = state
            .as_ref()
            .is_some_and(|(_, size)| size + line.len() as u64 > self.max_bytes);
        if needs_rotation {
            *state = None;
            let _ = fs::rename(&self.path, rotated(&self.path));
            *state = self.open();
        }
        if let Some((file, size)) = state.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            *size += line.len() as u64;
        }
    }

    fn open(&self) -> Option<(File, u64)> {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok()?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Some((file, size))
    }
}

/// Pfad der rotierten Datei, zum Beispiel `plugin.log.1`.
pub fn rotated(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".1");
    path.with_file_name(name)
}

/// Zeilenumbrüche würden das Log zerreißen.
fn one_line(message: &str) -> String {
    message.replace(['\r', '\n'], " ")
}

/// `2026-11-17 08:30:00` in UTC, ohne zusätzliche Abhängigkeit.
pub fn timestamp(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// Tage seit 1970-01-01 in ein Datum, nach Howard Hinnant.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn zeitstempel() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01 00:00:00");
        let t = UNIX_EPOCH + Duration::from_secs(1_794_131_316);
        assert_eq!(timestamp(t), "2026-11-08 09:48:36");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29
        assert_eq!(timestamp(leap), "2000-02-29 00:00:00");
    }
}
