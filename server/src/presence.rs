//! Herzschlag aus dem Spiel: wer gerade fährt, mit welchem Lkw, wo und mit welchem Auftrag.
//!
//! Das Plugin meldet alle 10 Sekunden, solange das Spiel läuft und nicht pausiert ist. Online
//! ist, wer sich in den letzten [`ONLINE_SECS`] Sekunden gemeldet hat. Der letzte Herzschlag
//! je Fahrer und Firma liegt in `presence`, jede Meldung überschreibt die vorige.
//!
//! Passt das Kennzeichen zu einem Firmen-Lkw, den der Fahrer gerade geliehen hat, wird der
//! gemeldete Zustand als letzter bekannter Zustand des Lkw gespeichert (`trucks.last_*`).
//! Nimmt der Chef den Lkw ohne Kilometerstand zurück, etwa nach einem Absturz, gilt dieser.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// So lange nach dem letzten Herzschlag gilt ein Fahrer als online.
pub const ONLINE_SECS: i64 = 30;

/// Längste Texte (Stadt, Fracht) in Zeichen.
pub const TEXT_MAX: usize = 64;

/// Längstes Kennzeichen in Zeichen, mit Markup.
pub const PLATE_MAX: usize = 64;

/// Kennzeichen zum Vergleichen: ohne Markup, ohne Land, Leerraum zusammengefasst, groß
/// geschrieben. `DD<img src=...>RI 772|germany` und `dd ri 772` ergeben dasselbe.
pub fn plate_key(plate: &str) -> String {
    spedhub_save::plate_text(plate).to_uppercase()
}

/// Text vom Client: ohne Steuerzeichen, Leerraum zusammengefasst, höchstens `max` Zeichen.
/// Leer wird zu `None`.
pub fn clean_text(s: Option<&str>, max: usize) -> Option<String> {
    let s = s?;
    let words: Vec<&str> = s
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect();
    let joined: String = words.join(" ").chars().take(max).collect();
    let trimmed = joined.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[derive(Debug, Default, Deserialize)]
pub struct WearReq {
    pub engine: Option<f64>,
    pub transmission: Option<f64>,
    pub cabin: Option<f64>,
    pub chassis: Option<f64>,
    pub wheels: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct JobReq {
    pub cargo: Option<String>,
    pub source_city: Option<String>,
    pub target_city: Option<String>,
}

/// Ein Herzschlag. Nur das Kennzeichen ist Pflicht. Unbekannte Felder werden übergangen.
#[derive(Debug, Default, Deserialize)]
pub struct HeartbeatReq {
    #[serde(default)]
    pub plate: String,
    pub odometer_km: Option<f64>,
    pub wear: Option<WearReq>,
    pub fuel_relative: Option<f64>,
    pub fuel_liters: Option<f64>,
    pub near_city: Option<String>,
    pub speed_kmh: Option<f64>,
    /// Laufender Auftrag, `null` oder fehlend heißt: keiner
    pub job: Option<JobReq>,
}

/// Ein geprüfter Herzschlag.
#[derive(Debug, Clone, PartialEq)]
pub struct Heartbeat {
    pub plate: String,
    pub odometer_km: Option<f64>,
    pub engine: Option<f64>,
    pub transmission: Option<f64>,
    pub cabin: Option<f64>,
    pub chassis: Option<f64>,
    pub wheels: Option<f64>,
    pub fuel_relative: Option<f64>,
    pub fuel_liters: Option<f64>,
    pub near_city: Option<String>,
    pub speed_kmh: Option<f64>,
    pub job: Option<Job>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub cargo: Option<String>,
    pub source_city: Option<String>,
    pub target_city: Option<String>,
}

fn number(v: Option<f64>, max: f64, what: &str) -> Result<Option<f64>, String> {
    match v {
        None => Ok(None),
        Some(f) if f.is_finite() && (0.0..=max).contains(&f) => Ok(Some(f)),
        Some(_) => Err(format!("{what} muss zwischen 0 und {max} liegen")),
    }
}

impl HeartbeatReq {
    /// Prüft alle Werte. Unplausible Zahlen (negativ, Verschleiß über 1) geben eine Meldung
    /// für die Antwort 400.
    pub fn validate(self) -> Result<Heartbeat, String> {
        let plate = self.plate.trim();
        if plate.is_empty() || plate_key(plate).is_empty() {
            return Err("Das Kennzeichen fehlt".into());
        }
        if plate.chars().count() > PLATE_MAX || plate.chars().any(char::is_control) {
            return Err(format!(
                "Das Kennzeichen darf höchstens {PLATE_MAX} Zeichen ohne Steuerzeichen haben"
            ));
        }
        let wear = self.wear.unwrap_or_default();
        let job = self.job.map(|j| Job {
            cargo: clean_text(j.cargo.as_deref(), TEXT_MAX),
            source_city: clean_text(j.source_city.as_deref(), TEXT_MAX),
            target_city: clean_text(j.target_city.as_deref(), TEXT_MAX),
        });
        Ok(Heartbeat {
            plate: plate.to_string(),
            odometer_km: number(self.odometer_km, 100_000_000.0, "Der Kilometerstand")?,
            engine: number(wear.engine, 1.0, "Der Verschleiß des Motors")?,
            transmission: number(wear.transmission, 1.0, "Der Verschleiß des Getriebes")?,
            cabin: number(wear.cabin, 1.0, "Der Verschleiß der Kabine")?,
            chassis: number(wear.chassis, 1.0, "Der Verschleiß des Fahrgestells")?,
            wheels: number(wear.wheels, 1.0, "Der Verschleiß der Räder")?,
            fuel_relative: number(self.fuel_relative, 1.0, "Der Tankstand")?,
            fuel_liters: number(self.fuel_liters, 100_000.0, "Der Tankinhalt")?,
            near_city: clean_text(self.near_city.as_deref(), TEXT_MAX),
            speed_kmh: number(self.speed_kmh, 1000.0, "Die Geschwindigkeit")?,
            // Ein Auftrag ganz ohne Angaben zählt als keiner
            job: job.filter(|j| {
                j.cargo.is_some() || j.source_city.is_some() || j.target_city.is_some()
            }),
        })
    }
}

impl Heartbeat {
    /// Alle gemeldeten Werte als JSON für `presence.data`.
    pub fn data(&self) -> Value {
        let mut wear = Map::new();
        for (k, v) in [
            ("engine", self.engine),
            ("transmission", self.transmission),
            ("cabin", self.cabin),
            ("chassis", self.chassis),
            ("wheels", self.wheels),
        ] {
            if let Some(v) = v {
                wear.insert(k.into(), json!(v));
            }
        }
        json!({
            "plate": self.plate,
            "odometer_km": self.odometer_km,
            "wear": wear,
            "fuel_relative": self.fuel_relative,
            "fuel_liters": self.fuel_liters,
            "near_city": self.near_city,
            "speed_kmh": self.speed_kmh,
            "job": self.job.as_ref().map(job_json),
        })
    }

    /// Mischt die gemeldeten Werte in einen Zustand, wie er in `trucks.state` steht
    /// (Feldnamen von `spedhub_save::TruckState`). Was nicht gemeldet wurde, bleibt.
    /// `odometer` ist der schon geprüfte Kilometerstand.
    pub fn merge_into(&self, state: &mut Map<String, Value>, odometer: Option<f64>) {
        if let Some(km) = odometer {
            state.insert("odometer".into(), json!(km.floor() as i64));
            // Unter 1, wie im Spielstand
            let fraction = (km - km.floor()).clamp(0.0, 0.999);
            state.insert("odometer_float_part".into(), json!(fraction));
        }
        for (k, v) in [
            ("engine_wear", self.engine),
            ("transmission_wear", self.transmission),
            ("cabin_wear", self.cabin),
            ("chassis_wear", self.chassis),
            ("fuel_relative", self.fuel_relative),
        ] {
            if let Some(v) = v {
                state.insert(k.into(), json!(v));
            }
        }
        // Die Telemetrie kennt nur einen Wert für alle Räder. Der gilt für jede Achse.
        if let Some(w) = self.wheels {
            let n = state
                .get("wheels_wear")
                .and_then(Value::as_array)
                .map_or(1, |a| a.len().max(1));
            state.insert("wheels_wear".into(), json!(vec![w; n]));
        }
    }
}

pub fn job_json(j: &Job) -> Value {
    json!({ "cargo": j.cargo, "source_city": j.source_city, "target_city": j.target_city })
}

/// Der Firmen-Lkw zu einem Kennzeichen, über das aktuelle oder ein altes Kennzeichen.
/// Passen mehrere Lkw, gewinnt der, den `user_id` gerade geliehen hat, egal über welches
/// Kennzeichen. Sonst gewinnt ein aktuelles Kennzeichen, dann das zuletzt ersetzte alte.
///
/// Der Launcher setzt einen Lkw mit dem Kennzeichen aus dem Bauplan ein. Hat das Spiel das
/// Kennzeichen einmal ersetzt, fährt der Lkw also wieder mit seinem alten, das inzwischen
/// ein anderer Lkw als aktuelles tragen kann.
///
/// Ausgemusterte und verkaufte Lkw zählen nicht. Steht ein verkaufter Lkw noch in einem
/// Spielstand und wird gefahren, ist das kein Firmen-Lkw mehr und wird nicht als fremd
/// gefahren markiert.
pub fn find_truck(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    plate: &str,
) -> rusqlite::Result<Option<i64>> {
    let key = plate_key(plate);
    if key.is_empty() {
        return Ok(None);
    }
    // Erst die aktuellen Kennzeichen, dann die alten, das zuletzt ersetzte zuerst
    let current: Vec<(i64, String)> = conn
        .prepare(
            "SELECT id, plate FROM trucks WHERE company_id = ?1 AND retired_at IS NULL ORDER BY id",
        )?
        .query_map([company_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let old: Vec<(i64, String)> = conn
        .prepare(
            "SELECT p.truck_id, p.plate FROM truck_plates p JOIN trucks t ON t.id = p.truck_id
             WHERE t.company_id = ?1 AND t.retired_at IS NULL
             ORDER BY p.replaced_at DESC, p.rowid DESC",
        )?
        .query_map([company_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut candidates: Vec<i64> = Vec::new();
    for (id, p) in current.into_iter().chain(old) {
        if plate_key(&p) == key && !candidates.contains(&id) {
            candidates.push(id);
        }
    }
    for &id in &candidates {
        if lease_holder(conn, id)? == Some(user_id) {
            return Ok(Some(id));
        }
    }
    Ok(candidates.first().copied())
}

/// Wer den Lkw gerade geliehen hat.
pub fn lease_holder(conn: &Connection, truck_id: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT user_id FROM leases WHERE truck_id = ?1 AND returned_at IS NULL",
        [truck_id],
        |r| r.get(0),
    )
    .optional()
}

/// Was beim Herzschlag passiert ist.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    /// Der passende Firmen-Lkw, falls es einen gibt
    pub truck_id: Option<i64>,
    /// Der Fahrer hat ihn gerade geliehen
    pub own_lease: bool,
}

/// Speichert einen Herzschlag: `presence` überschreiben und, beim eigenen geliehenen Lkw,
/// den letzten bekannten Zustand setzen.
pub fn record(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    hb: &Heartbeat,
    now: i64,
) -> rusqlite::Result<Recorded> {
    let truck_id = find_truck(conn, company_id, user_id, &hb.plate)?;
    let own_lease = match truck_id {
        Some(id) => lease_holder(conn, id)? == Some(user_id),
        None => false,
    };
    if let (Some(id), true) = (truck_id, own_lease) {
        let (odometer, state, last_state): (i64, String, Option<String>) = conn.query_row(
            "SELECT odometer_km, state, last_state FROM trucks WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        // Ausgangspunkt: der letzte bekannte Zustand dieser Leihe, sonst der beim Ausleihen
        let base = last_state.unwrap_or(state);
        let mut merged = match serde_json::from_str::<Value>(&base) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        };
        // Nie kleiner als der Stand beim Ausleihen
        let km = hb.odometer_km.filter(|&km| km >= odometer as f64);
        hb.merge_into(&mut merged, km);
        let last_km = km.map(|k| k.floor() as i64);
        conn.execute(
            "UPDATE trucks SET last_seen_at = ?1, last_state = ?2,
                    last_odometer_km = COALESCE(?3, last_odometer_km)
             WHERE id = ?4",
            params![now, Value::Object(merged).to_string(), last_km, id],
        )?;
    }
    conn.execute(
        "INSERT INTO presence (company_id, user_id, last_seen, plate, data, truck_id, own_lease)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (company_id, user_id) DO UPDATE SET
            last_seen = excluded.last_seen, plate = excluded.plate, data = excluded.data,
            truck_id = excluded.truck_id, own_lease = excluded.own_lease",
        params![
            company_id,
            user_id,
            now,
            hb.plate,
            hb.data().to_string(),
            truck_id,
            own_lease
        ],
    )?;
    Ok(Recorded {
        truck_id,
        own_lease,
    })
}

/// Der letzte Herzschlag eines Fahrers.
#[derive(Debug, Clone)]
pub struct Presence {
    pub user_id: i64,
    pub last_seen: i64,
    pub plate: String,
    pub data: Value,
    pub truck_id: Option<i64>,
    pub own_lease: bool,
}

impl Presence {
    pub fn online(&self, now: i64) -> bool {
        self.last_seen >= now - ONLINE_SECS
    }

    pub fn near_city(&self) -> Value {
        self.data.get("near_city").cloned().unwrap_or(Value::Null)
    }

    pub fn speed_kmh(&self) -> Value {
        self.data.get("speed_kmh").cloned().unwrap_or(Value::Null)
    }

    pub fn job(&self) -> Value {
        self.data.get("job").cloned().unwrap_or(Value::Null)
    }
}

/// Alle letzten Herzschläge einer Firma, je Fahrer.
pub fn all(
    conn: &Connection,
    company_id: i64,
) -> rusqlite::Result<std::collections::HashMap<i64, Presence>> {
    let mut stmt = conn.prepare(
        "SELECT user_id, last_seen, plate, data, truck_id, own_lease FROM presence
         WHERE company_id = ?1",
    )?;
    let rows = stmt.query_map([company_id], |r| {
        let data: String = r.get(3)?;
        Ok(Presence {
            user_id: r.get(0)?,
            last_seen: r.get(1)?,
            plate: r.get(2)?,
            data: serde_json::from_str(&data).unwrap_or(Value::Null),
            truck_id: r.get(4)?,
            own_lease: r.get(5)?,
        })
    })?;
    rows.map(|p| p.map(|p| (p.user_id, p))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kennzeichen_vergleichen() {
        assert_eq!(plate_key("SPED 001"), "SPED 001");
        assert_eq!(plate_key(" sped   001|_germany "), "SPED 001");
        assert_eq!(
            plate_key(
                "DD<offset hshift=4 vshift=-5><img src=/a.mat><offset hshift=4>RI 772|germany"
            ),
            "DD RI 772"
        );
        assert_eq!(plate_key("<img src=/a.mat>"), "");
    }

    #[test]
    fn texte_bereinigen() {
        assert_eq!(clean_text(None, 64), None);
        assert_eq!(clean_text(Some("  "), 64), None);
        assert_eq!(
            clean_text(Some(" Holz\u{0}\n bretter "), 64),
            Some("Holz bretter".into())
        );
        assert_eq!(clean_text(Some("abcdef"), 3), Some("abc".into()));
    }

    #[test]
    fn zustand_mischen() {
        let hb = HeartbeatReq {
            plate: "X".into(),
            wear: Some(WearReq {
                engine: Some(0.25),
                wheels: Some(0.5),
                ..Default::default()
            }),
            ..Default::default()
        }
        .validate()
        .unwrap();
        let mut state = serde_json::from_value::<Map<String, Value>>(json!({
            "odometer": 1000, "engine_wear": 0.1, "cabin_wear": 0.2, "wheels_wear": [0.0, 0.0, 0.0]
        }))
        .unwrap();
        hb.merge_into(&mut state, Some(1234.5));
        assert_eq!(state["odometer"], 1234);
        assert_eq!(state["odometer_float_part"], 0.5);
        assert_eq!(state["engine_wear"], 0.25);
        assert_eq!(state["cabin_wear"], 0.2, "nicht gemeldet, bleibt");
        assert_eq!(state["wheels_wear"], json!([0.5, 0.5, 0.5]));
    }
}
