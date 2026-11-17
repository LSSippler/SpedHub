//! Lkw exportieren, einsetzen und entfernen, Zustand und Kennzeichen lesen und setzen.
//!
//! Alle Funktionen, die einen Spielstand ändern, arbeiten auf einer Kopie. Erst wenn alles
//! geklappt hat und die Integritätsprüfung sauber ist, wird die Kopie übernommen. Bei einem
//! Fehler bleibt der Spielstand also so, wie er war.

use crate::check::check;
use crate::error::{Error, Result};
use crate::layout::{
    self, CITY_PREFIX, NULL, driver_ai, game_progress as gp, garage as g, player as pl,
    player_job as pj, player_vehicles as pv, profit_log, vehicle as v,
};
use crate::plate::{custom_plate, plate_country, plate_matches, plate_text, validate_plate};
use crate::sii::{
    Field, Save, Unit, array_base, is_nameless, valid_key, valid_kind, valid_name, valid_value,
};
use crate::state::TruckState;
use crate::value::{quote, unquote};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Version des Paketformats. Steigt, wenn sich der Aufbau ändert.
pub const PACKAGE_VERSION: u32 = 1;

/// Ein Lkw zum Mitnehmen: Fahrzeug-Unit und Zubehör als Bauplan, dazu Modell, Kennzeichen,
/// Zustand und alle `data_path`-Einträge. Statt Unit-Namen stehen Platzhalter `@0`, `@1` und
/// so weiter. `@0` ist immer die `vehicle`-Unit.
///
/// Beim Einsetzen gelten `license_plate` und `state`, nicht die gleichnamigen Felder im
/// Bauplan. So kann der Server Zustand und Kennzeichen pflegen, ohne den Bauplan anzufassen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TruckPackage {
    pub version: u32,
    /// Zum Beispiel `scania.s_2016`
    pub model: String,
    /// Inhalt der Zeichenkette ohne Anführungszeichen, zum Beispiel `SPED 001|_germany`
    pub license_plate: String,
    pub state: TruckState,
    /// Alle `data_path`-Einträge ohne Anführungszeichen, sortiert und ohne Doppelte.
    /// Daraus ergibt sich, welche DLCs und Mods ein Fahrer braucht.
    pub data_paths: Vec<String>,
    pub units: Vec<PackageUnit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageUnit {
    #[serde(rename = "type")]
    pub kind: String,
    /// Platzhalter dieser Unit, `@` plus Position in `units`
    pub id: String,
    /// Name der Unit im Spielstand, aus dem exportiert wurde. Nur zur Nachverfolgung und für
    /// [`InsertOptions::keep_names`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    /// Felder als Paare aus Schlüssel und Wert, in der Reihenfolge der Datei
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct InsertOptions {
    /// Die Unit-Namen aus der Quelle übernehmen statt neue zu vergeben. Für den Vergleich mit
    /// `tools/transplant.py`. Fehler, wenn einer der Namen im Ziel schon vorkommt.
    pub keep_names: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertReport {
    /// Name der neuen `vehicle`-Unit
    pub vehicle: String,
    pub profit_log: String,
    /// War das Ziel in einem Schnellauftrag, der dafür entfernt wurde?
    pub removed_quick_job: bool,
    /// Anzahl der Units, die mit dem Schnellauftrag weggefallen sind
    pub removed_units: usize,
    /// Der bisherige Lkw des Spielers, der in einen freien Stellplatz umgezogen ist
    pub moved_truck: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveReport {
    pub vehicle: String,
    pub removed_units: usize,
    /// Der Lkw, der statt des entfernten jetzt aktiv ist
    pub replacement: Option<String>,
}

/// Kurzinfo zu einem Lkw im Spielstand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruckInfo {
    pub unit: String,
    pub plate: String,
    pub model: Option<String>,
    pub active: bool,
    pub own: bool,
}

// ------------------------------------------------------------------- Exportieren

/// Macht aus einem Lkw ein Paket. Ohne Kennzeichen wird der Lkw genommen, den der Spieler
/// gerade fährt.
pub fn export_truck(save: &Save, plate: Option<&str>) -> Result<TruckPackage> {
    let p = Player::find(save)?;
    let vehicle = p.locate(save, plate)?;
    let names = save.closure(&vehicle, true);
    let ids: HashMap<&str, String> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), placeholder(i)))
        .collect();

    let mut units = Vec::with_capacity(names.len());
    let mut data_paths = BTreeSet::new();
    let mut models = BTreeSet::new();
    for (i, name) in names.iter().enumerate() {
        let u = save.require(name)?;
        if i > 0 && !u.kind.ends_with(v::ACCESSORY_KIND_SUFFIX) {
            return Err(Error::Unsupported(format!(
                "am Lkw hängt die Unit {name} vom Typ {}, erwartet ist nur Zubehör",
                u.kind
            )));
        }
        let mut fields = Vec::with_capacity(u.fields.len());
        for f in &u.fields {
            let value = match ids.get(f.value.as_str()) {
                Some(id) => id.clone(),
                None if parse_placeholder(&f.value).is_some() => {
                    return Err(Error::Unsupported(format!(
                        "der Wert {} in {name} sieht aus wie ein Platzhalter",
                        f.value
                    )));
                }
                None if is_nameless(f.value.trim()) => {
                    return Err(Error::Structure(format!(
                        "{name}, Feld {} verweist auf {}, das es nicht gibt",
                        f.key, f.value
                    )));
                }
                None => f.value.clone(),
            };
            if f.key == v::DATA_PATH {
                let path =
                    unquote(&value).ok_or_else(|| Error::invalid_value(name, &f.key, &value))?;
                if let Some(m) = model_from_path(path) {
                    models.insert(m.to_string());
                }
                data_paths.insert(path.to_string());
            }
            fields.push((f.key.clone(), value));
        }
        units.push(PackageUnit {
            kind: u.kind.clone(),
            id: placeholder(i),
            source_name: Some(name.clone()),
            fields,
        });
    }

    let vu = save.require(&vehicle)?;
    Ok(TruckPackage {
        version: PACKAGE_VERSION,
        model: single_model(models, &vehicle)?,
        license_plate: vehicle_plate(vu)?,
        state: TruckState::read(vu)?,
        data_paths: data_paths.into_iter().collect(),
        units,
    })
}

impl TruckPackage {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| Error::InvalidPackage(format!("JSON konnte nicht erzeugt werden: {e}")))
    }

    pub fn from_json(text: &str) -> Result<TruckPackage> {
        serde_json::from_str(text)
            .map_err(|e| Error::InvalidPackage(format!("kein gültiges JSON: {e}")))
    }

    /// Prüft das Paket auf sich allein, ohne Zielspielstand.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(Error::InvalidPackage(m));
        if self.version != PACKAGE_VERSION {
            return bad(format!(
                "Paketversion {} wird nicht unterstützt, erwartet ist {PACKAGE_VERSION}",
                self.version
            ));
        }
        let Some(first) = self.units.first() else {
            return bad("das Paket enthält keine Units".into());
        };
        if first.kind != v::KIND {
            return bad(format!("die erste Unit muss vom Typ {} sein", v::KIND));
        }
        let mut models = BTreeSet::new();
        for (i, u) in self.units.iter().enumerate() {
            if u.id != placeholder(i) {
                return bad(format!(
                    "Unit {i} hat den Platzhalter {}, erwartet ist {}",
                    u.id,
                    placeholder(i)
                ));
            }
            if !valid_kind(&u.kind) || (i > 0 && !u.kind.ends_with(v::ACCESSORY_KIND_SUFFIX)) {
                return bad(format!("Unit {} hat den unerlaubten Typ {}", u.id, u.kind));
            }
            if let Some(n) = &u.source_name
                && !(valid_name(n) && is_nameless(n))
            {
                return bad(format!("Unit {} hat den ungültigen Quellnamen {n}", u.id));
            }
            for (key, value) in &u.fields {
                if !valid_key(key) || !valid_value(value) {
                    return bad(format!("Unit {} hat ein ungültiges Feld {key:?}", u.id));
                }
                match parse_placeholder(value) {
                    Some(j) if j >= self.units.len() => {
                        return bad(format!(
                            "{}, Feld {key} verweist auf {value}, das es nicht gibt",
                            u.id
                        ));
                    }
                    Some(_) => {}
                    None if is_nameless(value.trim()) => {
                        return bad(format!(
                            "{}, Feld {key} enthält den Unit-Namen {value} statt eines Platzhalters",
                            u.id
                        ));
                    }
                    None => {}
                }
                if key == v::DATA_PATH
                    && let Some(m) = unquote(value).and_then(model_from_path)
                {
                    models.insert(m.to_string());
                }
            }
        }
        let model = single_model(models, "@0").map_err(|e| Error::InvalidPackage(e.to_string()))?;
        if model != self.model {
            return bad(format!(
                "das Modell {} passt nicht zum Bauplan ({model})",
                self.model
            ));
        }
        validate_plate(&self.license_plate)?;
        self.state.validate()?;
        Ok(())
    }
}

// ------------------------------------------------------------------- Einsetzen

/// Setzt einen Lkw aus einem Paket in den Spielstand und macht ihn zum aktiven Lkw.
///
/// Unterstützt sind zwei Ausgangslagen:
/// - Der Spieler ist in einem Schnellauftrag. Auftrag, Firmen-Lkw und Anhänger kommen weg.
///   Hat er schon eigene Lkw, zieht der bisherige in einen freien Stellplatz der Heimatgarage.
/// - Der Spieler fährt einen eigenen Lkw ohne Auftrag. Der bisherige Lkw zieht in einen
///   freien Stellplatz der Heimatgarage. Gibt es keinen, ist das ein Fehler.
pub fn insert_truck(
    save: &mut Save,
    package: &TruckPackage,
    options: &InsertOptions,
) -> Result<InsertReport> {
    package.validate()?;
    let mut work = save.clone();
    let report = insert_into(&mut work, package, options)?;
    ensure_intact(&work)?;
    *save = work;
    Ok(report)
}

fn insert_into(
    work: &mut Save,
    package: &TruckPackage,
    options: &InsertOptions,
) -> Result<InsertReport> {
    let p = Player::find(work)?;

    // 1. Ausgangslage klären, einen Schnellauftrag räumen
    let job = work.get(&p.unit, pl::CURRENT_JOB)?.to_string();
    let removed_quick_job = job != NULL;
    let removed_units = if removed_quick_job {
        remove_quick_job(work, &p, &job)?
    } else {
        let active = work.get(&p.assigned, pv::VEHICLE)?;
        let trucks = work.array(&p.unit, pl::TRUCKS)?;
        if active == NULL || !trucks.iter().any(|t| t == active) {
            return Err(Error::Unsupported(
                "der Spieler fährt weder einen eigenen Lkw noch einen Schnellauftrag".into(),
            ));
        }
        0
    };

    // 2. Kennzeichen noch frei?
    for name in work.names_of_kind(v::KIND) {
        if let Some(plate) = work.unit(&name).and_then(|u| vehicle_plate(u).ok())
            && plate_matches(&plate, &package.license_plate)
        {
            return Err(Error::PlateExists(plate_text(&package.license_plate)));
        }
    }

    // 3. Stellplatz: der neue Lkw kommt in den Slot des Spieler-Fahrers. Steht dort schon
    //    ein Lkw, zieht er in einen freien Slot (ohne Lkw und ohne Fahrer).
    let mut slots = work.array(&p.garage, g::VEHICLES)?;
    let drivers = work.array(&p.garage, g::DRIVERS)?;
    if slots.len() != drivers.len() {
        return Err(Error::Structure(format!(
            "in {} sind die Listen {} und {} verschieden lang",
            p.garage,
            g::VEHICLES,
            g::DRIVERS
        )));
    }
    let slot = drivers.iter().position(|d| *d == p.driver).ok_or_else(|| {
        Error::Structure(format!(
            "der Spieler-Fahrer {} hat keinen Platz in der Heimatgarage {}",
            p.driver, p.hq
        ))
    })?;
    let previous = slots.get(slot).cloned().unwrap_or_else(|| NULL.into());
    let moved_truck = if previous != NULL {
        let free = slots
            .iter()
            .zip(&drivers)
            .position(|(v, d)| v == NULL && d == NULL)
            .ok_or_else(|| Error::NoFreeSlot(p.hq.clone()))?;
        set_slot(&mut slots, free, &previous)?;
        Some(previous)
    } else {
        None
    };

    // 4. Namen vergeben
    let mut names_gen = NameGenerator::new(work);
    let names: Vec<String> = if options.keep_names {
        let mut names = Vec::with_capacity(package.units.len());
        for u in &package.units {
            let n = u.source_name.clone().ok_or_else(|| {
                Error::InvalidPackage(format!("Unit {} hat keinen Quellnamen", u.id))
            })?;
            names_gen.reserve(&n)?;
            names.push(n);
        }
        names
    } else {
        package
            .units
            .iter()
            .map(|_| names_gen.next())
            .collect::<Result<_>>()?
    };
    let profit_log_name = names_gen.next()?;
    let vehicle = names
        .first()
        .cloned()
        .ok_or_else(|| Error::InvalidPackage("das Paket enthält keine Units".into()))?;

    // 5. Units bauen, Zustand und Kennzeichen aus dem Paket übernehmen. Neue Felder bekommen
    //    die übliche Einrückung des Ziels.
    let indent = work.indent().to_string();
    let mut units = build_units(package, &names, &indent)?;
    let vu = units
        .first_mut()
        .ok_or_else(|| Error::InvalidPackage("das Paket enthält keine Units".into()))?;
    package.state.apply(vu)?;
    vu.set(v::LICENSE_PLATE, &quote(&package.license_plate))?;
    units.push(Unit {
        kind: profit_log::KIND.into(),
        name: profit_log_name.clone(),
        fields: profit_log::EMPTY
            .iter()
            .map(|(k, val)| Field::with_indent(*k, *val, indent.as_str()))
            .collect(),
    });
    let pos = work
        .position(&p.unit)
        .ok_or_else(|| Error::MissingUnit(p.unit.clone()))?;
    work.insert_units(pos + 1, units)?;

    // 6. Verweise eintragen
    let (mut trucks, mut logs) = truck_lists(work, &p)?;
    trucks.push(vehicle.clone());
    logs.push(profit_log_name.clone());
    work.set_array(&p.unit, pl::TRUCKS, &trucks)?;
    work.set_array(&p.unit, pl::TRUCK_PROFIT_LOGS, &logs)?;
    work.set(&p.unit, pl::MY_VEHICLES_MODE, pl::MY_VEHICLES_MODE_TRUCK)?;

    let placement = work.get(&p.assigned, pv::PLACEMENT)?.to_string();
    work.set(&p.assigned, pv::VEHICLE, &vehicle)?;
    work.set(&p.mine, pv::VEHICLE, &vehicle)?;
    work.set(&p.mine, pv::PLACEMENT, &placement)?;
    work.set(&p.mine, pv::CITY, &format!("{CITY_PREFIX}{}", p.hq))?;

    set_slot(&mut slots, slot, &vehicle)?;
    work.set_array(&p.garage, g::VEHICLES, &slots)?;

    let progress = work.single(gp::KIND)?;
    let mut owned = work.array(&progress, gp::OWNED_TRUCKS)?;
    let entry = gp::owned_truck_entry(&package.model);
    if !owned.contains(&entry) {
        owned.push(entry);
        work.set_array(&progress, gp::OWNED_TRUCKS, &owned)?;
    }

    Ok(InsertReport {
        vehicle,
        profit_log: profit_log_name,
        removed_quick_job,
        removed_units,
        moved_truck,
    })
}

/// Entfernt Schnellauftrag, Firmen-Lkw und Anhänger mit allem Zubehör.
fn remove_quick_job(work: &mut Save, p: &Player, job: &str) -> Result<usize> {
    let ju = work.require(job)?;
    if ju.kind != pj::KIND {
        return Err(Error::Structure(format!(
            "{} verweist auf {job}, das ist kein {}",
            pl::CURRENT_JOB,
            pj::KIND
        )));
    }
    let company_truck = ju.require(pj::COMPANY_TRUCK)?.to_string();
    if company_truck == NULL {
        return Err(Error::Unsupported(
            "der Spieler fährt gerade einen Auftrag mit eigenem Lkw. Erst abliefern oder abbrechen"
                .into(),
        ));
    }
    let assigned_vehicle = work.get(&p.assigned, pv::VEHICLE)?.to_string();
    if assigned_vehicle != company_truck {
        return Err(Error::Structure(format!(
            "der Firmen-Lkw {company_truck} des Auftrags ist nicht der Lkw, den der Spieler fährt"
        )));
    }
    let trailer = work.get(&p.assigned, pv::TRAILER)?.to_string();

    let mut delete = Vec::new();
    for root in [assigned_vehicle.as_str(), trailer.as_str(), job] {
        if root != NULL {
            delete.extend(work.closure(root, true));
        }
    }
    let delete: HashSet<String> = delete.into_iter().collect();
    let allowed = [p.unit.as_str(), p.assigned.as_str()];
    for (target, unit, key) in work.references_into(&delete) {
        if !delete.contains(&unit) && !allowed.contains(&unit.as_str()) {
            return Err(Error::Unsupported(format!(
                "{target} gehört zum Schnellauftrag, wird aber noch von {unit}, Feld {key} verwendet"
            )));
        }
    }
    work.remove_units(&delete);
    work.set(&p.unit, pl::CURRENT_JOB, NULL)?;
    work.set(&p.assigned, pv::TRAILER, NULL)?;
    work.set_array(&p.assigned, pv::TRAILER_PLACEMENTS, &[])?;
    work.set(&p.assigned, pv::TRAILER_ATTACHED, "false")?;
    Ok(delete.len())
}

fn build_units(package: &TruckPackage, names: &[String], indent: &str) -> Result<Vec<Unit>> {
    package
        .units
        .iter()
        .zip(names)
        .map(|(u, name)| {
            let fields = u
                .fields
                .iter()
                .map(|(key, value)| {
                    let value = match parse_placeholder(value) {
                        Some(j) => names.get(j).cloned().ok_or_else(|| {
                            Error::InvalidPackage(format!("Platzhalter {value} gibt es nicht"))
                        })?,
                        None => value.clone(),
                    };
                    Ok(Field::with_indent(key.clone(), value, indent))
                })
                .collect::<Result<_>>()?;
            Ok(Unit {
                kind: u.kind.clone(),
                name: name.clone(),
                fields,
            })
        })
        .collect()
}

// ------------------------------------------------------------------- Entfernen

/// Entfernt einen eigenen Lkw mit Zubehör und Statistik. Ist es der aktive Lkw, muss ein
/// anderer eigener Lkw als Ersatz angegeben sein. Der Ersatz übernimmt den Stellplatz des
/// entfernten Lkw, sein alter Stellplatz wird frei.
pub fn remove_truck(
    save: &mut Save,
    plate: &str,
    replacement: Option<&str>,
) -> Result<RemoveReport> {
    let mut work = save.clone();
    let report = remove_from(&mut work, plate, replacement)?;
    ensure_intact(&work)?;
    *save = work;
    Ok(report)
}

fn remove_from(work: &mut Save, plate: &str, replacement: Option<&str>) -> Result<RemoveReport> {
    let p = Player::find(work)?;
    let (mut trucks, mut logs) = truck_lists(work, &p)?;
    let vehicle = find_by_plate(work, &trucks, plate)?;
    let index = trucks
        .iter()
        .position(|t| *t == vehicle)
        .ok_or_else(|| Error::TruckNotFound(plate.into()))?;
    let log = logs.get(index).cloned().unwrap_or_else(|| NULL.into());

    let assigned_vehicle = work.get(&p.assigned, pv::VEHICLE)?.to_string();
    let my_vehicle = work.get(&p.mine, pv::VEHICLE)?.to_string();
    let active = assigned_vehicle == vehicle || my_vehicle == vehicle;
    let replacement = if active {
        if work.get(&p.unit, pl::CURRENT_JOB)? != NULL {
            return Err(Error::Unsupported(
                "der Lkw steckt in einem laufenden Auftrag".into(),
            ));
        }
        let shown = work
            .unit(&vehicle)
            .and_then(|u| vehicle_plate(u).ok())
            .map_or_else(|| plate.to_string(), |pl| plate_text(&pl));
        let query = replacement.ok_or(Error::ReplacementRequired(shown))?;
        let r = find_by_plate(work, &trucks, query)?;
        if r == vehicle {
            return Err(Error::InvalidInput(
                "Der Ersatz ist derselbe Lkw, der entfernt werden soll".into(),
            ));
        }
        Some(r)
    } else {
        None
    };

    // Was nur an diesem Lkw hängt: Zubehör, Statistik mit Einträgen
    let mut delete: HashSet<String> = work.closure(&vehicle, true).into_iter().collect();
    if log != NULL {
        delete.extend(work.closure(&log, true));
    }
    for (target, unit, key) in work.references_into(&delete) {
        if delete.contains(&unit) {
            continue;
        }
        let kind = work
            .unit(&unit)
            .map(|u| u.kind.as_str())
            .unwrap_or_default();
        let base = array_base(&key).map_or(key.as_str(), |(b, _)| b);
        let known = (unit == p.unit && (base == pl::TRUCKS || base == pl::TRUCK_PROFIT_LOGS))
            || ((unit == p.assigned || unit == p.mine) && key == pv::VEHICLE)
            || (kind == g::KIND && base == g::VEHICLES)
            || (kind == driver_ai::KIND && driver_ai::TRUCK_FIELDS.contains(&key.as_str()));
        if !known {
            return Err(Error::Unsupported(format!(
                "{target} gehört zum Lkw, wird aber noch von {unit}, Feld {key} verwendet"
            )));
        }
    }

    trucks.remove(index);
    if index < logs.len() {
        logs.remove(index);
    }
    work.set_array(&p.unit, pl::TRUCKS, &trucks)?;
    work.set_array(&p.unit, pl::TRUCK_PROFIT_LOGS, &logs)?;
    if trucks.is_empty() {
        work.set(&p.unit, pl::MY_VEHICLES_MODE, pl::MY_VEHICLES_MODE_NONE)?;
    }

    if let Some(r) = &replacement {
        if assigned_vehicle == vehicle {
            work.set(&p.assigned, pv::VEHICLE, r)?;
        }
        if my_vehicle == vehicle {
            work.set(&p.mine, pv::VEHICLE, r)?;
        }
    }

    // Garagen: der entfernte Lkw geht raus, der Ersatz zieht auf seinen Platz
    let mut vehicle_had_slot = false;
    for garage in work.names_of_kind(g::KIND) {
        let old = work.array(&garage, g::VEHICLES)?;
        let new: Vec<String> = old
            .iter()
            .map(|slot| {
                if *slot == vehicle {
                    vehicle_had_slot = true;
                    replacement.clone().unwrap_or_else(|| NULL.into())
                } else if replacement.as_ref() == Some(slot) {
                    NULL.into()
                } else {
                    slot.clone()
                }
            })
            .collect();
        if new != old {
            work.set_array(&garage, g::VEHICLES, &new)?;
        }
    }
    if replacement.is_some() && !vehicle_had_slot {
        return Err(Error::Structure(
            "der aktive Lkw steht in keiner Garage, der Ersatz hätte keinen Platz".into(),
        ));
    }

    // Angestellte Fahrer, die auf einen der beiden Lkw zeigen, verlieren ihn
    for driver in work.names_of_kind(driver_ai::KIND) {
        for field in driver_ai::TRUCK_FIELDS {
            let current = work.unit(&driver).and_then(|u| u.get(field));
            if current.is_some_and(|c| c == vehicle || replacement.as_deref() == Some(c)) {
                work.set(&driver, field, NULL)?;
            }
        }
    }

    work.remove_units(&delete);
    Ok(RemoveReport {
        vehicle,
        removed_units: delete.len(),
        replacement,
    })
}

// ------------------------------------------------------------------- Zustand, Kennzeichen

/// Zustand eines Lkw. Ohne Kennzeichen der Lkw, den der Spieler gerade fährt.
pub fn truck_state(save: &Save, plate: Option<&str>) -> Result<TruckState> {
    let p = Player::find(save)?;
    let vehicle = p.locate(save, plate)?;
    TruckState::read(save.require(&vehicle)?)
}

pub fn set_truck_state(save: &mut Save, plate: Option<&str>, state: &TruckState) -> Result<()> {
    let p = Player::find(save)?;
    let vehicle = p.locate(save, plate)?;
    state.apply(save.require_mut(&vehicle)?)
}

/// Kennzeichen eines Lkw als Inhalt der Zeichenkette, zum Beispiel `SPED 001|_germany`.
pub fn license_plate(save: &Save, plate: Option<&str>) -> Result<String> {
    let p = Player::find(save)?;
    let vehicle = p.locate(save, plate)?;
    vehicle_plate(save.require(&vehicle)?)
}

/// Gibt einem Lkw ein selbst vergebenes Kennzeichen. Ohne Land bleibt das bisherige.
/// Gibt das neue Kennzeichen zurück.
pub fn set_license_plate(
    save: &mut Save,
    plate: Option<&str>,
    text: &str,
    country: Option<&str>,
) -> Result<String> {
    let p = Player::find(save)?;
    let vehicle = p.locate(save, plate)?;
    let old = vehicle_plate(save.require(&vehicle)?)?;
    let new = custom_plate(text, country.unwrap_or_else(|| plate_country(&old)))?;
    for name in save.names_of_kind(v::KIND) {
        if name != vehicle
            && let Some(other) = save.unit(&name).and_then(|u| vehicle_plate(u).ok())
            && plate_matches(&other, &new)
        {
            return Err(Error::PlateExists(plate_text(&new)));
        }
    }
    save.set(&vehicle, v::LICENSE_PLATE, &quote(&new))?;
    Ok(new)
}

/// Alle Lkw, die dem Spieler gehören oder die er gerade fährt.
pub fn list_trucks(save: &Save) -> Result<Vec<TruckInfo>> {
    let p = Player::find(save)?;
    let own = save.array(&p.unit, pl::TRUCKS)?;
    let active = save.get(&p.assigned, pv::VEHICLE)?.to_string();
    let mut names = own.clone();
    if active != NULL && !names.contains(&active) {
        names.push(active.clone());
    }
    names
        .into_iter()
        .map(|n| {
            let u = save.require(&n)?;
            let model = save
                .closure(&n, true)
                .iter()
                .filter_map(|a| save.unit(a)?.get(v::DATA_PATH))
                .filter_map(|dp| unquote(dp).and_then(model_from_path))
                .map(str::to_string)
                .next();
            Ok(TruckInfo {
                plate: vehicle_plate(u)?,
                model,
                active: n == active,
                own: own.contains(&n),
                unit: n,
            })
        })
        .collect()
}

// ------------------------------------------------------------------- Hilfen

/// Die Units rund um den Spieler, die fast jede Operation braucht.
struct Player {
    unit: String,
    assigned: String,
    mine: String,
    hq: String,
    garage: String,
    driver: String,
}

impl Player {
    fn find(save: &Save) -> Result<Player> {
        let unit = save.single(pl::KIND)?;
        let reference = |key: &str| -> Result<String> {
            let value = save.get(&unit, key)?;
            save.resolves(value)
                .map(str::to_string)
                .ok_or_else(|| Error::invalid_value(&unit, key, value))
        };
        let assigned = reference(pl::ASSIGNED_VEHICLES)?;
        let mine = reference(&format!("{}[{}]", pl::MY_VEHICLES, pl::MY_TRUCK_INDEX))?;
        let hq = save.get(&unit, pl::HQ_CITY)?.to_string();
        let garage = format!("{}{hq}", g::NAME_PREFIX);
        save.require(&garage)?;
        let driver = save
            .array(&unit, pl::DRIVERS)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Structure(format!("{} ist leer", pl::DRIVERS)))?;
        Ok(Player {
            unit,
            assigned,
            mine,
            hq,
            garage,
            driver,
        })
    }

    /// Lkw per Kennzeichen unter den eigenen Lkw und dem gefahrenen. Ohne Kennzeichen der
    /// gefahrene.
    fn locate(&self, save: &Save, plate: Option<&str>) -> Result<String> {
        let active = save.get(&self.assigned, pv::VEHICLE)?.to_string();
        let name = match plate {
            Some(q) => {
                let mut candidates = save.array(&self.unit, pl::TRUCKS)?;
                if active != NULL && !candidates.contains(&active) {
                    candidates.push(active);
                }
                find_by_plate(save, &candidates, q)?
            }
            None if active == NULL => {
                return Err(Error::InvalidInput(
                    "Der Spieler fährt gerade keinen Lkw".into(),
                ));
            }
            None => active,
        };
        let kind = &save.require(&name)?.kind;
        if kind != v::KIND {
            return Err(Error::Structure(format!(
                "{name} ist kein Lkw, sondern {kind}"
            )));
        }
        Ok(name)
    }
}

fn find_by_plate(save: &Save, candidates: &[String], query: &str) -> Result<String> {
    let mut hits = candidates.iter().filter(|n| {
        save.unit(n)
            .filter(|u| u.kind == v::KIND)
            .and_then(|u| vehicle_plate(u).ok())
            .is_some_and(|p| plate_matches(&p, query))
    });
    match (hits.next(), hits.next()) {
        (Some(n), None) => Ok(n.clone()),
        (None, _) => Err(Error::TruckNotFound(query.into())),
        (Some(_), Some(_)) => Err(Error::AmbiguousPlate(query.into())),
    }
}

fn truck_lists(save: &Save, p: &Player) -> Result<(Vec<String>, Vec<String>)> {
    let trucks = save.array(&p.unit, pl::TRUCKS)?;
    let logs = save.array(&p.unit, pl::TRUCK_PROFIT_LOGS)?;
    if trucks.len() != logs.len() {
        return Err(Error::Structure(format!(
            "{} und {} sind verschieden lang",
            pl::TRUCKS,
            pl::TRUCK_PROFIT_LOGS
        )));
    }
    Ok((trucks, logs))
}

fn vehicle_plate(unit: &Unit) -> Result<String> {
    let raw = unit.require(v::LICENSE_PLATE)?;
    unquote(raw)
        .map(str::to_string)
        .ok_or_else(|| Error::invalid_value(&unit.name, v::LICENSE_PLATE, raw))
}

fn set_slot(slots: &mut [String], index: usize, value: &str) -> Result<()> {
    let slot = slots
        .get_mut(index)
        .ok_or_else(|| Error::Structure("Stellplatz außerhalb der Liste".into()))?;
    *slot = value.into();
    Ok(())
}

fn ensure_intact(save: &Save) -> Result<()> {
    let report = check(save);
    if report.is_ok() {
        Ok(())
    } else {
        Err(Error::Integrity(report.problems))
    }
}

fn model_from_path(path: &str) -> Option<&str> {
    let m = path
        .strip_prefix(v::MODEL_PATH_PREFIX)?
        .strip_suffix(v::MODEL_PATH_SUFFIX)?;
    (!m.is_empty() && !m.contains('/')).then_some(m)
}

fn single_model(models: BTreeSet<String>, unit: &str) -> Result<String> {
    let mut it = models.into_iter();
    match (it.next(), it.next()) {
        (Some(m), None) => Ok(m),
        (None, _) => Err(Error::Structure(format!(
            "am Lkw {unit} ist kein Modell zu erkennen (data_path {}<modell>{} fehlt)",
            v::MODEL_PATH_PREFIX,
            v::MODEL_PATH_SUFFIX
        ))),
        (Some(a), Some(b)) => Err(Error::Structure(format!(
            "am Lkw {unit} hängen mehrere Modelle ({a}, {b})"
        ))),
    }
}

fn placeholder(i: usize) -> String {
    format!("@{i}")
}

fn parse_placeholder(value: &str) -> Option<usize> {
    let digits = value.strip_prefix('@')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Vergibt neue, eindeutige Unit-Namen nach [`layout::NAME_PREFIX`].
struct NameGenerator<'a> {
    save: &'a Save,
    taken: HashSet<String>,
    next: u32,
}

impl<'a> NameGenerator<'a> {
    fn new(save: &'a Save) -> Self {
        NameGenerator {
            save,
            taken: HashSet::new(),
            next: 0,
        }
    }

    /// Einen vorgegebenen Namen belegen, für `keep_names`.
    fn reserve(&mut self, name: &str) -> Result<()> {
        if !valid_name(name) || !is_nameless(name) {
            return Err(Error::InvalidPackage(format!(
                "ungültiger Unit-Name {name}"
            )));
        }
        if self.save.contains(name) || !self.taken.insert(name.into()) {
            return Err(Error::NameCollision(name.into()));
        }
        Ok(())
    }

    fn next(&mut self) -> Result<String> {
        loop {
            let n = self.next;
            self.next = n
                .checked_add(1)
                .ok_or_else(|| Error::Structure("keine freien Unit-Namen mehr".into()))?;
            let per_middle = (0x1_0000 - layout::NAME_LOW_START).div_ceil(layout::NAME_STEP);
            let middle = layout::NAME_MIDDLE_START + n / per_middle;
            let low = layout::NAME_LOW_START + (n % per_middle) * layout::NAME_STEP;
            if middle > 0xffff {
                return Err(Error::Structure("keine freien Unit-Namen mehr".into()));
            }
            let name = format!("{}{middle:04x}.{low:04x}", layout::NAME_PREFIX);
            if !self.save.contains(&name) && self.taken.insert(name.clone()) {
                return Ok(name);
            }
        }
    }
}
