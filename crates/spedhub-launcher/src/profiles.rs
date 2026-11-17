//! ETS2-Profile und ihre Speicherplätze finden.
//!
//! ```text
//! Dokumente\Euro Truck Simulator 2\
//!   profiles\<Profilname als Hex>\save\<Speicherplatz>\game.sii, info.sii
//!   steam_profiles\<Profilname als Hex>\save\...
//! ```
//!
//! Profile unter `steam_profiles` werden mit Steam Cloud abgeglichen. Steam kann dort einen
//! bearbeiteten Spielstand mit einer älteren Fassung aus der Cloud überschreiben.

use crate::error::{Error, Result};
use spedhub_save::Save;
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const PROFILES: &str = "profiles";
pub const STEAM_PROFILES: &str = "steam_profiles";

/// Ein Profil im Dokumente-Ordner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Lesbarer Name. Lässt sich der Ordnername nicht als Hex lesen, steht hier der Ordnername.
    pub name: String,
    /// Ordnername, also der Profilname als Hex
    pub folder: String,
    pub dir: PathBuf,
    /// Liegt unter `steam_profiles`, wird also mit Steam Cloud abgeglichen
    pub steam_cloud: bool,
}

/// Ein Speicherplatz in `save\`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSlot {
    /// Ordnername, zum Beispiel `autosave` oder `3`
    pub id: String,
    pub dir: PathBuf,
    /// Name aus `info.sii`, leer bei Autosaves oder wenn `info.sii` fehlt
    pub name: String,
    /// Zeitpunkt der letzten Änderung von `game.sii`
    pub modified: SystemTime,
}

impl SaveSlot {
    pub fn game(&self) -> PathBuf {
        self.dir.join("game.sii")
    }

    pub fn info(&self) -> PathBuf {
        self.dir.join("info.sii")
    }

    /// Text für die Oberfläche: Name, sonst eine Bezeichnung für die festen Plätze.
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        let fixed = match self.id.as_str() {
            "autosave" => "Autosave",
            "autosave_job" => "Autosave nach Auftrag",
            "quicksave" => "Schnellspeicher",
            "multiplayer_backup" => "Sicherung aus dem Convoy",
            id if id.starts_with("autosave_drive") => "Autosave während der Fahrt",
            _ => "",
        };
        if fixed.is_empty() {
            format!("Speicherplatz {}", self.id)
        } else {
            format!("{fixed} ({})", self.id)
        }
    }
}

/// Liest einen Ordnernamen als Hex-kodierten UTF-8-Namen. `None`, wenn das nicht geht.
pub fn decode_hex_name(folder: &str) -> Option<String> {
    if folder.is_empty() || !folder.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..folder.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(folder.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// Profilname als Ordnername, so wie das Spiel ihn anlegt (Großbuchstaben).
pub fn encode_hex_name(name: &str) -> String {
    name.bytes().map(|b| format!("{b:02X}")).collect()
}

/// Alle Profile in `profiles` und `steam_profiles`, sortiert nach Name.
pub fn find_profiles(ets2_dir: &Path) -> Vec<Profile> {
    let mut profiles = Vec::new();
    for (sub, steam_cloud) in [(PROFILES, false), (STEAM_PROFILES, true)] {
        let Ok(entries) = fs::read_dir(ets2_dir.join(sub)) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let folder = entry.file_name().to_string_lossy().into_owned();
            let name = decode_hex_name(&folder).unwrap_or_else(|| folder.clone());
            profiles.push(Profile {
                name,
                folder,
                dir,
                steam_cloud,
            });
        }
    }
    profiles.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.steam_cloud.cmp(&b.steam_cloud))
    });
    profiles
}

/// Alle Speicherplätze mit `game.sii`, der neueste zuerst.
pub fn list_slots(profile_dir: &Path) -> Vec<SaveSlot> {
    let Ok(entries) = fs::read_dir(profile_dir.join("save")) else {
        return Vec::new();
    };
    let mut slots: Vec<SaveSlot> = entries
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            let modified = fs::metadata(dir.join("game.sii")).ok()?.modified().ok()?;
            let name = Save::read(dir.join("info.sii"))
                .ok()
                .and_then(|(info, _)| spedhub_save::info_name(&info).ok())
                .unwrap_or_default();
            Some(SaveSlot {
                id: entry.file_name().to_string_lossy().into_owned(),
                dir,
                name,
                modified,
            })
        })
        .collect();
    slots.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
    slots
}

/// Der neueste Speicherplatz, gemessen an `game.sii`.
pub fn newest_slot(profile_dir: &Path) -> Result<SaveSlot> {
    list_slots(profile_dir).into_iter().next().ok_or_else(|| {
        Error::Other(format!(
            "Im Profil {} gibt es noch keinen Spielstand. Bitte einmal im Spiel speichern",
            profile_dir.display()
        ))
    })
}

/// Nächster freier Ordner für einen eigenen Speicherplatz: die kleinste Zahl ab 1, die noch
/// nicht vergeben ist. So nummeriert auch das Spiel seine Speicherplätze.
pub fn next_free_slot_id(profile_dir: &Path) -> String {
    let save = profile_dir.join("save");
    (1u32..)
        .map(|n| n.to_string())
        .find(|id| !save.join(id).exists())
        .unwrap_or_else(|| "spedhub".into())
}
