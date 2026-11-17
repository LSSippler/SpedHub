//! Zustand eines Lkw: Kilometer, Wartungs-Kilometer, Verschleiß, Tank.

use crate::error::{Error, Result};
use crate::layout::vehicle as v;
use crate::sii::Unit;
use crate::value::{format_float, parse_float, parse_int};
use serde::{Deserialize, Serialize};

/// Alle Werte, die sich beim Fahren ändern. Verschleiß und Tank liegen zwischen 0 und 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TruckState {
    /// Kilometerstand, ganze Kilometer
    pub odometer: u32,
    /// Nachkommaanteil des Kilometerstands, 0 bis unter 1
    pub odometer_float_part: f32,
    /// Kilometer für die Wartung
    pub integrity_odometer: u32,
    pub integrity_odometer_float_part: f32,
    pub fuel_relative: f32,
    pub engine_wear: f32,
    pub transmission_wear: f32,
    pub cabin_wear: f32,
    pub chassis_wear: f32,
    pub engine_wear_unfixable: f32,
    pub transmission_wear_unfixable: f32,
    pub cabin_wear_unfixable: f32,
    pub chassis_wear_unfixable: f32,
    /// Je Achse ein Wert, die Anzahl hängt vom Lkw ab
    pub wheels_wear: Vec<f32>,
    pub wheels_wear_unfixable: Vec<f32>,
}

/// Einzelne Kommazahlen mit ihrem Feldnamen, damit Lesen, Schreiben und Prüfen dieselbe
/// Liste benutzen.
fn floats(s: &TruckState) -> [(&'static str, f32, Range); 11] {
    [
        (
            v::ODOMETER_FLOAT_PART,
            s.odometer_float_part,
            Range::Fraction,
        ),
        (
            v::INTEGRITY_ODOMETER_FLOAT_PART,
            s.integrity_odometer_float_part,
            Range::Fraction,
        ),
        (v::FUEL_RELATIVE, s.fuel_relative, Range::Unit),
        (v::ENGINE_WEAR, s.engine_wear, Range::Unit),
        (v::TRANSMISSION_WEAR, s.transmission_wear, Range::Unit),
        (v::CABIN_WEAR, s.cabin_wear, Range::Unit),
        (v::CHASSIS_WEAR, s.chassis_wear, Range::Unit),
        (
            v::ENGINE_WEAR_UNFIXABLE,
            s.engine_wear_unfixable,
            Range::Unit,
        ),
        (
            v::TRANSMISSION_WEAR_UNFIXABLE,
            s.transmission_wear_unfixable,
            Range::Unit,
        ),
        (v::CABIN_WEAR_UNFIXABLE, s.cabin_wear_unfixable, Range::Unit),
        (
            v::CHASSIS_WEAR_UNFIXABLE,
            s.chassis_wear_unfixable,
            Range::Unit,
        ),
    ]
}

#[derive(Clone, Copy)]
enum Range {
    /// 0 bis 1
    Unit,
    /// 0 bis unter 1
    Fraction,
}

impl Range {
    fn contains(self, f: f32) -> bool {
        match self {
            Range::Unit => (0.0..=1.0).contains(&f),
            Range::Fraction => (0.0..1.0).contains(&f),
        }
    }
}

impl TruckState {
    /// Liest den Zustand aus einer `vehicle`-Unit.
    pub fn read(unit: &Unit) -> Result<TruckState> {
        let float = |key: &str| -> Result<f32> {
            let raw = unit.require(key)?;
            parse_float(raw).ok_or_else(|| Error::invalid_value(&unit.name, key, raw))
        };
        let int = |key: &str| -> Result<u32> {
            let raw = unit.require(key)?;
            parse_int(raw)
                .and_then(|i| u32::try_from(i).ok())
                .ok_or_else(|| Error::invalid_value(&unit.name, key, raw))
        };
        let list = |key: &str| -> Result<Vec<f32>> {
            unit.array(key)?
                .iter()
                .map(|raw| {
                    parse_float(raw).ok_or_else(|| Error::invalid_value(&unit.name, key, raw))
                })
                .collect()
        };
        Ok(TruckState {
            odometer: int(v::ODOMETER)?,
            odometer_float_part: float(v::ODOMETER_FLOAT_PART)?,
            integrity_odometer: int(v::INTEGRITY_ODOMETER)?,
            integrity_odometer_float_part: float(v::INTEGRITY_ODOMETER_FLOAT_PART)?,
            fuel_relative: float(v::FUEL_RELATIVE)?,
            engine_wear: float(v::ENGINE_WEAR)?,
            transmission_wear: float(v::TRANSMISSION_WEAR)?,
            cabin_wear: float(v::CABIN_WEAR)?,
            chassis_wear: float(v::CHASSIS_WEAR)?,
            engine_wear_unfixable: float(v::ENGINE_WEAR_UNFIXABLE)?,
            transmission_wear_unfixable: float(v::TRANSMISSION_WEAR_UNFIXABLE)?,
            cabin_wear_unfixable: float(v::CABIN_WEAR_UNFIXABLE)?,
            chassis_wear_unfixable: float(v::CHASSIS_WEAR_UNFIXABLE)?,
            wheels_wear: list(v::WHEELS_WEAR)?,
            wheels_wear_unfixable: list(v::WHEELS_WEAR_UNFIXABLE)?,
        })
    }

    /// Prüft die Wertebereiche, ohne etwas zu ändern.
    pub fn validate(&self) -> Result<()> {
        for (key, f, range) in floats(self) {
            if !range.contains(f) {
                return Err(out_of_range(key, f, range));
            }
        }
        for (key, list) in [
            (v::WHEELS_WEAR, &self.wheels_wear),
            (v::WHEELS_WEAR_UNFIXABLE, &self.wheels_wear_unfixable),
        ] {
            if let Some(&f) = list.iter().find(|&&f| !Range::Unit.contains(f)) {
                return Err(out_of_range(key, f, Range::Unit));
            }
        }
        Ok(())
    }

    /// Schreibt den Zustand in eine `vehicle`-Unit. Werte, die sich nicht geändert haben,
    /// bleiben byte-genau so stehen, wie sie waren. Bei einem Fehler bleibt die Unit unverändert.
    pub fn apply(&self, unit: &mut Unit) -> Result<()> {
        self.validate()?;
        let mut work = unit.clone();
        for (key, n) in [
            (v::ODOMETER, self.odometer),
            (v::INTEGRITY_ODOMETER, self.integrity_odometer),
        ] {
            if parse_int(work.require(key)?) != Some(i64::from(n)) {
                work.set(key, &n.to_string())?;
            }
        }
        for (key, f, _) in floats(self) {
            set_float(&mut work, key, f)?;
        }
        for (key, list) in [
            (v::WHEELS_WEAR, &self.wheels_wear),
            (v::WHEELS_WEAR_UNFIXABLE, &self.wheels_wear_unfixable),
        ] {
            let old = work.array(key)?;
            if old.len() != list.len() {
                return Err(Error::InvalidInput(format!(
                    "{key}: der Lkw hat {} Werte, im Zustand stehen {}",
                    old.len(),
                    list.len()
                )));
            }
            let new: Vec<String> = old
                .iter()
                .zip(list)
                .map(|(raw, &f)| keep_or_format(raw, f))
                .collect();
            if new != old {
                work.set_array(key, &new)?;
            }
        }
        *unit = work;
        Ok(())
    }
}

fn out_of_range(key: &str, f: f32, range: Range) -> Error {
    let bound = match range {
        Range::Unit => "0 bis 1",
        Range::Fraction => "0 bis unter 1",
    };
    Error::InvalidInput(format!("{key} muss zwischen {bound} liegen, ist aber {f}"))
}

fn keep_or_format(raw: &str, f: f32) -> String {
    match parse_float(raw) {
        Some(old) if old.to_bits() == f.to_bits() => raw.to_string(),
        _ => format_float(f),
    }
}

fn set_float(unit: &mut Unit, key: &str, f: f32) -> Result<()> {
    let raw = unit.require(key)?.to_string();
    let new = keep_or_format(&raw, f);
    if new != raw {
        unit.set(key, &new)?;
    }
    Ok(())
}
