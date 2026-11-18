//! Lkw zwischen Spielstand und Server bewegen. Alles am Spielstand macht `spedhub-save`.
//!
//! - [`new_truck_from_save`]: Lkw aus einem Spielstand als Paket für den Fuhrpark
//! - [`deploy_truck`]: geliehenen Lkw als neuen Speicherplatz ins Profil schreiben
//! - [`prepare_slot`] und [`write_slot`]: dasselbe in zwei Schritten, dazwischen lässt sich
//!   der Kontostand setzen (Geldabgleich)
//! - [`read_for_return`]: Kilometerstand und Zustand für die Rückgabe lesen
//!
//! Kennzeichen: Jeder Firmen-Lkw hat auf dem Server ein eigenes Kennzeichen. Das Paket vom
//! Server bekommt es vor dem Einsetzen ([`package_from_checkout`], [`set_company_plate`]), als
//! selbst vergebenes Kennzeichen mit dem Land aus dem Bauplan. Das Kennzeichen im Bauplan
//! stammt aus dem Spielstand, aus dem der Chef den Lkw übernommen hat, und kann dort einem
//! anderen Lkw gehören. Im Spiel bekam so ein eingesetzter Lkw das Kennzeichen eines anderen
//! Firmen-Lkw.

use crate::client::{LeasedTruck, NewTruck};
use crate::error::{Error, Result};
use crate::profiles::{SaveSlot, newest_slot, next_free_slot_id};
use crate::system::GameCheck;
use serde_json::Value;
use spedhub_save::{
    DEFAULT_MIN_MATCH, InsertOptions, Save, TruckPackage, TruckState, custom_plate, export_truck,
    find_package_truck, info_experience, insert_truck, list_trucks, money, plate::plate_matches,
    plate_country, plate_text, set_info_file_time, set_info_money, set_info_name, set_money,
    truck_state, write_saves,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Präfix für den Namen des neuen Speicherplatzes.
pub const SLOT_NAME_PREFIX: &str = "SpedHub: ";
/// Name eines Speicherplatzes, der nur den Kontostand abgleicht.
pub const SYNC_SLOT_NAME: &str = "SpedHub: Geldabgleich";

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
///
/// `save_plate` findet den Lkw im Spielstand. `company_plate` ist sein Kennzeichen in der
/// Firma, das der Chef festlegt (vorbelegt mit dem Vorschlag des Servers). Das Kennzeichen aus
/// dem Spielstand wird nicht das Firmen-Kennzeichen: Es kann einem anderen Lkw gehören, etwa
/// das Wunschkennzeichen eines Profils. Im Bauplan bleibt es stehen, beim Einsetzen gilt das
/// Kennzeichen vom Server.
pub fn new_truck_from_save(
    game_sii: &Path,
    save_plate: &str,
    company_plate: &str,
) -> Result<NewTruck> {
    let (save, _) = Save::read(game_sii)?;
    let package = export_truck(&save, Some(save_plate))?;
    // Lässt es sich so einsetzen? Sonst lieber jetzt eine Meldung als beim Ausleihen.
    let mut check = package.clone();
    set_company_plate(&mut check, company_plate)?;
    let state =
        serde_json::to_value(&package.state).map_err(|e| Error::Other(format!("Zustand: {e}")))?;
    Ok(NewTruck {
        plate: plate_text(&check.license_plate),
        model: package.model.clone(),
        odometer_km: i64::from(package.state.odometer),
        package: package.to_json()?,
        state,
    })
}

/// Setzt das Kennzeichen eines Firmen-Lkw ins Paket, bevor es in den Spielstand geht: als
/// selbst vergebenes Kennzeichen `<TEXT>|_<land>` im Format des Spiels, mit dem Land aus dem
/// Kennzeichen im Bauplan. `plate` ist das Kennzeichen vom Server, ohne Land.
///
/// Fehler, wenn sich `plate` nicht als eigenes Kennzeichen setzen lässt (erlaubt sind A bis Z,
/// Ziffern, Leerzeichen, Punkt und Bindestrich, höchstens 16 Zeichen) oder der Bauplan kein
/// Land nennt. Der Chef kann das Kennzeichen dann im Dashboard ändern.
pub fn set_company_plate(package: &mut TruckPackage, plate: &str) -> Result<()> {
    let country = plate_country(&package.license_plate).to_string();
    if country.is_empty() {
        return Err(Error::Other(format!(
            "Im Bauplan steht kein Land zum Kennzeichen ({}). Das Kennzeichen {} lässt sich deshalb nicht setzen",
            package.license_plate,
            plate_text(plate)
        )));
    }
    package.license_plate = custom_plate(&plate_text(plate), &country).map_err(|e| {
        Error::Other(format!(
            "Das Kennzeichen {} lässt sich nicht in den Spielstand schreiben: {e}. Der Chef kann es im Dashboard ändern",
            plate_text(plate)
        ))
    })?;
    Ok(())
}

/// Das Paket für den Spielstand aus dem, was der Server beim Ausleihen liefert. Kilometerstand,
/// Zustand und Kennzeichen vom Server gelten vor den Werten im Bauplan: Der Server ist die
/// Wahrheit.
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
    set_company_plate(&mut package, &truck.plate)?;
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

/// Ein neuer Speicherplatz, fertig im Speicher, noch nicht geschrieben.
pub struct PreparedSlot {
    source: SaveSlot,
    game: Save,
    info: Save,
    name: String,
    /// Kontostand im Ausgangs-Speicherplatz
    pub ingame_money: i64,
    /// Erfahrungspunkte aus `info.sii`, `None`, wenn das Feld fehlt oder unlesbar ist
    pub xp: Option<u64>,
    /// Änderungszeit von `game.sii` des Ausgangs-Speicherplatzes, Unix-Sekunden. Der
    /// Kontostand darin enthält alle Ereignisse bis zu diesem Zeitpunkt.
    pub saved_at: i64,
}

impl PreparedSlot {
    /// Aus welchem Speicherplatz der neue entsteht.
    pub fn source_label(&self) -> String {
        self.source.label()
    }

    /// Setzt den Kontostand in `game.sii` und `info.sii`.
    pub fn set_money(&mut self, amount: i64) -> Result<()> {
        set_money(&mut self.game, amount)?;
        set_info_money(&mut self.info, amount)?;
        Ok(())
    }
}

/// Erster Schritt: Spiel prüfen, neuesten Speicherplatz lesen, Kontostand, Erfahrung und
/// Speicherzeit auslesen und, mit `package`, den Lkw einsetzen. Geschrieben wird nichts.
pub fn prepare_slot(
    profile_dir: &Path,
    package: Option<&TruckPackage>,
    game: &dyn GameCheck,
) -> Result<PreparedSlot> {
    ensure_game_closed(game)?;
    let source = newest_slot(profile_dir)?;
    let (mut save, _) = Save::read(source.game())?;
    let (info, _) = Save::read(source.info())?;
    let ingame_money = money(&save)?;
    let xp = info_experience(&info).ok();

    let name = match package {
        None => SYNC_SLOT_NAME.to_string(),
        Some(package) => {
            // Steht der Lkw schon im Spielstand, etwa von einer früheren Leihe, dann womöglich
            // unter einem Kennzeichen, das das Spiel vergeben hat. Die Prüfung auf gleiches
            // Kennzeichen beim Einsetzen greift dann nicht, deshalb hier zusätzlich über die Teile.
            if let Some(m) = find_package_truck(&save, package, DEFAULT_MIN_MATCH)? {
                return Err(Error::Other(format!(
                    "Der Lkw steht schon im neuesten Spielstand ({}), unter dem Kennzeichen {}. Er wird nicht ein zweites Mal eingesetzt",
                    source.label(),
                    plate_text(&m.plate)
                )));
            }
            insert_truck(&mut save, package, &InsertOptions::default())?;
            format!("{SLOT_NAME_PREFIX}{}", plate_text(&package.license_plate))
        }
    };
    let saved_at = unix_seconds(source.modified) as i64;
    Ok(PreparedSlot {
        source,
        game: save,
        info,
        name,
        ingame_money,
        xp,
        saved_at,
    })
}

/// Zweiter Schritt: Namen und Zeit in `info.sii` setzen, Ausgangs-Speicherplatz sichern, kurz
/// vor dem Schreiben noch einmal das Spiel prüfen und den neuen Speicherplatz anlegen.
/// Vorhandenes wird nie überschrieben. Scheitert das Schreiben, wird der neue Ordner entfernt.
pub fn write_slot(
    prepared: PreparedSlot,
    profile_dir: &Path,
    backup_root: &Path,
    game: &dyn GameCheck,
    now: SystemTime,
) -> Result<Deployed> {
    let PreparedSlot {
        source,
        game: save,
        mut info,
        name,
        ..
    } = prepared;
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

/// Setzt einen Lkw in den neuesten Spielstand des Profils ein und legt das Ergebnis als neuen
/// Speicherplatz mit eigener `info.sii` an, ohne den Kontostand anzufassen. Mit Geldabgleich
/// geht das über [`crate::flows::sync_and_write`].
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
    let prepared = prepare_slot(profile_dir, Some(package), game)?;
    write_slot(prepared, profile_dir, backup_root, game, now)
}

/// Wie der Lkw im Spielstand gefunden wurde.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FoundBy {
    Plate,
    /// Über Modell, Teile und Kilometerstand. `share` ist die Übereinstimmung der Teile, 0 bis 1.
    Parts {
        share: f64,
    },
}

/// Was der Spielstand für die Rückgabe hergibt.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnData {
    pub odometer_km: i64,
    /// Zustand als JSON für den Server
    pub state: Value,
    /// Aus welchem Speicherplatz
    pub slot: String,
    pub found_by: FoundBy,
    /// Das Kennzeichen im Spielstand, wenn es nicht mehr das vom Server ist. Das Spiel ersetzt
    /// Kennzeichen manchmal, der Server soll das neue erfahren.
    pub new_plate: Option<String>,
}

/// Liest Kilometerstand und Zustand eines geliehenen Lkw aus dem neuesten Spielstand.
///
/// Zuerst über das Kennzeichen. Liegt das Paket vor, zählt ein Lkw mit dem Kennzeichen nur,
/// wenn auch das Modell passt: Kennzeichen können zwischen Lkw kollidieren, etwa mit einem
/// eigenen Lkw des Fahrers. Findet sich so keiner, oder haben mehrere Lkw das Kennzeichen, und
/// liegt das Paket vor, geht es über Modell, Teile und Kilometerstand
/// ([`spedhub_save::find_package_truck`]). Passen mehrere Lkw gleich gut, ist das ein Fehler.
/// `None`, wenn beides nichts findet. Dann gibt der Fahrer den Kilometerstand von Hand ein.
pub fn read_for_return(
    profile_dir: &Path,
    plate: &str,
    package: Option<&TruckPackage>,
) -> Result<Option<ReturnData>> {
    let slot = newest_slot(profile_dir)?;
    let (save, _) = Save::read(slot.game())?;
    // Mit Paket: Ein Lkw mit dem Kennzeichen, aber einem anderen Modell, ist nicht gemeint
    let model_differs = match package {
        Some(p) => !list_trucks(&save)?.iter().any(|t| {
            plate_matches(&t.plate, plate) && t.model.as_deref() == Some(p.model.as_str())
        }),
        None => false,
    };
    let by_plate = if model_differs {
        Err(spedhub_save::Error::TruckNotFound(plate.to_string()))
    } else {
        truck_state(&save, Some(plate))
    };
    let (state, found_by, new_plate) = match by_plate {
        Ok(state) => (state, FoundBy::Plate, None),
        Err(spedhub_save::Error::TruckNotFound(_) | spedhub_save::Error::AmbiguousPlate(_))
            if package.is_some() =>
        {
            let Some(package) = package else {
                return Ok(None);
            };
            let Some(m) = find_package_truck(&save, package, DEFAULT_MIN_MATCH)? else {
                return Ok(None);
            };
            let now = plate_text(&m.plate);
            let changed = !now.eq_ignore_ascii_case(&plate_text(plate));
            (
                m.state,
                FoundBy::Parts { share: m.share },
                changed.then_some(now),
            )
        }
        Err(spedhub_save::Error::TruckNotFound(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    Ok(Some(ReturnData {
        odometer_km: i64::from(state.odometer),
        state: serde_json::to_value(&state).map_err(|e| Error::Other(e.to_string()))?,
        slot: slot.id,
        found_by,
        new_plate,
    }))
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

/// Unix-Sekunden, so wie sie als `file_time` in `info.sii` stehen.
pub fn unix_seconds(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
