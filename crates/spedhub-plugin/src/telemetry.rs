//! Laufende Werte aus dem Spiel für den Herzschlag.
//!
//! Die FFI-Schicht schreibt Kanäle (Kilometerstand, Verschleiß, Tank, Geschwindigkeit) und
//! Konfigurationen (Lkw, Auftrag) in einen [`Snapshot`]. Der Herzschlag-Thread liest ihn alle
//! paar Sekunden und macht daraus einen [`Heartbeat`]. Beide greifen über [`Telemetry`] zu,
//! einen Mutex, der nur für das Kopieren einzelner Werte gehalten wird.
//!
//! Eine Stadt in der Nähe liefert das Telemetry SDK nicht. `near_city` bleibt deshalb leer.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

/// Längste Texte (Kennzeichen, Fracht, Städte) in Zeichen, wie beim Server.
pub const TEXT_MAX: usize = 64;

/// Ein Auftrag aus der Job-Konfiguration des SDK.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInfo {
    pub cargo: Option<String>,
    pub source_city: Option<String>,
    pub target_city: Option<String>,
}

impl JobInfo {
    /// Aus den Rohwerten der Konfiguration. Leere Texte zählen als fehlend.
    pub fn new(
        cargo: Option<String>,
        source_city: Option<String>,
        target_city: Option<String>,
    ) -> JobInfo {
        JobInfo {
            cargo: clean(cargo.as_deref()),
            source_city: clean(source_city.as_deref()),
            target_city: clean(target_city.as_deref()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cargo.is_none() && self.source_city.is_none() && self.target_city.is_none()
    }
}

/// Text ohne Steuerzeichen, Leerraum zusammengefasst, höchstens [`TEXT_MAX`] Zeichen.
pub fn clean(s: Option<&str>) -> Option<String> {
    let words: Vec<&str> = s?
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect();
    let text: String = words.join(" ").chars().take(TEXT_MAX).collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Der lesbare Teil eines Kennzeichens aus dem Spiel: ohne Markup wie `<img src=...>` und
/// ohne Land hinter `|`. `DD<offset hshift=4><img src=...>RI 772` wird zu `DD RI 772`.
/// Der Server vergleicht genauso, siehe `spedhub_save::plate_text`.
pub fn readable_plate(raw: &str) -> String {
    let text = raw.rsplit_once('|').map_or(raw, |(t, _)| t);
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    clean(Some(&out)).unwrap_or_default()
}

/// Was das Plugin gerade über das Spiel weiß. Fehlt ein Wert, ist er `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Das Spiel ist pausiert oder im Menü. Am Anfang immer, bis das SDK `started` meldet.
    pub paused: bool,
    /// Kennzeichen des gefahrenen Lkw, lesbar (siehe [`readable_plate`])
    pub plate: Option<String>,
    /// Tankinhalt bei vollem Tank in Litern, aus der Lkw-Konfiguration
    pub fuel_capacity: Option<f32>,
    pub odometer_km: Option<f32>,
    pub wear_engine: Option<f32>,
    pub wear_transmission: Option<f32>,
    pub wear_cabin: Option<f32>,
    pub wear_chassis: Option<f32>,
    pub wear_wheels: Option<f32>,
    /// Tankinhalt in Litern
    pub fuel_liters: Option<f32>,
    /// Geschwindigkeit in Metern je Sekunde, rückwärts negativ
    pub speed_ms: Option<f32>,
    /// Der laufende Auftrag
    pub job: Option<JobInfo>,
    /// Der letzte nicht leere Auftrag. Das SDK leert die Job-Konfiguration manchmal schon
    /// vor dem Event `job.delivered`, deshalb bleibt er hier, bis er gebucht ist.
    pub last_job: Option<JobInfo>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            paused: true,
            plate: None,
            fuel_capacity: None,
            odometer_km: None,
            wear_engine: None,
            wear_transmission: None,
            wear_cabin: None,
            wear_chassis: None,
            wear_wheels: None,
            fuel_liters: None,
            speed_ms: None,
            job: None,
            last_job: None,
        }
    }
}

impl Snapshot {
    /// Neue Lkw-Konfiguration. Ein leeres Kennzeichen heißt: kein Lkw.
    pub fn set_truck(&mut self, plate: Option<&str>, fuel_capacity: Option<f32>) {
        self.plate = plate.map(readable_plate).filter(|p| !p.is_empty());
        self.fuel_capacity = fuel_capacity.filter(|c| c.is_finite() && *c > 0.0);
    }

    /// Neue Job-Konfiguration. `None` oder ein leerer Auftrag heißt: keiner. Der letzte nicht
    /// leere bleibt in [`Snapshot::last_job`].
    pub fn set_job(&mut self, job: Option<JobInfo>) {
        let job = job.filter(|j| !j.is_empty());
        if job.is_some() {
            self.last_job.clone_from(&job);
        }
        self.job = job;
    }

    /// Der Herzschlag, der jetzt rausgehen soll. `None`, wenn das Spiel pausiert ist oder
    /// kein Kennzeichen bekannt ist.
    pub fn heartbeat(&self) -> Option<Heartbeat> {
        if self.paused {
            return None;
        }
        let plate = self.plate.clone().filter(|p| !p.is_empty())?;
        let unit = |v: Option<f32>| {
            v.filter(|f| f.is_finite())
                .map(|f| f64::from(f.clamp(0.0, 1.0)))
        };
        let positive = |v: Option<f32>| v.filter(|f| f.is_finite() && *f >= 0.0).map(f64::from);
        let wear = Wear {
            engine: unit(self.wear_engine),
            transmission: unit(self.wear_transmission),
            cabin: unit(self.wear_cabin),
            chassis: unit(self.wear_chassis),
            wheels: unit(self.wear_wheels),
        };
        let fuel_liters = positive(self.fuel_liters);
        let fuel_relative = match (fuel_liters, self.fuel_capacity) {
            (Some(l), Some(c)) if c > 0.0 => Some((l / f64::from(c)).clamp(0.0, 1.0)),
            _ => None,
        };
        Some(Heartbeat {
            plate,
            odometer_km: positive(self.odometer_km),
            wear: (!wear.is_empty()).then_some(wear),
            fuel_relative,
            fuel_liters,
            near_city: None,
            speed_kmh: self
                .speed_ms
                .filter(|v| v.is_finite())
                .map(|v| (f64::from(v.abs()) * 3.6).min(1000.0)),
            job: self.job.clone(),
            profile: None,
        })
    }
}

/// Verschleiß, je 0 bis 1.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Wear {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transmission: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cabin: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chassis: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheels: Option<f64>,
}

impl Wear {
    fn is_empty(&self) -> bool {
        self == &Wear::default()
    }
}

/// Ein Herzschlag, wie er an `POST /api/companies/{id}/heartbeat` geht.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub plate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub odometer_km: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wear: Option<Wear>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_relative: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_liters: Option<f64>,
    /// Gibt das SDK nicht her, bleibt immer leer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near_city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_kmh: Option<f64>,
    /// Der laufende Auftrag, `null` heißt: keiner
    #[serde(default)]
    pub job: Option<JobInfo>,
    /// Name des aktiven ETS2-Profils. Setzt der Herzschlag-Thread, wenn es das Firmenprofil ist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// Gemeinsamer Zugriff auf den [`Snapshot`] für Spiel-Thread und Herzschlag-Thread.
#[derive(Debug, Clone, Default)]
pub struct Telemetry(Arc<Mutex<Snapshot>>);

impl Telemetry {
    fn lock(&self) -> MutexGuard<'_, Snapshot> {
        // Ein Fehler in einem anderen Thread soll das Spiel nicht mitreißen
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ändert den Snapshot. Hält den Mutex nur für die Dauer von `f`.
    pub fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        f(&mut self.lock());
    }

    /// Eine Kopie des Snapshots.
    pub fn snapshot(&self) -> Snapshot {
        self.lock().clone()
    }

    pub fn set_paused(&self, paused: bool) {
        self.update(|s| s.paused = paused);
    }

    /// Nimmt den letzten Auftrag heraus, wenn er gebucht wird.
    pub fn take_last_job(&self) -> Option<JobInfo> {
        self.lock().last_job.take()
    }
}
