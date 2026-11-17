//! Lkw zwischen Spielstand und Server bewegen. Alles am Spielstand macht `spedhub-save`.
//!
//! - [`new_truck_from_save`]: Lkw aus einem Spielstand als Paket für den Fuhrpark
//! - [`deploy_truck`]: geliehenen Lkw als neuen Speicherplatz ins Profil schreiben
//! - [`read_for_return`]: Kilometerstand und Zustand für die Rückgabe lesen

use crate::client::{LeasedTruck, NewTruck};
use crate::error::{Error, Result};
use crate::profiles::{SaveSlot, newest_slot, next_free_slot_id};
use crate::system::GameCheck;
use serde_json::Value;
use spedhub_save::{
    InsertOptions, Save, TruckPackage, TruckState, export_truck, insert_truck, plate_text,
    set_info_file_time, set_info_name, truck_state, write_saves,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Präfix für den Namen des neuen Speicherplatzes.
pub const SLOT_NAME_PREFIX: &str = "SpedHub: ";

/// Ein Lkw in einem Spielstand, zur Auswahl in der Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTruck {
    /// Lesbares Kennzeichen, zum Beispiel `SPED 001`
    pub plate: String,
    pub model: Option<String>,
    pub active: bool,
    pub own: bool,
}

/// Alle Lkw, die der Spieler im Spielstand besitzt oder gerade fährt.
pub fn trucks_in_save(game_sii: &Path) -> Result<Vec<SaveTruck>> {
    let (save, _) = Save::read(game_sii)?;
    Ok(spedhub_save::list_trucks(&save)?
        .into_iter()
        .map(|t| SaveTruck {
            plate: plate_text(&t.plate),
            model: t.model,
            active: t.active,
            own: t.own,
        })
        .collect())
}

/// Macht aus einem Lkw im Spielstand die Anfrage für `POST /api/companies/{id}/trucks`.
pub fn new_truck_from_save(game_sii: &Path, plate: &str) -> Result<NewTruck> {
    let (save, _) = Save::read(game_sii)?;
    let package = export_truck(&save, Some(plate))?;
    let state =
        serde_json::to_value(&package.state).map_err(|e| Error::Other(format!("Zustand: {e}")))?;
    Ok(NewTruck {
        plate: plate_text(&package.license_plate),
        model: package.model.clone(),
        odometer_km: i64::from(package.state.odometer),
        package: package.to_json()?,
        state,
    })
}

/// Das Paket für den Spielstand aus dem, was der Server beim Ausleihen liefert. Kilometerstand
/// und Zustand vom Server gelten vor den Werten im Bauplan: Der Server ist die Wahrheit.
pub fn package_from_checkout(truck: &LeasedTruck) -> Result<TruckPackage> {
    if truck.package.trim().is_empty() {
        return Err(Error::Other(format!(
            "Zum Lkw {} hat der Server keinen Bauplan. Er wurde ohne Spielstand angelegt und lässt sich nicht einsetzen",
            truck.plate
        )));
    }
    let mut package = TruckPackage::from_json(&truck.package)?;
    if let Ok(state) = serde_json::from_value::<TruckState>(truck.state.clone()) {
        package.state = state;
    }
    package.state.odometer = u32::try_from(truck.odometer_km.max(0)).unwrap_or(u32::MAX);
    Ok(package)
}

/// Was beim Einsetzen passiert ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployed {
    /// Der neue Speicherplatz
    pub slot_id: String,
    pub slot_dir: PathBuf,
    /// Name im Ladebildschirm, `SpedHub: <Kennzeichen>`
    pub name: String,
    /// Der Speicherplatz, aus dem der neue entstanden ist
    pub source_slot: String,
    /// Kopie des Ausgangs-Speicherplatzes
    pub backup_dir: PathBuf,
}

/// Setzt einen Lkw in den neuesten Spielstand des Profils ein und legt das Ergebnis als neuen
/// Speicherplatz mit eigener `info.sii` an. Nichts Vorhandenes wird überschrieben.
///
/// Reihenfolge: Spiel prüfen, neuesten Speicherplatz lesen, Lkw einsetzen (auf einer Kopie),
/// Ausgangs-Speicherplatz sichern, neuen Speicherplatz schreiben. Läuft das Spiel oder lässt
/// sich das nicht prüfen, wird nichts geschrieben.
pub fn deploy_truck(
    profile_dir: &Path,
    package: &TruckPackage,
    backup_root: &Path,
    game: &dyn GameCheck,
    now: SystemTime,
) -> Result<Deployed> {
    ensure_game_closed(game)?;
    let source = newest_slot(profile_dir)?;
    let (mut save, _) = Save::read(source.game())?;
    let (mut info, _) = Save::read(source.info())?;

    insert_truck(&mut save, package, &InsertOptions::default())?;
    let name = format!("{SLOT_NAME_PREFIX}{}", plate_text(&package.license_plate));
    set_info_name(&mut info, &name)?;
    set_info_file_time(&mut info, unix_seconds(now))?;

    let backup_dir = backup_slot(&source, profile_dir, backup_root, now)?;

    // Kurz vor dem Schreiben noch einmal: Das Spiel könnte inzwischen gestartet sein.
    ensure_game_closed(game)?;
    let slot_id = next_free_slot_id(profile_dir);
    let slot_dir = profile_dir.join("save").join(&slot_id);
    fs::create_dir_all(&slot_dir).map_err(|e| Error::io(&slot_dir, &e))?;
    let (game_path, info_path) = (slot_dir.join("game.sii"), slot_dir.join("info.sii"));
    let items: [(&Path, &Save); 2] = [(&game_path, &save), (&info_path, &info)];
    if let Err(e) = write_saves(&items) {
        let _ = fs::remove_dir_all(&slot_dir);
        return Err(e.into());
    }
    Ok(Deployed {
        slot_id,
        slot_dir,
        name,
        source_slot: source.id,
        backup_dir,
    })
}

/// Was der Spielstand für die Rückgabe hergibt.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnData {
    pub odometer_km: i64,
    /// Zustand als JSON für den Server
    pub state: Value,
    /// Aus welchem Speicherplatz
    pub slot: String,
}

/// Liest Kilometerstand und Zustand des Lkw mit diesem Kennzeichen aus dem neuesten Spielstand.
/// `None`, wenn der Lkw dort nicht vorkommt. Dann gibt der Fahrer den Kilometerstand von Hand ein.
pub fn read_for_return(profile_dir: &Path, plate: &str) -> Result<Option<ReturnData>> {
    let slot = newest_slot(profile_dir)?;
    let (save, _) = Save::read(slot.game())?;
    match truck_state(&save, Some(plate)) {
        Ok(state) => Ok(Some(ReturnData {
            odometer_km: i64::from(state.odometer),
            state: serde_json::to_value(&state).map_err(|e| Error::Other(e.to_string()))?,
            slot: slot.id,
        })),
        Err(spedhub_save::Error::TruckNotFound(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn ensure_game_closed(game: &dyn GameCheck) -> Result<()> {
    match game.is_running() {
        Ok(false) => Ok(()),
        Ok(true) => Err(Error::GameRunning),
        Err(e) => Err(Error::Other(format!(
            "Ob Euro Truck Simulator 2 läuft, ließ sich nicht prüfen ({e}). Zur Sicherheit wurde nichts geschrieben"
        ))),
    }
}

/// Kopiert alle Dateien des Speicherplatzes nach
/// `<backup_root>\<Profilordner>\<Zeit>-<Speicherplatz>\`.
fn backup_slot(
    slot: &SaveSlot,
    profile_dir: &Path,
    backup_root: &Path,
    now: SystemTime,
) -> Result<PathBuf> {
    let profile = profile_dir
        .file_name()
        .map_or_else(|| "profil".into(), |n| n.to_string_lossy().into_owned());
    let base = backup_root.join(profile);
    let stamp = unix_seconds(now);
    let mut dir = base.join(format!("{stamp}-{}", slot.id));
    let mut n = 2;
    while dir.exists() {
        dir = base.join(format!("{stamp}-{}-{n}", slot.id));
        n += 1;
    }
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, &e))?;
    let entries = fs::read_dir(&slot.dir).map_err(|e| Error::io(&slot.dir, &e))?;
    for entry in entries.flatten() {
        let from = entry.path();
        if from.is_file() {
            let to = dir.join(entry.file_name());
            fs::copy(&from, &to).map_err(|e| Error::io(&to, &e))?;
        }
    }
    Ok(dir)
}

fn unix_seconds(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
