use std::{fmt, path::PathBuf};

/// Fehler des Launchers. Die Meldungen sind deutsch und für die Oberfläche gedacht.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Der Server hat mit einem Fehler geantwortet. `message` ist seine Meldung im Klartext.
    Server {
        status: u16,
        message: String,
    },
    /// Der Server ist nicht erreichbar oder die Antwort ließ sich nicht lesen.
    Network(String),
    /// Fehler aus der Save-Bibliothek
    Save(spedhub_save::Error),
    Io {
        path: PathBuf,
        message: String,
    },
    /// Das Spiel läuft, in den Spielstand wird nicht geschrieben.
    GameRunning,
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, e: &std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            message: e.to_string(),
        }
    }

    /// Ist die Anmeldung abgelaufen oder ungültig?
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Error::Server { status: 401, .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Server { message, .. } => write!(f, "Server: {message}"),
            Error::Network(m) => write!(f, "Server nicht erreichbar: {m}"),
            Error::Save(e) => write!(f, "Spielstand: {e}"),
            Error::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Error::GameRunning => write!(
                f,
                "Euro Truck Simulator 2 läuft gerade. Bitte das Spiel beenden, dann noch einmal versuchen. In den Spielstand wurde nichts geschrieben"
            ),
            Error::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<spedhub_save::Error> for Error {
    fn from(e: spedhub_save::Error) -> Self {
        Error::Save(e)
    }
}
