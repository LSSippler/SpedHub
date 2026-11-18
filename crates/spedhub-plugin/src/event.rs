//! Übersetzung der Gameplay-Events aus dem Telemetry SDK in Buchungen für den Server.
//!
//! | SDK-Event | Betrag aus | Server-Typ |
//! |---|---|---|
//! | `job.delivered` | `revenue` | `job_delivered` |
//! | `job.cancelled` | `cancel.penalty` | `job_cancelled` |
//! | `player.fined` | `fine.amount` | `fined` |
//! | `player.tollgate.paid` | `pay.amount` | `tollgate` |
//! | `player.use.ferry` | `pay.amount` | `ferry` |
//! | `player.use.train` | `pay.amount` | `train` |
//!
//! Beträge sind in der Spielwährung, bei ETS2 also Euro. Bei `job_delivered` geht außerdem die
//! Strecke als `distance_km` mit, der Server schreibt sie dem Fahrer als Firmenkilometer gut.

use crate::telemetry::JobInfo;
use serde::{Deserialize, Serialize};

/// Die Werte eines Gameplay-Events, so wie die FFI-Schicht sie aus dem SDK kopiert.
/// Fehlt ein Attribut, ist das Feld `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameplayData {
    /// Kennung des Events, zum Beispiel `job.delivered`
    pub id: String,
    pub revenue: Option<i64>,
    pub cancel_penalty: Option<i64>,
    pub fine_amount: Option<i64>,
    pub fine_offence: Option<String>,
    pub pay_amount: Option<i64>,
    pub distance_km: Option<f32>,
    pub cargo_damage: Option<f32>,
    pub source_name: Option<String>,
    pub target_name: Option<String>,
}

/// Eine Buchung, wie sie an `POST /api/companies/{id}/events` geht.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Booking {
    pub event_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub amount: i64,
    /// Gefahrene Kilometer, gerundet. Nur bei `job_delivered`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_km: Option<i64>,
    pub note: String,
    /// Wann das Event im Spiel passiert ist: Systemzeit dieses Rechners, Unix-Sekunden UTC.
    /// Der Server ordnet das Event damit dem richtigen Geldabgleich zu, auch wenn es erst
    /// später ankommt. Ältere Zeilen im Postausgang haben das Feld nicht.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<i64>,
    /// Fracht, Start- und Zielstadt, nur bei `job_delivered`. Aus der letzten nicht leeren
    /// Job-Konfiguration des SDK. Ältere Zeilen im Postausgang haben die Felder nicht.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_city: Option<String>,
    /// Name des ETS2-Profils, das beim Annehmen aktiv war. Das Plugin nimmt nur Ereignisse aus
    /// dem Firmenprofil an, der Server prüft es noch einmal. Ältere Zeilen im Postausgang haben
    /// das Feld nicht.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

impl Booking {
    /// Hängt Fracht und Städte eines Auftrags an.
    pub fn with_job(self, job: Option<JobInfo>) -> Booking {
        let job = job.unwrap_or_default();
        Booking {
            cargo: job.cargo,
            source_city: job.source_city,
            target_city: job.target_city,
            ..self
        }
    }

    /// Hängt den Namen des aktiven Profils an.
    pub fn with_profile(self, profile: &str) -> Booking {
        Booking {
            profile: Some(profile.to_string()),
            ..self
        }
    }
}

/// Jetzt als Unix-Sekunden UTC, nach der Systemuhr.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Eine Buchung ohne `event_id`. Die vergibt der Aufrufer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub kind: &'static str,
    pub amount: i64,
    pub distance_km: Option<i64>,
    pub note: String,
}

impl Draft {
    /// Die Buchung mit `event_id`, ohne Zeitpunkt.
    pub fn with_id(self, event_id: String) -> Booking {
        Booking {
            event_id,
            kind: self.kind.to_string(),
            amount: self.amount,
            distance_km: self.distance_km,
            note: self.note,
            occurred_at: None,
            cargo: None,
            source_city: None,
            target_city: None,
            profile: None,
        }
    }

    /// Die Buchung mit `event_id` und dem Zeitpunkt im Spiel (Unix-Sekunden UTC).
    pub fn with_id_at(self, event_id: String, occurred_at: i64) -> Booking {
        Booking {
            occurred_at: Some(occurred_at),
            ..self.with_id(event_id)
        }
    }
}

/// Übersetzt ein Event.
///
/// - `Ok(Some(_))`: eine Buchung
/// - `Ok(None)`: ein Event, das SpedHub nicht bucht
/// - `Err(_)`: ein gebuchtes Event mit fehlendem oder unbrauchbarem Betrag. Die Meldung
///   gehört ins Log.
pub fn translate(data: &GameplayData) -> Result<Option<Draft>, String> {
    let (kind, attribute, amount, note) = match data.id.as_str() {
        "job.delivered" => (
            "job_delivered",
            "revenue",
            data.revenue,
            delivered_note(data),
        ),
        "job.cancelled" => (
            "job_cancelled",
            "cancel.penalty",
            data.cancel_penalty,
            "Auftrag abgebrochen".to_string(),
        ),
        "player.fined" => (
            "fined",
            "fine.amount",
            data.fine_amount,
            fine_note(data.fine_offence.as_deref()),
        ),
        "player.tollgate.paid" => ("tollgate", "pay.amount", data.pay_amount, "Maut".into()),
        "player.use.ferry" => (
            "ferry",
            "pay.amount",
            data.pay_amount,
            route_note("Fähre", data),
        ),
        "player.use.train" => (
            "train",
            "pay.amount",
            data.pay_amount,
            route_note("Zug", data),
        ),
        _ => return Ok(None),
    };
    let amount = amount.ok_or_else(|| format!("{}: {attribute} fehlt, nicht gebucht", data.id))?;
    if amount < 0 {
        return Err(format!(
            "{}: negativer Betrag {amount}, nicht gebucht",
            data.id
        ));
    }
    let distance_km = match kind {
        "job_delivered" => data
            .distance_km
            .filter(|v| v.is_finite())
            .map(|v| v.max(0.0).round() as i64),
        _ => None,
    };
    Ok(Some(Draft {
        kind,
        amount,
        distance_km,
        note,
    }))
}

fn delivered_note(data: &GameplayData) -> String {
    let mut parts = vec!["Auftrag abgeliefert".to_string()];
    if let Some(km) = data.distance_km.filter(|v| v.is_finite()) {
        parts.push(format!("{} km", km.max(0.0).round()));
    }
    if let Some(damage) = data.cargo_damage.filter(|v| v.is_finite()) {
        let percent = format!("{:.1}", (damage * 100.0).clamp(0.0, 100.0)).replace('.', ",");
        parts.push(format!("Frachtschaden {percent} %"));
    }
    parts.join(", ")
}

fn fine_note(offence: Option<&str>) -> String {
    match offence {
        Some(o) => match offence_label(o) {
            Some(label) => format!("Strafe: {label}"),
            None => format!("Strafe: {o}"),
        },
        None => "Strafe".into(),
    }
}

/// Deutsche Bezeichnungen der Werte von `fine.offence` aus SDK 1.14.
pub fn offence_label(offence: &str) -> Option<&'static str> {
    Some(match offence {
        "crash" => "Unfall",
        "avoid_sleeping" => "Ruhezeit nicht eingehalten",
        "wrong_way" => "Falschfahrt",
        "speeding_camera" => "Blitzer",
        "no_lights" => "Ohne Licht",
        "red_signal" => "Rote Ampel",
        "speeding" => "Zu schnell",
        "avoid_weighing" => "Waage umfahren",
        "illegal_trailer" => "Unerlaubter Anhänger",
        "avoid_inspection" => "Kontrolle umfahren",
        "illegal_border_crossing" => "Unerlaubter Grenzübertritt",
        "hard_shoulder_violation" => "Auf dem Standstreifen",
        "damaged_vehicle_usage" => "Mit beschädigtem Fahrzeug",
        "generic" => "Allgemein",
        _ => return None,
    })
}

fn route_note(what: &str, data: &GameplayData) -> String {
    let clean = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    match (clean(&data.source_name), clean(&data.target_name)) {
        (Some(a), Some(b)) => format!("{what} {a} nach {b}"),
        (None, Some(b)) => format!("{what} nach {b}"),
        _ => what.to_string(),
    }
}
