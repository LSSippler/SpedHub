//! Telemetrie-Plugin für Euro Truck Simulator 2.
//!
//! Das Spiel lädt die DLL aus `<Spielordner>\bin\win_x64\plugins\`. Das Plugin meldet
//! Gameplay-Events (Auftrag abgeliefert, abgebrochen, Strafe, Maut, Fähre, Zug) an den
//! SpedHub-Server. Ins Spiel schreibt es nichts, es nutzt nur das offizielle Telemetry SDK.
//!
//! Aufbau:
//!
//! - [`ffi`]: dünne Schicht zum SDK, kopiert Werte und reicht sie weiter
//! - [`agent`]: Warteschlange und Sender-Thread
//! - [`event`]: Übersetzung der Events in Buchungen
//! - [`ids`]: eindeutige `event_id`s über Neustarts hinweg
//! - [`outbox`]: Postausgang für Events, die noch nicht beim Server sind
//! - [`send`]: HTTP an den Server
//! - [`config`]: `plugin.json` und Ablageorte
//! - [`log`]: Log-Datei mit Rotation
//!
//! Alles außer [`ffi`] lässt sich ohne Spiel testen.
#![cfg_attr(
    not(test),
    warn(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod agent;
pub mod config;
pub mod event;
pub mod ffi;
pub mod ids;
pub mod log;
pub mod outbox;
pub mod send;

pub use agent::{Agent, Settings, Submitted};
pub use config::{Config, ConfigError, Paths};
pub use event::{Booking, GameplayData, translate};
