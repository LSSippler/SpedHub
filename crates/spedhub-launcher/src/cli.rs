//! Kommandozeile des Launchers, ohne Oberfläche.
//!
//! | Aufruf | Was passiert |
//! |---|---|
//! | `spedhub-launcher` | Oberfläche |
//! | `spedhub-launcher --install-plugin [--game-dir <Ordner>] [--report-game-dir <Datei>] [--quiet]` | Plugin ins Spiel kopieren |
//! | `spedhub-launcher --uninstall-plugin [--quiet]` | Plugin aus dem Spiel entfernen |
//! | `spedhub-launcher --remember-game-dir <Ordner> [--quiet]` | Spielordner in den Einstellungen merken |
//! | `spedhub-launcher --version` | Version, unter Windows in einem Fenster |
//!
//! Das Setup ruft `--install-plugin` mit seinen Adminrechten auf, die Deinstallation
//! `--uninstall-plugin`. Die Oberfläche startet sich über UAC mit `--install-plugin` neu.
//! Rückmeldung gibt der Exit-Code ([`EXIT_OK`] und folgende), Fehler zeigt `main.rs` unter
//! Windows zusätzlich in einer MessageBox. Mit `--quiet` gibt es keine Fenster, für eine
//! stille Installation.
//!
//! Mit Adminrechten läuft der Launcher unter Umständen mit einem anderen Konto, wenn ein
//! Standardbenutzer im UAC-Dialog die Anmeldung eines Admins eingibt. Ein dort gewählter
//! Spielordner landet dann in dessen `launcher.json`. Deshalb wählt die Oberfläche den Ordner
//! selbst und gibt ihn mit `--game-dir` weiter. Das Setup lässt sich einen von Hand gewählten
//! Ordner mit `--report-game-dir` in eine Datei schreiben und merkt ihn mit
//! `--remember-game-dir` als ursprünglicher Benutzer.

use crate::error::{Error, Result};
use crate::plugin_install::{self, GameSource, Installed, Removal};
use crate::settings::{AppPaths, Settings};
use crate::steam::{self, RegistryValues};
use crate::system::GameCheck;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Alles erledigt
pub const EXIT_OK: i32 = 0;
/// Fehler, die Meldung steht in der MessageBox
pub const EXIT_ERROR: i32 = 1;
/// Spiel nicht gefunden und kein Ordner gewählt
pub const EXIT_CANCELLED: i32 = 2;
/// Unbekannte Option
pub const EXIT_USAGE: i32 = 3;

pub const USAGE: &str = "spedhub-launcher [--install-plugin [--game-dir <Ordner>] [--report-game-dir <Datei>] | --uninstall-plugin | --remember-game-dir <Ordner>] [--quiet] [--version]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Die Oberfläche starten
    Gui,
    InstallPlugin {
        game_dir: Option<PathBuf>,
        /// Hierhin wird ein von Hand gewählter Spielordner geschrieben ([`write_game_dir_report`])
        report: Option<PathBuf>,
        quiet: bool,
    },
    UninstallPlugin {
        quiet: bool,
    },
    /// Einen Spielordner prüfen und in `launcher.json` merken
    RememberGameDir {
        game_dir: PathBuf,
        quiet: bool,
    },
    Version,
}

/// Der Wert hinter einer Option.
fn value(
    args: &mut dyn Iterator<Item = OsString>,
    option: &str,
    what: &str,
) -> std::result::Result<PathBuf, String> {
    match args.next() {
        Some(v) if !v.is_empty() => Ok(PathBuf::from(v)),
        _ => Err(format!("Zu {option} fehlt {what}.\n{USAGE}")),
    }
}

/// Liest die Kommandozeile ohne den Programmnamen.
pub fn parse_args<I>(args: I) -> std::result::Result<Command, String>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let mut install = false;
    let mut uninstall = false;
    let mut version = false;
    let mut quiet = false;
    let mut game_dir = None;
    let mut report = None;
    let mut remember = None;
    let mut args = args.into_iter().map(Into::into);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--install-plugin") => install = true,
            Some("--uninstall-plugin") => uninstall = true,
            Some("--quiet") => quiet = true,
            Some("--version") => version = true,
            Some("--game-dir") => game_dir = Some(value(&mut args, "--game-dir", "der Ordner")?),
            Some("--report-game-dir") => {
                report = Some(value(&mut args, "--report-game-dir", "die Datei")?)
            }
            Some("--remember-game-dir") => {
                remember = Some(value(&mut args, "--remember-game-dir", "der Ordner")?)
            }
            _ => {
                return Err(format!(
                    "Unbekannte Option {}.\n{USAGE}",
                    arg.to_string_lossy()
                ));
            }
        }
    }
    if [install, uninstall, remember.is_some()]
        .iter()
        .filter(|b| **b)
        .count()
        > 1
    {
        return Err(format!(
            "--install-plugin, --uninstall-plugin und --remember-game-dir gehen nicht zusammen.\n{USAGE}"
        ));
    }
    if (game_dir.is_some() || report.is_some()) && !install {
        return Err(format!(
            "--game-dir und --report-game-dir gibt es nur mit --install-plugin.\n{USAGE}"
        ));
    }
    Ok(if version {
        Command::Version
    } else if install {
        Command::InstallPlugin {
            game_dir,
            report,
            quiet,
        }
    } else if uninstall {
        Command::UninstallPlugin { quiet }
    } else if let Some(game_dir) = remember {
        Command::RememberGameDir { game_dir, quiet }
    } else {
        Command::Gui
    })
}

/// Setzt ein Argument für eine Windows-Kommandozeile in Anführungszeichen, wenn nötig, nach
/// den Regeln, mit denen Rust und die C-Laufzeit sie wieder zerlegen: Backslashes vor einem
/// Anführungszeichen und am Ende werden verdoppelt.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Parameter für den Neustart mit Adminrechten aus der Oberfläche.
pub fn install_parameters(game_dir: Option<&Path>) -> String {
    let mut p = String::from("--install-plugin");
    if let Some(dir) = game_dir {
        p.push_str(" --game-dir ");
        p.push_str(&quote_arg(&dir.to_string_lossy()));
    }
    p
}

/// Was die Installation braucht. In Tests nachgebaut.
pub struct InstallEnv<'a> {
    /// Ablage für `launcher.json`, dort wird ein gewählter Spielordner gemerkt
    pub paths: Option<&'a AppPaths>,
    pub registry: RegistryValues,
    /// Die mitgelieferte DLL
    pub bundled: Option<PathBuf>,
    pub game: &'a dyn GameCheck,
    /// Merkt eine Datei zum Löschen beim Neustart vor ([`crate::system::delete_on_reboot`])
    pub delete_later: &'a dyn Fn(&Path) -> std::result::Result<(), String>,
}

impl InstallEnv<'_> {
    fn remembered(&self) -> Option<PathBuf> {
        self.paths.and_then(|p| Settings::load(p).game_dir)
    }
}

/// Ergebnis von `--install-plugin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed {
        installed: Installed,
        game_dir: PathBuf,
        source: GameSource,
        /// Ein von Hand gewählter Ordner ließ sich nicht in den Einstellungen merken
        not_remembered: Option<String>,
    },
    /// Spiel nicht gefunden und kein Ordner gewählt
    Cancelled { reason: String },
}

/// Text, mit dem nach einem Ordner gefragt wird, wenn ETS2 nicht gefunden wurde.
pub fn choose_prompt(reason: &str) -> String {
    format!(
        "{reason}.\n\nBitte im nächsten Fenster den Ordner von Euro Truck Simulator 2 wählen, meist Steam\\steamapps\\common\\Euro Truck Simulator 2."
    )
}

/// `--install-plugin`. Findet das Spiel ([`plugin_install::locate_game`]) und kopiert die
/// DLL. Wird es nicht gefunden, fragt `choose` nach einem Ordner: Es bekommt den Grund und
/// liefert den gewählten Ordner oder `None` für Abbrechen. Ein falscher Ordner wird mit dem
/// Grund noch einmal angeboten. Ein gewählter oder übergebener Ordner wird in den
/// Einstellungen gemerkt. Passt ein übergebener Ordner nicht, ist das ein Fehler, gefragt wird
/// dann nicht: Wer ihn übergibt, hat schon gewählt.
pub fn install_plugin(
    explicit: Option<&Path>,
    env: &InstallEnv<'_>,
    choose: &mut dyn FnMut(&str) -> Option<PathBuf>,
) -> Result<InstallOutcome> {
    let remembered = env.remembered();
    let (game_dir, source) =
        match plugin_install::locate_game(explicit, remembered.as_deref(), &env.registry) {
            Ok(found) => found,
            Err(reason) if explicit.is_some() => return Err(Error::Other(reason)),
            Err(mut reason) => loop {
                let Some(dir) = choose(&choose_prompt(&reason)) else {
                    return Ok(InstallOutcome::Cancelled { reason });
                };
                match steam::validate_game_dir(&dir) {
                    Ok(d) => break (d, GameSource::Argument),
                    Err(e) => reason = e,
                }
            },
        };
    let bundled = env.bundled.as_deref().ok_or_else(|| {
        Error::Other(
            "Neben dem Launcher liegt keine spedhub_plugin.dll. Bitte SpedHub neu installieren"
                .into(),
        )
    })?;
    let installed = plugin_install::install(bundled, &game_dir, env.game)?;
    let mut not_remembered = None;
    if source == GameSource::Argument
        && let Some(paths) = env.paths
        && let Err(e) = plugin_install::remember_game_dir(paths, &game_dir)
    {
        not_remembered = Some(e.to_string());
    }
    Ok(InstallOutcome::Installed {
        installed,
        game_dir,
        source,
        not_remembered,
    })
}

/// Schreibt einen von Hand gewählten Spielordner für das Setup in eine Datei: UTF-8 mit BOM,
/// damit Inno Setup Umlaute richtig liest, eine Zeile.
pub fn write_game_dir_report(file: &Path, game_dir: &Path) -> Result<()> {
    let text = format!("\u{feff}{}\r\n", game_dir.display());
    std::fs::write(file, text).map_err(|e| Error::io(file, &e))
}

/// `--remember-game-dir`. Prüft den Ordner und merkt ihn in `launcher.json`. Liefert den
/// Spielordner, wie er gemerkt wurde (ein darin gewähltes `bin\win_x64` wird zum Spielordner).
pub fn remember_game_dir(paths: Option<&AppPaths>, game_dir: &Path) -> Result<PathBuf> {
    let paths = paths.ok_or_else(|| {
        Error::Other(
            "Der Ordner für die Einstellungen ist nicht bekannt (LOCALAPPDATA fehlt)".into(),
        )
    })?;
    // Eine vom Setup durchgereichte Zeile kann noch das BOM tragen
    let raw = game_dir.to_string_lossy();
    let cleaned = raw.trim_start_matches('\u{feff}').trim();
    let dir = steam::validate_game_dir(Path::new(cleaned)).map_err(Error::Other)?;
    plugin_install::remember_game_dir(paths, &dir)?;
    Ok(dir)
}

/// Text der Nachfrage, wenn das Spiel die DLL beim Entfernen festhält.
pub fn retry_prompt(reason: &str) -> String {
    format!(
        "{reason}.\n\nWiederholen: nach dem Beenden des Spiels noch einmal versuchen.\nAbbrechen: Das Plugin wird umbenannt, damit das Spiel es nicht mehr lädt, und beim nächsten Neustart von Windows gelöscht."
    )
}

/// `--uninstall-plugin`. Entfernt die DLL aus dem gemerkten und dem über Steam gefundenen
/// Spielordner. Wird das Spiel nicht gefunden, gibt es nichts zu tun.
///
/// Läuft das Spiel oder hält es die DLL fest, fragt `retry` mit dem Grund, ob es noch einmal
/// versucht werden soll (`true`). Sonst wird die DLL mit [`plugin_install::set_aside`]
/// umbenannt und zum Löschen beim Neustart vorgemerkt. So bleibt nach einer Deinstallation nie
/// eine DLL zurück, die das Spiel lädt, auch nicht bei einer stillen Deinstallation, bei der
/// `retry` immer `false` liefert.
///
/// Liefert die Ordner, aus denen eine DLL entfernt oder zur Seite gelegt wurde, mit dem
/// Ergebnis.
pub fn uninstall_plugin(
    env: &InstallEnv<'_>,
    retry: &mut dyn FnMut(&str) -> bool,
) -> Result<Vec<(PathBuf, Removal)>> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = env
        .remembered()
        .and_then(|d| steam::validate_game_dir(&d).ok())
    {
        dirs.push(d);
    }
    if let Ok(d) = steam::find_ets2(&env.registry)
        && !dirs.contains(&d)
    {
        dirs.push(d);
    }
    let mut removed = Vec::new();
    let mut errors = Vec::new();
    for dir in dirs {
        let result = loop {
            match plugin_install::uninstall(&dir, env.game) {
                Err(Error::PluginInUse(reason)) => {
                    if !retry(&retry_prompt(&reason)) {
                        break plugin_install::set_aside(&dir, env.delete_later);
                    }
                }
                other => break other,
            }
        };
        match result {
            Ok(Removal::NotThere) => {}
            Ok(r) => removed.push((dir, r)),
            Err(e) => errors.push(e.to_string()),
        }
    }
    if errors.is_empty() {
        Ok(removed)
    } else {
        Err(Error::Other(errors.join("\n")))
    }
}

/// Hinweis nach dem Entfernen, wenn eine DLL nur zur Seite gelegt werden konnte. `None`, wenn
/// alles gelöscht ist.
pub fn removal_note(removed: &[(PathBuf, Removal)]) -> Option<String> {
    let notes: Vec<String> = removed
        .iter()
        .filter_map(|(_, r)| match r {
            Removal::SetAside {
                path,
                delete_on_reboot: Ok(()),
            } => Some(format!(
                "Euro Truck Simulator 2 hat das Plugin noch geladen. Es ist umbenannt in {} und wird beim nächsten Start des Spiels nicht mehr geladen. Die Datei wird beim nächsten Neustart von Windows gelöscht",
                path.display()
            )),
            Removal::SetAside {
                path,
                delete_on_reboot: Err(e),
            } => Some(format!(
                "Euro Truck Simulator 2 hat das Plugin noch geladen. Es ist umbenannt in {} und wird beim nächsten Start des Spiels nicht mehr geladen. Die Datei bitte nach dem Beenden des Spiels von Hand löschen ({e})",
                path.display()
            )),
            _ => None,
        })
        .collect();
    (!notes.is_empty()).then(|| notes.join("\n\n"))
}
