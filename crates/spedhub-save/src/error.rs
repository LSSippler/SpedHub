use crate::check::Problem;
use std::{fmt, path::PathBuf};

/// Alle Fehler der Bibliothek. Die Meldungen sind für Menschen gedacht und deutsch.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Lesen oder Schreiben einer Datei ist gescheitert.
    Io {
        path: PathBuf,
        message: String,
    },
    /// Die Datei ließ sich nicht in Text umwandeln (Format unbekannt, Daten kaputt).
    Decode(String),
    /// Der Text entspricht nicht dem Aufbau eines Spielstands.
    Parse {
        line: usize,
        message: String,
    },
    MissingUnit(String),
    MissingField {
        unit: String,
        field: String,
    },
    InvalidValue {
        unit: String,
        field: String,
        value: String,
    },
    /// Die Integritätsprüfung hat Probleme gefunden. Es wurde nichts geschrieben.
    Integrity(Vec<Problem>),
    TruckNotFound(String),
    AmbiguousPlate(String),
    /// Mehrere Lkw passen gleich gut zu einem Paket. Enthält ihre Kennzeichen.
    AmbiguousTruck(Vec<String>),
    PlateExists(String),
    /// In der Heimatgarage ist kein Stellplatz für den bisherigen Lkw frei. Enthält die Stadt.
    NoFreeSlot(String),
    /// Der aktive Lkw soll entfernt werden, aber es ist kein Ersatz angegeben.
    ReplacementRequired(String),
    NameCollision(String),
    InvalidPackage(String),
    InvalidInput(String),
    /// Der Spielstand ist in einem Zustand, den die Bibliothek bewusst nicht bearbeitet.
    Unsupported(String),
    /// Der Spielstand ist anders aufgebaut als erwartet.
    Structure(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn missing_field(unit: &str, field: &str) -> Self {
        Error::MissingField {
            unit: unit.into(),
            field: field.into(),
        }
    }

    pub(crate) fn invalid_value(unit: &str, field: &str, value: &str) -> Self {
        Error::InvalidValue {
            unit: unit.into(),
            field: field.into(),
            value: value.into(),
        }
    }

    pub(crate) fn io(path: impl Into<PathBuf>, e: &std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            message: e.to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Error::Decode(m) => write!(f, "Spielstand konnte nicht gelesen werden: {m}"),
            Error::Parse { line, message } => write!(f, "Zeile {line}: {message}"),
            Error::MissingUnit(u) => write!(f, "Unit {u} fehlt im Spielstand"),
            Error::MissingField { unit, field } => {
                write!(f, "Feld {field} fehlt in Unit {unit}")
            }
            Error::InvalidValue { unit, field, value } => {
                write!(f, "Ungültiger Wert in {unit}, Feld {field}: {value}")
            }
            Error::Integrity(problems) => {
                write!(
                    f,
                    "Integritätsprüfung fehlgeschlagen, es wurde nichts geschrieben"
                )?;
                for p in problems.iter().take(20) {
                    write!(f, "\n  {p}")?;
                }
                if problems.len() > 20 {
                    write!(f, "\n  und {} weitere", problems.len() - 20)?;
                }
                Ok(())
            }
            Error::TruckNotFound(p) => write!(f, "Kein Lkw mit dem Kennzeichen {p} gefunden"),
            Error::AmbiguousPlate(p) => write!(f, "Mehrere Lkw haben das Kennzeichen {p}"),
            Error::AmbiguousTruck(plates) => write!(
                f,
                "Mehrere Lkw im Spielstand passen gleich gut: {}. Welcher es ist, lässt sich nicht sicher sagen",
                plates.join(", ")
            ),
            Error::PlateExists(p) => {
                write!(
                    f,
                    "Ein Lkw mit dem Kennzeichen {p} steht schon im Spielstand"
                )
            }
            Error::NoFreeSlot(city) => write!(
                f,
                "In der Heimatgarage {city} ist kein Stellplatz frei. Der bisherige Lkw braucht einen freien Platz"
            ),
            Error::ReplacementRequired(p) => write!(
                f,
                "Der Lkw {p} ist der aktive Lkw. Zum Entfernen muss ein Ersatz angegeben sein"
            ),
            Error::NameCollision(n) => write!(f, "Den Unit-Namen {n} gibt es im Ziel schon"),
            Error::InvalidPackage(m) => write!(f, "Lkw-Paket ungültig: {m}"),
            Error::InvalidInput(m) => write!(f, "{m}"),
            Error::Unsupported(m) => write!(f, "Nicht unterstützt: {m}"),
            Error::Structure(m) => write!(f, "Unerwarteter Aufbau des Spielstands: {m}"),
        }
    }
}

impl std::error::Error for Error {}
