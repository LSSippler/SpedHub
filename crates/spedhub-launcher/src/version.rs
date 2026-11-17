//! Versionsnummern lesen und vergleichen.
//!
//! Alle Teile von SpedHub haben dieselbe Version aus der `Cargo.toml` im Wurzelordner. Server
//! und Launcher passen zusammen, wenn Haupt- und Nebenversion gleich sind: `0.2.0` mit
//! `0.2.7`, aber nicht mit `0.3.0`. Für Updates zählt die ganze Version, `0.2.1` ist neuer als
//! `0.2.0`.

use std::{cmp::Ordering, fmt};

/// Version dieses Launchers.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Header, in dem der Launcher seine Version an den Server schickt.
pub const HEADER: &str = "X-SpedHub-Version";

/// Eine Version wie `0.2.0` oder `0.3.0-rc1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Vorabversion hinter `-`, etwa `rc1`. Sie ist älter als dieselbe Version ohne Zusatz.
    pub pre: Option<String>,
}

impl Version {
    /// Liest `0.2.0`, `v0.2.0`, `0.3.0-rc1` und `0.2.0+build`. `None` bei allem anderen.
    pub fn parse(text: &str) -> Option<Version> {
        let t = text.trim();
        let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
        // Build-Angaben hinter + zählen nicht
        let t = t.split('+').next()?;
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            Some(_) => return None,
            None => (t, None),
        };
        let mut parts = core.split('.');
        let mut number = || -> Option<u64> {
            let p = parts.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()
        };
        let (major, minor, patch) = (number()?, number()?, number()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// Die Version dieses Launchers.
    pub fn current() -> Version {
        Version::parse(VERSION).expect("CARGO_PKG_VERSION ist eine gültige Version")
    }

    /// Passen die beiden zusammen, gleiche Haupt- und Nebenversion?
    pub fn compatible_with(&self, other: &Version) -> bool {
        self.major == other.major && self.minor == other.minor
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

/// Wie steht eine Version zu einer anderen?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Newer,
    Same,
    Older,
}

/// Ist `candidate` neuer, gleich oder älter als `current`? `None`, wenn sich eine der beiden
/// nicht lesen lässt.
pub fn compare(candidate: &str, current: &str) -> Option<Relation> {
    let (a, b) = (Version::parse(candidate)?, Version::parse(current)?);
    Some(match a.cmp(&b) {
        Ordering::Greater => Relation::Newer,
        Ordering::Equal => Relation::Same,
        Ordering::Less => Relation::Older,
    })
}

/// Passen zwei Versionen zusammen? Eine Version, die sich nicht lesen lässt, passt nie.
pub fn compatible(a: &str, b: &str) -> bool {
    match (Version::parse(a), Version::parse(b)) {
        (Some(x), Some(y)) => x.compatible_with(&y),
        _ => false,
    }
}

/// Warnung für die Oberfläche, wenn Launcher und Server nicht zusammenpassen. `None`, wenn
/// alles passt. `server` ist `None`, wenn der Server keine Version meldet (älter als 0.2).
pub fn mismatch_warning(launcher: &str, server: Option<&str>) -> Option<String> {
    match server {
        Some(s) if compatible(launcher, s) => None,
        Some(s) => Some(format!(
            "Der Server hat Version {s}, dieser Launcher {launcher}. Die Versionen passen nicht zusammen, manches kann fehlschlagen. Bitte Launcher und Server auf dieselbe Version bringen"
        )),
        None => Some(format!(
            "Der Server meldet keine Version, er ist älter als dieser Launcher ({launcher}). Manches kann fehlschlagen. Bitte den Server aktualisieren"
        )),
    }
}
