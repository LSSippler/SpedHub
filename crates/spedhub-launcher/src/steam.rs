//! ETS2 über Steam finden.
//!
//! 1. Steam-Ordner aus der Registry: `HKCU\Software\Valve\Steam` Wert `SteamPath`, dann
//!    `HKLM\SOFTWARE\WOW6432Node\Valve\Steam` und `HKLM\SOFTWARE\Valve\Steam`, je Wert
//!    `InstallPath`.
//! 2. Alle Bibliotheken aus `steamapps\libraryfolders.vdf` (Valve KeyValues). Der Steam-Ordner
//!    selbst ist immer auch eine Bibliothek.
//! 3. ETS2 (App-ID 227300) liegt in der Bibliothek, deren `apps` die ID enthält, sonst in der
//!    ersten, in der `steamapps\appmanifest_227300.acf` liegt.
//! 4. Spielordner: `steamapps\common\<installdir>`, `installdir` aus dem App-Manifest, sonst
//!    `Euro Truck Simulator 2`. Darin muss `bin\win_x64` liegen.
//!
//! Alles hier arbeitet nur mit Dateien und übergebenen Werten, damit es sich ohne Windows und
//! ohne Steam testen lässt. Gelesen wird die Registry in [`crate::system`].

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

/// App-ID von Euro Truck Simulator 2 bei Steam.
pub const ETS2_APP_ID: &str = "227300";
/// Ordnername, wenn das App-Manifest keinen nennt.
pub const DEFAULT_INSTALL_DIR: &str = "Euro Truck Simulator 2";

// ------------------------------------------------------------------- KeyValues (VDF)

/// Ein Wert in einer VDF-Datei: Text oder ein Block mit Schlüsseln und Werten in Reihenfolge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Text(String),
    Block(Vec<(String, Vdf)>),
}

impl Vdf {
    /// Wert zu einem Schlüssel in einem Block, ohne Rücksicht auf Groß- und Kleinschreibung.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Block(items) => items
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Text(_) => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Vdf::Text(t) => Some(t),
            Vdf::Block(_) => None,
        }
    }

    pub fn items(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Block(items) => items,
            Vdf::Text(_) => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Result<Vec<(Token, usize)>, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    let mut line = 1;
    while let Some(c) = chars.next() {
        match c {
            '\n' => line += 1,
            c if c.is_whitespace() || c == '\u{feff}' => {}
            '{' => tokens.push((Token::Open, line)),
            '}' => tokens.push((Token::Close, line)),
            '/' if chars.peek() == Some(&'/') => {
                // Kommentar bis zum Zeilenende
                for c in chars.by_ref() {
                    if c == '\n' {
                        line += 1;
                        break;
                    }
                }
            }
            '"' => {
                let start = line;
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => {
                            return Err(format!(
                                "Zeile {start}: Anführungszeichen nicht geschlossen"
                            ));
                        }
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('\\') => s.push('\\'),
                            Some('"') => s.push('"'),
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            // Unbekannte Escapes bleiben, wie sie sind
                            Some(other) => {
                                if other == '\n' {
                                    line += 1;
                                }
                                s.push('\\');
                                s.push(other);
                            }
                            None => {
                                return Err(format!(
                                    "Zeile {start}: Anführungszeichen nicht geschlossen"
                                ));
                            }
                        },
                        Some(c) => {
                            if c == '\n' {
                                line += 1;
                            }
                            s.push(c);
                        }
                    }
                }
                tokens.push((Token::Text(s), start));
            }
            '[' => {
                // Bedingungen wie [$WIN32] gelten hier immer, sie werden übersprungen
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    if c == '\n' {
                        line += 1;
                    }
                }
            }
            _ => {
                // Schlüssel oder Wert ohne Anführungszeichen
                let mut s = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                tokens.push((Token::Text(s), line));
            }
        }
    }
    Ok(tokens)
}

type OpenBlock = (String, Vec<(String, Vdf)>, usize);

/// Liest eine VDF-Datei (Valve KeyValues als Text). Ergebnis ist der Block der obersten Ebene.
/// Eine kaputte Datei gibt einen Fehler mit Zeile, nie eine Panic.
pub fn parse_vdf(text: &str) -> Result<Vdf, String> {
    let tokens = tokenize(text)?;
    // Offene Blöcke: Schlüssel, bisherige Einträge, Zeile des Anfangs
    let mut stack: Vec<OpenBlock> = vec![(String::new(), Vec::new(), 0)];
    let mut i = 0;
    while let Some((token, line)) = tokens.get(i) {
        i += 1;
        match token {
            Token::Text(key) => match tokens.get(i) {
                Some((Token::Text(value), _)) => {
                    i += 1;
                    if let Some(top) = stack.last_mut() {
                        top.1.push((key.clone(), Vdf::Text(value.clone())));
                    }
                }
                Some((Token::Open, _)) => {
                    i += 1;
                    stack.push((key.clone(), Vec::new(), *line));
                }
                Some((Token::Close, l)) => {
                    return Err(format!("Zeile {l}: zu \"{key}\" fehlt der Wert"));
                }
                None => return Err(format!("Zeile {line}: zu \"{key}\" fehlt der Wert")),
            },
            Token::Close => {
                if stack.len() < 2 {
                    return Err(format!("Zeile {line}: Klammer zu viel"));
                }
                if let Some((key, items, _)) = stack.pop()
                    && let Some(parent) = stack.last_mut()
                {
                    parent.1.push((key, Vdf::Block(items)));
                }
            }
            Token::Open => return Err(format!("Zeile {line}: Klammer ohne Schlüssel")),
        }
    }
    if stack.len() > 1 {
        let (key, _, line) = stack.last().cloned().unwrap_or_default();
        return Err(format!(
            "Block \"{key}\" ab Zeile {line} ist nicht geschlossen, die Datei endet vorher"
        ));
    }
    let items = stack.pop().map(|(_, items, _)| items).unwrap_or_default();
    Ok(Vdf::Block(items))
}

// ------------------------------------------------------------------- Bibliotheken

/// Eine Steam-Bibliothek.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    pub path: PathBuf,
    /// App-IDs aus dem neuen Format. Leer im alten Format, dann zählt das App-Manifest.
    pub apps: Vec<String>,
}

/// Bibliotheken aus `libraryfolders.vdf`. Versteht das neue Format
/// (`"libraryfolders" { "0" { "path" "..." "apps" { "227300" "..." } } }`) und das alte
/// (`"LibraryFolders" { "1" "D:\\SteamLibrary" }`).
pub fn parse_library_folders(text: &str) -> Result<Vec<Library>, String> {
    let root = parse_vdf(text)?;
    let folders = root
        .get("libraryfolders")
        .filter(|v| matches!(v, Vdf::Block(_)))
        .ok_or_else(|| "Der Block \"libraryfolders\" fehlt".to_string())?;
    let mut out = Vec::new();
    for (key, value) in folders.items() {
        // Nur nummerierte Einträge sind Bibliotheken, daneben stehen etwa "contentstatsid"
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        match value {
            Vdf::Text(path) if !path.trim().is_empty() => out.push(Library {
                path: PathBuf::from(path),
                apps: Vec::new(),
            }),
            Vdf::Block(_) => {
                let Some(path) = value.get("path").and_then(Vdf::text) else {
                    continue;
                };
                let apps = value
                    .get("apps")
                    .map(|a| a.items().iter().map(|(k, _)| k.clone()).collect())
                    .unwrap_or_default();
                out.push(Library {
                    path: PathBuf::from(path),
                    apps,
                });
            }
            Vdf::Text(_) => {}
        }
    }
    Ok(out)
}

/// `installdir` aus einem App-Manifest (`"AppState" { "installdir" "..." }`).
pub fn parse_install_dir(text: &str) -> Option<String> {
    let root = parse_vdf(text).ok()?;
    let dir = root.get("AppState")?.get("installdir")?.text()?.trim();
    // Nur ein einfacher Ordnername, kein Pfad nach oben
    if dir.is_empty() || dir.contains("..") || dir.contains(['/', '\\', ':']) {
        return None;
    }
    Some(dir.to_string())
}

// ------------------------------------------------------------------- Registry und Suche

/// Was die Registry über Steam weiß. Unter Windows liest [`crate::system::steam_registry`]
/// die Werte, in Tests werden sie nachgebaut.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistryValues {
    /// `HKCU\Software\Valve\Steam`, Wert `SteamPath`. Steam schreibt ihn mit `/`.
    pub current_user_steam_path: Option<String>,
    /// `HKLM\SOFTWARE\WOW6432Node\Valve\Steam`, Wert `InstallPath`
    pub machine_wow64_install_path: Option<String>,
    /// `HKLM\SOFTWARE\Valve\Steam`, Wert `InstallPath`
    pub machine_install_path: Option<String>,
}

/// Für den Vergleich zweier Pfade: Groß- und Kleinschreibung und `/` gegen `\` egal, wie unter
/// Windows.
fn path_key(p: &Path) -> String {
    p.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

impl RegistryValues {
    /// Mögliche Steam-Ordner in dieser Reihenfolge, ohne Doppelte.
    pub fn steam_dirs(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for v in [
            &self.current_user_steam_path,
            &self.machine_wow64_install_path,
            &self.machine_install_path,
        ]
        .into_iter()
        .flatten()
        {
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            let p = PathBuf::from(v);
            if !out.iter().any(|o| path_key(o) == path_key(&p)) {
                out.push(p);
            }
        }
        out
    }
}

/// Warum ETS2 nicht gefunden wurde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindError {
    /// Kein Steam-Ordner in der Registry oder keiner davon existiert
    NoSteam,
    /// Steam ist da, aber ETS2 in keiner Bibliothek. `broken` nennt kaputte
    /// `libraryfolders.vdf` mit Grund.
    NotInstalled {
        libraries: Vec<PathBuf>,
        broken: Vec<String>,
    },
    /// Der Spielordner ist da, aber ohne `bin\win_x64`
    NoBinaries(PathBuf),
}

impl fmt::Display for FindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FindError::NoSteam => write!(
                f,
                "Euro Truck Simulator 2 wurde nicht gefunden: Steam ist auf diesem Rechner nicht eingetragen"
            ),
            FindError::NotInstalled { libraries, broken } => {
                write!(
                    f,
                    "Euro Truck Simulator 2 wurde in keiner Steam-Bibliothek gefunden"
                )?;
                if !libraries.is_empty() {
                    let list: Vec<String> =
                        libraries.iter().map(|l| l.display().to_string()).collect();
                    write!(f, " (gesucht in {})", list.join(", "))?;
                }
                for b in broken {
                    write!(f, ". {b}")?;
                }
                Ok(())
            }
            FindError::NoBinaries(dir) => write!(
                f,
                "Im Spielordner {} fehlt bin\\win_x64. Ist Euro Truck Simulator 2 ganz installiert?",
                dir.display()
            ),
        }
    }
}

impl std::error::Error for FindError {}

fn steamapps(library: &Path) -> PathBuf {
    library.join("steamapps")
}

/// Ordner mit den 64-Bit-Programmdateien des Spiels.
pub fn binaries_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("bin").join("win_x64")
}

/// Alle Bibliotheken einer Steam-Installation: der Steam-Ordner selbst und die aus
/// `libraryfolders.vdf`. Ist die Datei kaputt, steht der Grund im zweiten Teil.
pub fn libraries(steam_dir: &Path) -> (Vec<Library>, Option<String>) {
    let mut libs = vec![Library {
        path: steam_dir.to_path_buf(),
        apps: Vec::new(),
    }];
    let file = steamapps(steam_dir).join("libraryfolders.vdf");
    let mut problem = None;
    match fs::read(&file) {
        Ok(bytes) => match parse_library_folders(&String::from_utf8_lossy(&bytes)) {
            Ok(found) => {
                for lib in found {
                    match libs
                        .iter_mut()
                        .find(|l| path_key(&l.path) == path_key(&lib.path))
                    {
                        // Der Steam-Ordner steht im neuen Format als "0" mit seinen Apps drin
                        Some(existing) => existing.apps.extend(lib.apps),
                        None => libs.push(lib),
                    }
                }
            }
            Err(e) => problem = Some(format!("{} ist kaputt: {e}", file.display())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => problem = Some(format!("{} ließ sich nicht lesen: {e}", file.display())),
    }
    (libs, problem)
}

/// Spielordner in einer Bibliothek: `installdir` aus dem App-Manifest, sonst der Standardname.
pub fn game_dir_in(library: &Path) -> PathBuf {
    let manifest = steamapps(library).join(format!("appmanifest_{ETS2_APP_ID}.acf"));
    let install_dir = fs::read(&manifest)
        .ok()
        .and_then(|b| parse_install_dir(&String::from_utf8_lossy(&b)))
        .unwrap_or_else(|| DEFAULT_INSTALL_DIR.to_string());
    steamapps(library).join("common").join(install_dir)
}

fn has_manifest(library: &Path) -> bool {
    steamapps(library)
        .join(format!("appmanifest_{ETS2_APP_ID}.acf"))
        .is_file()
}

/// Sucht ETS2 in allen Bibliotheken aller Steam-Ordner. Liefert den Spielordner, in dem
/// `bin\win_x64` liegt.
pub fn find_ets2(registry: &RegistryValues) -> Result<PathBuf, FindError> {
    let steam_dirs: Vec<PathBuf> = registry
        .steam_dirs()
        .into_iter()
        .filter(|d| d.is_dir())
        .collect();
    if steam_dirs.is_empty() {
        return Err(FindError::NoSteam);
    }
    let mut all: Vec<Library> = Vec::new();
    let mut broken = Vec::new();
    for dir in &steam_dirs {
        let (libs, problem) = libraries(dir);
        broken.extend(problem);
        for lib in libs {
            if !all.iter().any(|l| path_key(&l.path) == path_key(&lib.path)) {
                all.push(lib);
            }
        }
    }
    // Zuerst Bibliotheken, die laut libraryfolders.vdf das Spiel haben, dann die mit Manifest
    let listed = all
        .iter()
        .filter(|l| l.apps.iter().any(|a| a == ETS2_APP_ID));
    let with_manifest = all
        .iter()
        .filter(|l| !l.apps.iter().any(|a| a == ETS2_APP_ID) && has_manifest(&l.path));
    let mut first_candidate = None;
    for lib in listed.chain(with_manifest) {
        let game = game_dir_in(&lib.path);
        if binaries_dir(&game).is_dir() {
            return Ok(game);
        }
        if first_candidate.is_none() && game.is_dir() {
            first_candidate = Some(game);
        }
    }
    match first_candidate {
        Some(game) => Err(FindError::NoBinaries(game)),
        None => Err(FindError::NotInstalled {
            libraries: all.into_iter().map(|l| l.path).collect(),
            broken,
        }),
    }
}

/// Prüft einen von Hand gewählten Ordner. Erlaubt sind der Spielordner selbst, `bin` und
/// `bin\win_x64` darin. Liefert den Spielordner.
pub fn validate_game_dir(dir: &Path) -> Result<PathBuf, String> {
    for candidate in dir.ancestors().take(3) {
        if binaries_dir(candidate).is_dir() {
            return Ok(candidate.to_path_buf());
        }
    }
    Err(format!(
        "In {} fehlt der Ordner bin\\win_x64. Bitte den Ordner von Euro Truck Simulator 2 wählen, meist steamapps\\common\\Euro Truck Simulator 2",
        dir.display()
    ))
}
