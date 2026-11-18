//! Anbindung an das Spiel über das offizielle SCS Telemetry SDK, mit `scs-sdk-plugin`.
//!
//! Das Makro `export_plugin!` erzeugt die beiden Funktionen, die das Spiel in der DLL sucht
//! (`scs_telemetry_init`, `scs_telemetry_shutdown`). Das Framework fängt an diesen Einstiegen
//! jede Panic ab. Zur Sicherheit ist hier jeder eigene Hook noch einmal in `catch_unwind`
//! gepackt, damit auch ein Fehler in SpedHub-Code nie bis ins Spiel durchschlägt.
//!
//! Diese Schicht kopiert nur die Werte aus dem SDK in [`GameplayData`] und in den
//! [`Snapshot`](crate::telemetry::Snapshot) des [`Agent`] und reicht sie weiter. Die ganze
//! Logik liegt in den anderen Modulen.
//!
//! Für den Herzschlag abonniert: die Kanäle `truck.odometer`, `truck.wear.engine`,
//! `truck.wear.transmission`, `truck.wear.cabin`, `truck.wear.chassis`, `truck.wear.wheels`,
//! `truck.fuel.amount` und `truck.speed`, die Konfigurationen `truck` (`license.plate`,
//! `fuel.capacity`, `brand_id`, `id`) und `job` (`cargo`, `source.city`, `destination.city`) und die Ereignisse
//! `paused` und `started`. Alles davon ist optional: Fehlt etwas im Spiel, bleibt der Wert leer,
//! und die DLL lädt trotzdem. Eine Stadt in der Nähe liefert das SDK nicht.

use crate::agent::{Agent, Settings};
use crate::config::Paths;
use crate::event::GameplayData;
use crate::telemetry::JobInfo;
use scs_sdk_plugin::sdk::channels::truck as ch;
use scs_sdk_plugin::sdk::configuration::{attributes as cfg, ids as cfg_ids};
use scs_sdk_plugin::sdk::gameplay::attributes as attr;
use scs_sdk_plugin::sdk::{LogLevel, TelemetryApiVersion, game};
use scs_sdk_plugin::{
    ChannelUpdate, ConfigurationEvent, Game, GameCompatibility, GameplayEvent, PluginCompatibility,
    PluginContext, PluginMetadata, PluginResult, TelemetryEvent, TelemetryEventKind,
    TelemetryPlugin,
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
                // Für den Herzschlag, alles optional
                context.subscribe_event_optional(TelemetryEventKind::Paused)?;
                context.subscribe_event_optional(TelemetryEventKind::Started)?;
                context.subscribe_event_optional(TelemetryEventKind::Configuration)?;
                context.subscribe_optional(ch::ODOMETER)?;
                context.subscribe_optional(ch::WEAR_ENGINE)?;
                context.subscribe_optional(ch::WEAR_TRANSMISSION)?;
                context.subscribe_optional(ch::WEAR_CABIN)?;
                context.subscribe_optional(ch::WEAR_CHASSIS)?;
                context.subscribe_optional(ch::WEAR_WHEELS)?;
                context.subscribe_optional(ch::FUEL)?;
                context.subscribe_optional(ch::SPEED)?;
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
        let _ = catch_unwind(AssertUnwindSafe(|| match event {
            TelemetryEvent::Gameplay(gameplay) => {
                agent.submit(&copy_gameplay(gameplay));
            }
            TelemetryEvent::Paused => agent.telemetry().set_paused(true),
            TelemetryEvent::Started => agent.telemetry().set_paused(false),
            TelemetryEvent::Configuration(c) => copy_configuration(agent, c),
            _ => {}
        }));
    }

    fn channel(&mut self, _context: &mut PluginContext<'_>, update: ChannelUpdate<'_>) {
        let Some(agent) = &self.agent else { return };
        let _ = catch_unwind(AssertUnwindSafe(|| copy_channel(agent, update)));
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

/// Kopiert einen Kanalwert in den Snapshot.
fn copy_channel(agent: &Agent, u: ChannelUpdate<'_>) {
    let value = |c| u.value(c);
    agent.telemetry().update(|s| {
        if u.is(ch::ODOMETER) {
            s.odometer_km = value(ch::ODOMETER);
        } else if u.is(ch::WEAR_ENGINE) {
            s.wear_engine = value(ch::WEAR_ENGINE);
        } else if u.is(ch::WEAR_TRANSMISSION) {
            s.wear_transmission = value(ch::WEAR_TRANSMISSION);
        } else if u.is(ch::WEAR_CABIN) {
            s.wear_cabin = value(ch::WEAR_CABIN);
        } else if u.is(ch::WEAR_CHASSIS) {
            s.wear_chassis = value(ch::WEAR_CHASSIS);
        } else if u.is(ch::WEAR_WHEELS) {
            s.wear_wheels = value(ch::WEAR_WHEELS);
        } else if u.is(ch::FUEL) {
            s.fuel_liters = value(ch::FUEL);
        } else if u.is(ch::SPEED) {
            s.speed_ms = value(ch::SPEED);
        }
    });
}

/// Kopiert die Lkw- und die Job-Konfiguration in den Snapshot. Eine leere Job-Konfiguration
/// heißt: kein Auftrag.
fn copy_configuration(agent: &Agent, c: ConfigurationEvent<'_>) {
    if c.is(cfg_ids::TRUCK) {
        let plate = c.string_owned(cfg::LICENSE_PLATE);
        let capacity = c.get(cfg::FUEL_CAPACITY);
        // Marke und Modell, damit der Server den Lkw nicht nur am Kennzeichen erkennt
        let brand = c.string_owned(cfg::BRAND_ID);
        let model = c.string_owned(cfg::ID);
        agent.telemetry().update(|s| {
            s.set_truck(plate.as_deref(), capacity);
            s.set_truck_model(brand.as_deref(), model.as_deref());
        });
    } else if c.is(cfg_ids::JOB) {
        let job = c.has_attributes().then(|| {
            JobInfo::new(
                c.string_owned(cfg::CARGO),
                c.string_owned(cfg::SOURCE_CITY),
                c.string_owned(cfg::DESTINATION_CITY),
            )
        });
        agent.telemetry().update(|s| s.set_job(job));
    }
}

scs_sdk_plugin::export_plugin!(SpedHubPlugin::default());
