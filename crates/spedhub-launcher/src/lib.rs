//! Launcher für SpedHub. Diese Bibliothek enthält alles außer der Oberfläche, damit es sich
//! ohne Fenster testen lässt.
//!
//! | Modul | Inhalt |
//! |---|---|
//! | [`cli`] | Kommandozeile: Plugin installieren und entfernen |
//! | [`client`] | Server-Client |
//! | [`flows`] | Abläufe mit Server und Spielstand: Bauplan holen, Geldabgleich |
//! | [`plugin_install`] | Plugin ins Spiel kopieren, ersetzen, entfernen, Stand prüfen |
//! | [`profiles`] | ETS2-Profile, Hex-Namen, Speicherplätze |
//! | [`savegame`] | Lkw übernehmen, einsetzen, für die Rückgabe lesen (über `spedhub-save`) |
//! | [`settings`] | Einstellungen mit verschlüsseltem Token, `plugin.json` |
//! | [`steam`] | ETS2 über Steam finden: Registry-Werte, `libraryfolders.vdf`, App-Manifest |
//! | [`system`] | Windows-Anbindung: Dokumente, laufendes Spiel, Steam, DPAPI, Registry, Dialoge |
//! | [`update`] | Neue Version über GitHub-Releases finden, Setup laden und prüfen |
//! | [`version`] | Versionen lesen und vergleichen, passen Launcher und Server zusammen? |

pub mod cli;
pub mod client;
pub mod error;
pub mod flows;
pub mod plugin_install;
pub mod profiles;
pub mod savegame;
pub mod settings;
pub mod steam;
pub mod system;
pub mod update;
pub mod version;

pub use error::{Error, Result};
