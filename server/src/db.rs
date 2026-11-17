use rusqlite::Connection;
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

/// Eine Verbindung hinter einem Mutex reicht für eine Firma mit einer Handvoll Fahrern.
pub type Db = Arc<Mutex<Connection>>;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    pw_hash     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS tokens (
    token_hash  TEXT PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id),
    created_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS companies (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    invite_code   TEXT NOT NULL UNIQUE,
    wage_percent  INTEGER NOT NULL DEFAULT 30 CHECK (wage_percent BETWEEN 0 AND 100),
    created_at    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS memberships (
    company_id  INTEGER NOT NULL REFERENCES companies(id),
    user_id     INTEGER NOT NULL REFERENCES users(id),
    role        TEXT NOT NULL CHECK (role IN ('chef', 'fahrer')),
    joined_at   INTEGER NOT NULL,
    PRIMARY KEY (company_id, user_id)
);
-- Ein Firmen-Lkw. `package` ist der Bauplan aus dem Spielstand (Fahrzeug und Zubehör als Text),
-- `state` der Zustand als JSON (Verschleiß, Tank). `rev` steigt bei jeder Rückgabe.
CREATE TABLE IF NOT EXISTS trucks (
    id           INTEGER PRIMARY KEY,
    company_id   INTEGER NOT NULL REFERENCES companies(id),
    plate        TEXT NOT NULL,
    model        TEXT NOT NULL,
    package      TEXT NOT NULL DEFAULT '',
    odometer_km  INTEGER NOT NULL DEFAULT 0,
    state        TEXT NOT NULL DEFAULT '{}',
    rev          INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL,
    UNIQUE (company_id, plate)
);
CREATE TABLE IF NOT EXISTS leases (
    id           INTEGER PRIMARY KEY,
    truck_id     INTEGER NOT NULL REFERENCES trucks(id),
    user_id      INTEGER NOT NULL REFERENCES users(id),
    rev_out      INTEGER NOT NULL,
    taken_at     INTEGER NOT NULL,
    returned_at  INTEGER
);
-- Ein Lkw kann nur einmal gleichzeitig verliehen sein.
CREATE UNIQUE INDEX IF NOT EXISTS one_open_lease ON leases(truck_id) WHERE returned_at IS NULL;
-- Die Kasse ist ein Journal. Der Stand ist immer die Summe der Buchungen.
-- account = 'firma' ist die Firmenkasse, 'fahrer' das Lohnkonto von user_id.
CREATE TABLE IF NOT EXISTS ledger (
    id          INTEGER PRIMARY KEY,
    company_id  INTEGER NOT NULL REFERENCES companies(id),
    account     TEXT NOT NULL CHECK (account IN ('firma', 'fahrer')),
    user_id     INTEGER REFERENCES users(id),
    kind        TEXT NOT NULL,
    amount      INTEGER NOT NULL,
    event_id    TEXT,
    note        TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL
);
-- Jedes Ereignis aus dem Spiel wird nur einmal gebucht, auch wenn der Client es doppelt schickt.
CREATE UNIQUE INDEX IF NOT EXISTS ledger_event ON ledger(company_id, event_id, account) WHERE event_id IS NOT NULL;
"#;

pub fn open(path: &str) -> rusqlite::Result<Db> {
    let conn = if path == ":memory:" {
        Connection::open_in_memory()?
    } else {
        let c = Connection::open(path)?;
        c.pragma_update(None, "journal_mode", "WAL")?;
        c
    };
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(SCHEMA)?;
    Ok(Arc::new(Mutex::new(conn)))
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
