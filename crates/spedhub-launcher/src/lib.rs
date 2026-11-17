//! Launcher für SpedHub. Diese Bibliothek enthält alles außer der Oberfläche, damit es sich
//! ohne Fenster testen lässt.
//!
//! | Modul | Inhalt |
//! |---|---|
//! | [`client`] | Server-Client |
//! | [`flows`] | Abläufe mit Server und Spielstand: Bauplan holen, Geldabgleich |
//! | [`profiles`] | ETS2-Profile, Hex-Namen, Speicherplätze |
//! | [`savegame`] | Lkw übernehmen, einsetzen, für die Rückgabe lesen (über `spedhub-save`) |
//! | [`settings`] | Einstellungen mit verschlüsseltem Token, `plugin.json` |
//! | [`system`] | Windows-Anbindung: Dokumente, laufendes Spiel, Steam, DPAPI |

pub mod client;
pub mod error;
pub mod flows;
pub mod profiles;
pub mod savegame;
pub mod settings;
pub mod system;

pub use error::{Error, Result};
