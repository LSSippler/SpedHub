//! Anbindung an das Spiel über das offizielle SCS Telemetry SDK, mit `scs-sdk-plugin`.
//!
//! Das Makro `export_plugin!` erzeugt die beiden Funktionen, die das Spiel in der DLL sucht
//! (`scs_telemetry_init`, `scs_telemetry_shutdown`). Das Framework fängt an diesen Einstiegen
//! jede Panic ab. Zur Sicherheit ist hier jeder eigene Hook noch einmal in `catch_unwind`
//! gepackt, damit auch ein Fehler in SpedHub-Code nie bis ins Spiel durchschlägt.
//!
//! Diese Schicht kopiert nur die Werte aus dem SDK in [`GameplayData`] und reicht sie an den
//! [`Agent`] weiter. Die ganze Logik liegt in den anderen Modulen.

use crate::agent::{Agent, Settings};
use crate::config::Paths;
use crate::event::GameplayData;
use scs_sdk_plugin::sdk::gameplay::attributes as attr;
use scs_sdk_plugin::sdk::{LogLevel, TelemetryApiVersion, game};
use scs_sdk_plugin::{
    Game, GameCompatibility, GameplayEvent, PluginCompatibility, PluginContext, PluginMetadata,
    PluginResult, TelemetryEvent, TelemetryEventKind, TelemetryPlugin,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Gameplay-Events gibt es ab Telemetry API 1.01 und ETS2-Schema 1.14 (ETS2 1.35).
static SUPPORTED_GAMES: [GameCompatibility; 1] = [GameCompatibility::new(
    Game::EuroTruckSimulator2,
    game::ets2::V1_14,
)];

#[derive(Default)]
pub struct SpedHubPlugin {
    agent: Option<Agent>,
}

impl TelemetryPlugin for SpedHubPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("SpedHub", env!("CARGO_PKG_VERSION"))
    }

    fn compatibility(&self) -> PluginCompatibility {
        PluginCompatibility::new(TelemetryApiVersion::V1_01, &SUPPORTED_GAMES)
    }

    fn initialize(&mut self, context: &mut PluginContext<'_>) -> PluginResult {
        let started = catch_unwind(AssertUnwindSafe(|| {
            let Some(paths) = Paths::from_env() else {
                return Err("LOCALAPPDATA ist nicht gesetzt".to_string());
            };
            Agent::start(&paths, Settings::default()).map_err(|e| e.to_string())
        }));
        match started {
            Ok(Ok(agent)) => {
                // Nur wer konfiguriert ist, meldet sich für Events an.
                context.subscribe_event(TelemetryEventKind::Gameplay)?;
                self.agent = Some(agent);
                context.log(LogLevel::Message, format_args!("SpedHub: aktiv"));
            }
            Ok(Err(reason)) => {
                context.log(
                    LogLevel::Warning,
                    format_args!("SpedHub: inaktiv. {reason}"),
                );
            }
            Err(_) => {
                context.log(
                    LogLevel::Error,
                    format_args!("SpedHub: interner Fehler beim Start, Plugin bleibt aus"),
                );
            }
        }
        // Auch ohne Konfiguration Erfolg melden: Das Plugin bleibt dann einfach still.
        Ok(())
    }

    fn event(&mut self, _context: &mut PluginContext<'_>, event: TelemetryEvent<'_>) {
        let Some(agent) = &self.agent else { return };
        let TelemetryEvent::Gameplay(gameplay) = event else {
            return;
        };
        let _ = catch_unwind(AssertUnwindSafe(|| {
            agent.submit(&copy_gameplay(gameplay));
        }));
    }

    fn shutdown(&mut self, _context: &mut PluginContext<'_>) {
        let agent = self.agent.take();
        let _ = catch_unwind(AssertUnwindSafe(move || {
            if let Some(agent) = agent {
                agent.shutdown();
            }
        }));
    }
}

/// Kopiert alle Attribute, die SpedHub braucht. Was fehlt, bleibt `None`.
fn copy_gameplay(g: GameplayEvent<'_>) -> GameplayData {
    GameplayData {
        id: g.id().into_owned(),
        revenue: g.get(attr::REVENUE),
        cancel_penalty: g.get(attr::CANCEL_PENALTY),
        fine_amount: g.get(attr::FINE_AMOUNT),
        fine_offence: g.string_owned(attr::FINE_OFFENCE),
        pay_amount: g.get(attr::PAY_AMOUNT),
        distance_km: g.get(attr::DISTANCE_KM),
        cargo_damage: g.get(attr::CARGO_DAMAGE),
        source_name: g.string_owned(attr::SOURCE_NAME),
        target_name: g.string_owned(attr::TARGET_NAME),
    }
}

scs_sdk_plugin::export_plugin!(SpedHubPlugin::default());
