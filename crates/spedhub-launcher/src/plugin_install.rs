//! Das Spiel-Plugin installieren, ersetzen, entfernen und seinen Stand prüfen.
//!
//! Die DLL liegt nach dem Setup neben `spedhub-launcher.exe`. Installiert wird sie nach
//! `<Spielordner>\bin\win_x64\plugins\spedhub_plugin.dll`. Kopiert wird über eine temporäre
//! Datei und Umbenennen, damit nie eine halbe DLL im Spiel liegt.
//!
//! Beim Entfernen hat ein laufendes Spiel die DLL geladen, Windows lässt sie dann nicht
//! löschen. [`set_aside`] benennt sie deshalb um, sodass sie nicht mehr auf `.dll` endet und das
//! Spiel sie beim nächsten Start nicht mehr lädt, und merkt sie zum Löschen beim nächsten
//! Neustart von Windows vor. Übrig gebliebene Dateien räumen [`install`] und [`uninstall`] weg.
//!
//! Wo das Spiel liegt, entscheidet [`locate_game`]: ein ausdrücklich übergebener Ordner, sonst
//! der in den Einstellungen gemerkte, sonst die Suche über Steam ([`crate::steam`]).

use crate::error::{Error, Result};
use crate::settings::{AppPaths, Settings};
use crate::steam::{self, FindError, RegistryValues};
use crate::system::GameCheck;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Dateiname der Plugin-DLL, neben dem Launcher und im Spiel.
pub const PLUGIN_DLL: &str = "spedhub_plugin.dll";

const GAME_RUNNING: &str = "Euro Truck Simulator 2 läuft gerade und hat das Plugin geladen. Bitte das Spiel beenden, dann noch einmal versuchen";

/// Endung einer zur Seite gelegten DLL, siehe [`set_aside`].
const SET_ASIDE_SUFFIX: &str = ".entfernt";

/// `<Spielordner>\bin\win_x64\plugins`
pub fn plugins_dir(game_dir: &Path) -> PathBuf {
    steam::binaries_dir(game_dir).join("plugins")
}

/// Wo die DLL im Spiel liegt.
pub fn installed_dll(game_dir: &Path) -> PathBuf {
    plugins_dir(game_dir).join(PLUGIN_DLL)
}

/// Die mitgelieferte DLL neben `spedhub-launcher.exe`.
pub fn bundled_dll() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(PLUGIN_DLL))
}

/// SHA-256 einer Datei als Kleinbuchstaben-Hex.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(buf.get(..n).unwrap_or_default());
    }
    Ok(hex(&hasher.finalize()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Stand des Plugins im Spiel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginStatus {
    /// Dieselbe DLL wie die mitgelieferte (gleiche SHA-256)
    Installed,
    /// Eine DLL liegt im Spiel, aber eine andere als die mitgelieferte
    Outdated,
    /// Keine DLL im Spiel
    Missing,
    /// Der Spielordner ist nicht bekannt
    GameNotFound,
    /// Neben dem Launcher liegt keine DLL, zum Beispiel bei einem selbst gebauten Launcher.
    /// Dann lässt sich weder vergleichen noch installieren.
    NoBundledDll,
}

impl PluginStatus {
    /// Kurzer Satz für die Kachel "Plugin", passt in eine Zeile. Die ausführliche Meldung steht
    /// in der Hinweiskarte darüber.
    pub fn short_note(self) -> &'static str {
        match self {
            PluginStatus::Installed => "aktuell, im Spielordner",
            PluginStatus::Outdated => "neue Version installieren",
            PluginStatus::Missing => "noch nicht installiert",
            PluginStatus::GameNotFound => "Spiel nicht gefunden",
            PluginStatus::NoBundledDll => "nicht im Paket enthalten",
        }
    }

    /// Text für die Oberfläche.
    pub fn label(self) -> &'static str {
        match self {
            PluginStatus::Installed => "Plugin installiert",
            PluginStatus::Outdated => "Plugin veraltet",
            PluginStatus::Missing => "Plugin fehlt",
            PluginStatus::GameNotFound => "Spiel nicht gefunden",
            PluginStatus::NoBundledDll => "Keine Plugin-DLL neben dem Launcher",
        }
    }

    /// Lässt sich (neu) installieren?
    pub fn can_install(self) -> bool {
        matches!(self, PluginStatus::Outdated | PluginStatus::Missing)
    }
}

/// Vergleicht die DLL im Spiel mit der mitgelieferten.
pub fn status(game_dir: Option<&Path>, bundled: Option<&Path>) -> PluginStatus {
    let Some(game_dir) = game_dir else {
        return PluginStatus::GameNotFound;
    };
    let target = installed_dll(game_dir);
    if !target.is_file() {
        return PluginStatus::Missing;
    }
    let Some(bundled) = bundled.filter(|b| b.is_file()) else {
        return PluginStatus::NoBundledDll;
    };
    match (sha256_file(&target), sha256_file(bundled)) {
        (Ok(a), Ok(b)) if a == b => PluginStatus::Installed,
        _ => PluginStatus::Outdated,
    }
}

/// Ist der Fehler einer, bei dem eine geladene DLL im Weg ist?
fn is_locked(e: &io::Error) -> bool {
    // 32: Freigabeverletzung, 33: Sperrverletzung, 5: Zugriff verweigert
    e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

fn locked_error(path: &Path, e: &io::Error) -> Error {
    if is_locked(e) {
        Error::PluginInUse(format!(
            "{} lässt sich nicht ersetzen ({e}). Läuft Euro Truck Simulator 2? Bitte das Spiel beenden, dann noch einmal versuchen. Fehlen Adminrechte, den Launcher über \"Plugin installieren\" starten",
            path.display()
        ))
    } else {
        Error::io(path, e)
    }
}

fn refuse_if_running(game: &dyn GameCheck) -> Result<()> {
    // Lässt sich nicht prüfen, ob das Spiel läuft: trotzdem versuchen. Ist die DLL gesperrt,
    // scheitert das Umbenennen mit einer klaren Meldung.
    if game.is_running() == Ok(true) {
        return Err(Error::PluginInUse(GAME_RUNNING.into()));
    }
    Ok(())
}

/// Ergebnis einer Installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub target: PathBuf,
    /// Eine andere DLL lag schon da und wurde ersetzt
    pub replaced: bool,
    /// Es lag schon genau diese DLL da, es wurde nichts kopiert
    pub unchanged: bool,
}

/// Kopiert die mitgelieferte DLL ins Spiel. Eine ältere wird ersetzt, der Ordner `plugins`
/// angelegt, wenn er fehlt.
pub fn install(bundled: &Path, game_dir: &Path, game: &dyn GameCheck) -> Result<Installed> {
    if !bundled.is_file() {
        return Err(Error::Other(format!(
            "Die Plugin-DLL fehlt: {}. Bitte SpedHub neu installieren",
            bundled.display()
        )));
    }
    if !steam::binaries_dir(game_dir).is_dir() {
        return Err(Error::Other(format!(
            "{} ist kein Ordner von Euro Truck Simulator 2, bin\\win_x64 fehlt",
            game_dir.display()
        )));
    }
    remove_leftovers(game_dir);
    let target = installed_dll(game_dir);
    let existed = target.is_file();
    let source_hash = sha256_file(bundled).map_err(|e| Error::io(bundled, &e))?;
    if existed && sha256_file(&target).ok().as_deref() == Some(source_hash.as_str()) {
        return Ok(Installed {
            target,
            replaced: false,
            unchanged: true,
        });
    }
    refuse_if_running(game)?;
    let dir = plugins_dir(game_dir);
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, &e))?;
    let tmp = dir.join(format!("{PLUGIN_DLL}.neu"));
    let copy = fs::copy(bundled, &tmp).map_err(|e| locked_error(&tmp, &e));
    if let Err(e) = copy {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, &target) {
        let _ = fs::remove_file(&tmp);
        return Err(locked_error(&target, &e));
    }
    let written = sha256_file(&target).map_err(|e| Error::io(&target, &e))?;
    if written != source_hash {
        return Err(Error::Other(format!(
            "{} ist nach dem Kopieren nicht gleich der mitgelieferten DLL. Bitte noch einmal installieren",
            target.display()
        )));
    }
    Ok(Installed {
        target,
        replaced: existed,
        unchanged: false,
    })
}

/// Ergebnis von [`uninstall`] und [`set_aside`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    /// Im Spiel lag keine DLL
    NotThere,
    Deleted,
    /// Die DLL ließ sich nicht löschen und ist umbenannt. Das Spiel lädt sie beim nächsten Start
    /// nicht mehr.
    SetAside {
        /// Neuer Name der Datei
        path: PathBuf,
        /// Ob sie zum Löschen beim nächsten Neustart von Windows vorgemerkt ist, sonst warum nicht
        delete_on_reboot: std::result::Result<(), String>,
    },
}

/// Entfernt Reste: eine halb kopierte DLL und zur Seite gelegte DLLs, die das Spiel inzwischen
/// nicht mehr festhält. Was sich nicht löschen lässt, bleibt liegen.
pub fn remove_leftovers(game_dir: &Path) {
    let dir = plugins_dir(game_dir);
    let _ = fs::remove_file(dir.join(format!("{PLUGIN_DLL}.neu")));
    let prefix = format!("{PLUGIN_DLL}.");
    for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(SET_ASIDE_SUFFIX) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Entfernt die DLL aus dem Spiel. Liegt keine da, ist das kein Fehler ([`Removal::NotThere`]).
/// Läuft das Spiel oder hält es die DLL fest, gibt es [`Error::PluginInUse`]. Dann lässt sich
/// nach dem Beenden des Spiels noch einmal versuchen oder die DLL mit [`set_aside`] unschädlich
/// machen.
pub fn uninstall(game_dir: &Path, game: &dyn GameCheck) -> Result<Removal> {
    let target = installed_dll(game_dir);
    remove_leftovers(game_dir);
    if !target.exists() {
        return Ok(Removal::NotThere);
    }
    refuse_if_running(game)?;
    match fs::remove_file(&target) {
        Ok(()) => Ok(Removal::Deleted),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Removal::NotThere),
        Err(e) => Err(locked_error(&target, &e)),
    }
}

/// Für den Fall, dass das Spiel die DLL geladen hat: Umbenennen geht unter Windows auch dann,
/// Löschen nicht. Die DLL heißt danach `spedhub_plugin.dll.<n>.entfernt`, endet also nicht mehr
/// auf `.dll`, und das Spiel lädt sie beim nächsten Start nicht mehr. `delete_later` merkt sie
/// zum Löschen beim nächsten Neustart von Windows vor ([`crate::system::delete_on_reboot`]).
/// Klappt das nicht, räumt die nächste Installation oder Deinstallation sie weg.
pub fn set_aside(
    game_dir: &Path,
    delete_later: &dyn Fn(&Path) -> std::result::Result<(), String>,
) -> Result<Removal> {
    let target = installed_dll(game_dir);
    if !target.exists() {
        return Ok(Removal::NotThere);
    }
    let dir = plugins_dir(game_dir);
    // Eine früher zur Seite gelegte DLL kann noch festgehalten sein, also ein freier Name
    let aside = (1..1000)
        .map(|n| dir.join(format!("{PLUGIN_DLL}.{n}{SET_ASIDE_SUFFIX}")))
        .find(|p| !p.exists())
        .ok_or_else(|| {
            Error::Other(format!(
                "In {} liegen zu viele alte Plugin-Dateien. Bitte das Spiel beenden und sie von Hand löschen",
                dir.display()
            ))
        })?;
    fs::rename(&target, &aside).map_err(|e| {
        Error::Other(format!(
            "{} ließ sich weder löschen noch umbenennen ({e}). Bitte das Spiel beenden und die Datei von Hand löschen",
            target.display()
        ))
    })?;
    let delete_on_reboot = delete_later(&aside);
    Ok(Removal::SetAside {
        path: aside,
        delete_on_reboot,
    })
}

/// Wo das Spiel liegt und woher das bekannt ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameSource {
    /// Ausdrücklich übergeben (`--game-dir`)
    Argument,
    /// In den Einstellungen gemerkt
    Settings,
    /// Über Steam gefunden
    Steam,
}

/// Sucht den Spielordner: `explicit`, dann `remembered` aus den Einstellungen, dann Steam.
/// Ein übergebener Ordner, der nicht passt, ist ein Fehler, es wird dann nicht woanders
/// gesucht. Ein gemerkter, der nicht mehr passt, wird übergangen.
pub fn locate_game(
    explicit: Option<&Path>,
    remembered: Option<&Path>,
    registry: &RegistryValues,
) -> std::result::Result<(PathBuf, GameSource), String> {
    if let Some(dir) = explicit {
        return steam::validate_game_dir(dir).map(|d| (d, GameSource::Argument));
    }
    if let Some(dir) = remembered
        && let Ok(d) = steam::validate_game_dir(dir)
    {
        return Ok((d, GameSource::Settings));
    }
    steam::find_ets2(registry)
        .map(|d| (d, GameSource::Steam))
        .map_err(|e: FindError| e.to_string())
}

/// Merkt sich den Spielordner in `launcher.json`. Liest die Datei dafür frisch, damit nichts
/// anderes verloren geht.
pub fn remember_game_dir(paths: &AppPaths, game_dir: &Path) -> Result<()> {
    let mut settings = Settings::load(paths);
    if settings.game_dir.as_deref() == Some(game_dir) {
        return Ok(());
    }
    settings.game_dir = Some(game_dir.to_path_buf());
    settings.save(paths)
}
