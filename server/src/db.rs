use rusqlite::Connection;
use std::sync::{Arc, Mutex};

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
-- Frühere Kennzeichen eines Lkw. Das Spiel kann das Kennzeichen ersetzen, der Launcher meldet
-- das neue bei der Rückgabe. Über die alten lässt sich Telemetrie noch zuordnen.
CREATE TABLE IF NOT EXISTS truck_plates (
    truck_id     INTEGER NOT NULL REFERENCES trucks(id),
    plate        TEXT NOT NULL,
    replaced_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS truck_plates_truck ON truck_plates(truck_id);
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
-- Lohnstufen einer Firma. Ein Fahrer hat die höchste Stufe (größte position), deren beide
-- Bedingungen er erfüllt: Level im Spiel und Firmenkilometer.
CREATE TABLE IF NOT EXISTS wage_levels (
    company_id    INTEGER NOT NULL REFERENCES companies(id),
    position      INTEGER NOT NULL,
    min_level     INTEGER NOT NULL,
    min_km        INTEGER NOT NULL,
    cents_per_km  INTEGER NOT NULL,
    PRIMARY KEY (company_id, position)
);
-- Eine Lohnabrechnung. kind = 'taeglich' läuft je Firma und Tag genau einmal, 'hand' löst der
-- Chef aus. Sie rechnet die Aufträge mit created_at in [period_from, period_to) ab.
CREATE TABLE IF NOT EXISTS payroll_runs (
    id           INTEGER PRIMARY KEY,
    company_id   INTEGER NOT NULL REFERENCES companies(id),
    day          INTEGER NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('taeglich', 'hand')),
    period_from  INTEGER NOT NULL,
    period_to    INTEGER NOT NULL,
    created_at   INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS payroll_once ON payroll_runs(company_id, day) WHERE kind = 'taeglich';
-- Eine Zeile je Fahrer und Abrechnung, damit das Kassenbuch zeigen kann, wie der Lohn zustande kam.
CREATE TABLE IF NOT EXISTS payroll_lines (
    payroll_id    INTEGER NOT NULL REFERENCES payroll_runs(id),
    user_id       INTEGER NOT NULL REFERENCES users(id),
    km            INTEGER NOT NULL,
    stage         INTEGER,
    cents_per_km  INTEGER NOT NULL,
    flat_days     INTEGER NOT NULL,
    flat          INTEGER NOT NULL,
    wage          INTEGER NOT NULL,
    claim_before  INTEGER NOT NULL,
    paid          INTEGER NOT NULL,
    claim_after   INTEGER NOT NULL,
    PRIMARY KEY (payroll_id, user_id)
);
-- Tage, für die ein Fahrer die Tagespauschale schon bekommen hat. Verhindert, dass eine Abrechnung
-- von Hand und die tägliche denselben Tag doppelt bezahlen.
CREATE TABLE IF NOT EXISTS flat_days (
    company_id  INTEGER NOT NULL REFERENCES companies(id),
    user_id     INTEGER NOT NULL REFERENCES users(id),
    day         INTEGER NOT NULL,
    payroll_id  INTEGER NOT NULL REFERENCES payroll_runs(id),
    PRIMARY KEY (company_id, user_id, day)
);
-- Offene Lohnforderungen, ebenfalls als Journal: positiv entsteht eine Forderung, negativ wird sie
-- bedient. Die offene Forderung eines Fahrers ist die Summe.
CREATE TABLE IF NOT EXISTS wage_claims (
    id          INTEGER PRIMARY KEY,
    company_id  INTEGER NOT NULL REFERENCES companies(id),
    user_id     INTEGER NOT NULL REFERENCES users(id),
    amount      INTEGER NOT NULL,
    payroll_id  INTEGER NOT NULL REFERENCES payroll_runs(id),
    created_at  INTEGER NOT NULL
);
-- Abgleichpunkte: was der Launcher im Spielstand vorgefunden hat und was er schreiben sollte.
-- Grundlage für den nächsten Abgleich ist write_money, wenn written = 1, sonst ingame_money.
-- last_ledger_id: größte Buchung beim Abgleich. Ereignisse mit Zeitpunkt wählt seit November
-- 2026 der Zeitpunkt (siehe COLUMNS), Ereignisse ohne zählen, wenn sie danach ankamen.
CREATE TABLE IF NOT EXISTS sync_points (
    id              INTEGER PRIMARY KEY,
    company_id      INTEGER NOT NULL REFERENCES companies(id),
    user_id         INTEGER NOT NULL REFERENCES users(id),
    ingame_money    INTEGER NOT NULL,
    xp              INTEGER,
    expected        INTEGER,
    write_money     INTEGER NOT NULL,
    last_ledger_id  INTEGER NOT NULL,
    written         INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sync_points_user ON sync_points(company_id, user_id);
-- Mehr Geld im Spiel, als die gemeldeten Ereignisse erklären. Wird nicht gutgeschrieben, der Chef
-- sieht es und schließt es.
CREATE TABLE IF NOT EXISTS unexplained (
    id          INTEGER PRIMARY KEY,
    company_id  INTEGER NOT NULL REFERENCES companies(id),
    user_id     INTEGER NOT NULL REFERENCES users(id),
    amount      INTEGER NOT NULL,
    sync_id     INTEGER NOT NULL REFERENCES sync_points(id),
    created_at  INTEGER NOT NULL,
    closed_at   INTEGER,
    closed_by   INTEGER REFERENCES users(id),
    close_note  TEXT NOT NULL DEFAULT ''
);
-- Wer gerade fährt: der letzte Herzschlag je Fahrer und Firma. Das ist Zustand, kein Journal,
-- jede Meldung überschreibt die vorige. `data` hält alle gemeldeten Werte als JSON. `truck_id`
-- ist der Firmen-Lkw, zu dem das Kennzeichen passt, `own_lease` = 1, wenn der Fahrer ihn
-- gerade geliehen hat.
CREATE TABLE IF NOT EXISTS presence (
    company_id  INTEGER NOT NULL REFERENCES companies(id),
    user_id     INTEGER NOT NULL REFERENCES users(id),
    last_seen   INTEGER NOT NULL,
    plate       TEXT NOT NULL,
    data        TEXT NOT NULL DEFAULT '{}',
    truck_id    INTEGER REFERENCES trucks(id),
    own_lease   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (company_id, user_id)
);
"#;

/// Spalten, die nach der ersten Fassung dazukamen. `CREATE TABLE IF NOT EXISTS` legt sie in einer
/// vorhandenen Datenbank nicht an, deshalb hier per `ALTER TABLE`. Vorhandene Zeilen behalten
/// ihre Werte, neue Spalten sind dort NULL oder der Standardwert.
const COLUMNS: &[(&str, &str, &str)] = &[
    // Erlösfaktor: gemeldeter Erlös mal Faktor geht in die Kasse
    ("companies", "revenue_factor", "REAL NOT NULL DEFAULT 1"),
    // Tagespauschale in Euro
    ("companies", "daily_flat", "INTEGER NOT NULL DEFAULT 50"),
    // Uhrzeit der täglichen Abrechnung in Minuten nach Mitternacht (Europe/Berlin)
    (
        "companies",
        "payroll_minute",
        "INTEGER NOT NULL DEFAULT 240",
    ),
    // Betriebsvorschuss in Euro
    ("companies", "advance", "INTEGER NOT NULL DEFAULT 2000"),
    // Ab hier wird abgerechnet. Davor galt der Lohnanteil.
    ("companies", "payroll_from", "INTEGER"),
    // Letzte Benutzung eines Tokens. Nach 30 Tagen ohne Benutzung läuft er ab.
    ("tokens", "last_used", "INTEGER"),
    // Letzter bekannter Zustand eines verliehenen Lkw aus dem Herzschlag des Ausleihers:
    // wann, Kilometerstand und Zustand (JSON wie `state`). Wird bei jeder Rückgabe geleert.
    ("trucks", "last_seen_at", "INTEGER"),
    ("trucks", "last_odometer_km", "INTEGER"),
    ("trucks", "last_state", "TEXT"),
    // Voller Betrag aus dem Spiel mit Vorzeichen, wie er den Kontostand im Spiel verändert hat
    ("ledger", "game_amount", "INTEGER"),
    // Gefahrene Kilometer eines abgelieferten Auftrags
    ("ledger", "distance_km", "INTEGER"),
    // Gehört zu dieser Lohnabrechnung
    ("ledger", "payroll_id", "INTEGER"),
    // Wann das Ereignis im Spiel passiert ist (Uhr des Fahrer-PCs, Unix-Sekunden). Leer, wenn
    // der Client keinen plausiblen Zeitpunkt geschickt hat, und bei alten Buchungen. Dann
    // zählt das Ereignis im ersten Abgleich, der nach ihm ankommt (siehe last_ledger_id).
    ("ledger", "occurred_at", "INTEGER"),
    // Fracht, Start- und Zielstadt eines abgelieferten Auftrags, wie das Plugin sie aus der
    // Job-Konfiguration des SDK kennt. Leer bei allen anderen und bei alten Buchungen.
    ("ledger", "cargo", "TEXT"),
    ("ledger", "source_city", "TEXT"),
    ("ledger", "target_city", "TEXT"),
    // Abgleich: Zeitpunkt des gelesenen Spielstands. Der Kontostand darin enthält die
    // Ereignisse bis hier.
    ("sync_points", "cutoff", "INTEGER"),
    // Wann der Launcher den neuen Spielstand geschrieben hat, gesetzt mit der Bestätigung
    ("sync_points", "written_at", "INTEGER"),
    // Kontostand im Spiel minus erwarteter Stand, mit spät gemeldeten Ereignissen korrigiert
    ("sync_points", "diff", "INTEGER"),
    // Für dieses Fenster gebuchte Betriebskosten (positiv), abzüglich Korrekturen
    ("sync_points", "costs_booked", "INTEGER NOT NULL DEFAULT 0"),
    // Diesem Fenster zugerechneter ungeklärter Betrag
    (
        "sync_points",
        "unexplained_attributed",
        "INTEGER NOT NULL DEFAULT 0",
    ),
];

/// Fünf Lohnstufen von 40 bis 80 Cent je Kilometer: (Level im Spiel, Firmenkilometer, Cent je km).
pub const DEFAULT_WAGE_LEVELS: [(i64, i64, i64); 5] = [
    (0, 0, 40),
    (5, 5_000, 50),
    (10, 20_000, 60),
    (20, 50_000, 70),
    (30, 100_000, 80),
];

fn migrate(conn: &Connection, now: i64) -> rusqlite::Result<()> {
    for (table, column, decl) in COLUMNS {
        let exists: bool = conn
            .prepare(&format!(
                "SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"
            ))?
            .exists([column])?;
        if !exists {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
        }
    }
    // Abgleichpunkte von vor den Abgleichfenstern: Zeitpunkt ist der Empfang beim Server,
    // Differenz und Aufteilung so, wie der Abgleich damals gebucht hat
    conn.execute_batch(
        "UPDATE sync_points SET cutoff = created_at WHERE cutoff IS NULL;
         UPDATE sync_points SET diff = ingame_money - expected,
                                costs_booked = MAX(0, expected - ingame_money),
                                unexplained_attributed = MAX(0, ingame_money - expected)
         WHERE diff IS NULL AND expected IS NOT NULL;",
    )?;
    // Tokens von vor dem Ablauf gelten ab jetzt als benutzt
    conn.execute(
        "UPDATE tokens SET last_used = ?1 WHERE last_used IS NULL",
        [now],
    )?;
    // Firmen aus der Zeit des Lohnanteils werden ab jetzt abgerechnet, nicht rückwirkend
    conn.execute(
        "UPDATE companies SET payroll_from = ?1 WHERE payroll_from IS NULL",
        [now],
    )?;
    let ids: Vec<i64> = conn
        .prepare(
            "SELECT id FROM companies c
             WHERE NOT EXISTS (SELECT 1 FROM wage_levels w WHERE w.company_id = c.id)",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for id in ids {
        insert_default_wage_levels(conn, id)?;
    }
    Ok(())
}

pub fn insert_default_wage_levels(conn: &Connection, company_id: i64) -> rusqlite::Result<()> {
    for (i, (level, km, cents)) in DEFAULT_WAGE_LEVELS.iter().enumerate() {
        conn.execute(
            "INSERT INTO wage_levels (company_id, position, min_level, min_km, cents_per_km)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![company_id, i as i64 + 1, level, km, cents],
        )?;
    }
    Ok(())
}

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
    migrate(&conn, now())?;
    Ok(Arc::new(Mutex::new(conn)))
}

pub fn now() -> i64 {
    crate::clock::system_now()
}
