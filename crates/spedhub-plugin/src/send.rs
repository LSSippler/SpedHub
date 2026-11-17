//! Ein Event an den Server schicken.

use crate::config::Config;
use crate::event::Booking;
use crate::telemetry::Heartbeat;
use std::time::Duration;

/// Was mit einem Event passiert ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Gebucht. `duplicate` heißt: Der Server kannte die `event_id` schon.
    Sent { duplicate: bool },
    /// Gerade nicht möglich (Server weg, Zeitüberschreitung, Serverfehler, Anmeldung
    /// abgelaufen). Später noch einmal.
    Retry(String),
    /// Der Server lehnt das Event dauerhaft ab (400). Erneut senden hilft nicht.
    Rejected(String),
}

pub struct Sender {
    agent: ureq::Agent,
}

impl Sender {
    pub fn new(timeout: Duration) -> Sender {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .timeout_connect(Some(timeout.min(Duration::from_secs(2))))
            .http_status_as_error(false)
            // Proxy-Variablen aus der Umgebung bewusst nicht beachten: Auf einem Spiele-PC
            // sind sie selten gewollt, und der Server läuft oft auf 127.0.0.1.
            .proxy(None)
            .user_agent(concat!("spedhub-plugin/", env!("CARGO_PKG_VERSION")))
            .build();
        Sender {
            agent: config.into(),
        }
    }

    pub fn send(&self, config: &Config, company_id: i64, booking: &Booking) -> Outcome {
        let url = format!("{}/api/companies/{company_id}/events", config.server_url);
        let response = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", config.token))
            .send_json(booking);
        let mut response = match response {
            Ok(r) => r,
            Err(e) => return Outcome::Retry(format!("Server nicht erreichbar: {e}")),
        };
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(64 * 1024)
            .read_to_string()
            .unwrap_or_default();
        match status {
            200..=299 => Outcome::Sent {
                duplicate: serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v.get("duplicate")?.as_bool())
                    .unwrap_or(false),
            },
            400 | 422 => Outcome::Rejected(format!("{status}: {}", server_error(&body))),
            _ => Outcome::Retry(format!("{status}: {}", server_error(&body))),
        }
    }
}

impl Sender {
    /// Schickt einen Herzschlag. Ein Fehler heißt nur: Dieser Herzschlag ist verloren. Er wird
    /// nicht wiederholt und nicht gespeichert.
    pub fn heartbeat(
        &self,
        config: &Config,
        company_id: i64,
        heartbeat: &Heartbeat,
    ) -> Result<(), String> {
        let url = format!("{}/api/companies/{company_id}/heartbeat", config.server_url);
        let mut response = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", config.token))
            .send_json(heartbeat)
            .map_err(|e| format!("Server nicht erreichbar: {e}"))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(64 * 1024)
            .read_to_string()
            .unwrap_or_default();
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(format!("{status}: {}", server_error(&body)))
        }
    }
}

/// Die Fehlermeldung aus `{ "error": "..." }`, sonst der Anfang der Antwort.
fn server_error(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect())
}
