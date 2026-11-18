//! Nur das Firmenprofil zählt.
//!
//! Ein Fahrer kann in ETS2 mehrere Profile haben, gebucht werden soll nur, was im Firmenprofil
//! passiert. Der Launcher meldet dafür den Namen des gewählten Firmenprofils
//! (`memberships.profile_name`), das Plugin schickt mit jedem Ereignis und Herzschlag das
//! aktive Profil mit.
//!
//! - Ist für den Fahrer ein Firmenprofil gemeldet, bucht der Server nur Ereignisse, die genau
//!   dieses Profil nennen. Alle anderen lehnt er mit 422 ab und legt sie in `rejected_events`
//!   ab, damit der Chef sie sieht. Herzschläge aus einem anderen Profil verwirft er.
//! - Ist keins gemeldet (ältere Launcher), bucht er wie bisher und merkt sich das Profil des
//!   Ereignisses nur in `ledger.profile`.
//!
//! Verglichen wird nach dem Trimmen genau, mit Groß- und Kleinschreibung, wie im Plugin.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// Längster Profilname in Zeichen.
pub const NAME_MAX: usize = 100;

/// Prüft einen Profilnamen vom Client. Leer oder fehlend wird zu `None`, getrimmt.
pub fn clean_name(name: Option<&str>) -> Result<Option<String>, String> {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
        return Err(format!(
            "Der Profilname darf höchstens {NAME_MAX} Zeichen ohne Steuerzeichen haben"
        ));
    }
    Ok(Some(name.to_string()))
}

/// Das gemeldete Firmenprofil eines Mitglieds, `None`, wenn keins gemeldet ist.
pub fn reported(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
) -> rusqlite::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT profile_name FROM memberships WHERE company_id = ?1 AND user_id = ?2",
            [company_id, user_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// Das Firmenprofil melden oder mit `None` löschen.
pub fn report(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    name: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE memberships SET profile_name = ?1 WHERE company_id = ?2 AND user_id = ?3",
        params![name, company_id, user_id],
    )?;
    Ok(())
}

/// Warum etwas nicht zum gemeldeten Firmenprofil passt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// Für das Protokoll und die Liste des Chefs
    pub reason: String,
    /// Für die Antwort an den Client
    pub message: String,
}

/// Passt `sent` zum gemeldeten Firmenprofil? `None` heißt: ja, oder es ist keins gemeldet.
pub fn mismatch(reported: Option<&str>, sent: Option<&str>) -> Option<Mismatch> {
    let company = reported?;
    match sent {
        Some(p) if p == company => None,
        Some(p) => Some(Mismatch {
            reason: format!("aus Profil '{p}', gemeldetes Firmenprofil ist '{company}'"),
            message: format!("Es kommt aus dem Profil '{p}', dein Firmenprofil ist '{company}'"),
        }),
        None => Some(Mismatch {
            reason: format!("ohne Profil, gemeldetes Firmenprofil ist '{company}'"),
            message: format!(
                "Es nennt kein Profil, dein Firmenprofil ist '{company}'. Bitte Plugin und Launcher aktualisieren"
            ),
        }),
    }
}

/// Ein abgelehntes Ereignis für den Chef ablegen. Je Fahrer und `event_id` nur einmal.
pub struct Rejected<'a> {
    pub company_id: i64,
    pub user_id: i64,
    pub event_id: &'a str,
    pub kind: &'a str,
    pub amount: i64,
    pub profile: Option<&'a str>,
    pub reason: &'a str,
    pub payload: &'a Value,
    pub now: i64,
}

pub fn record_rejected(conn: &Connection, r: &Rejected<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO rejected_events
             (company_id, user_id, event_id, type, amount, profile, reason, payload, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            r.company_id,
            r.user_id,
            r.event_id,
            r.kind,
            r.amount,
            r.profile,
            r.reason,
            r.payload.to_string(),
            r.now
        ],
    )?;
    Ok(())
}

/// Die abgelehnten Ereignisse einer Firma, neueste zuerst.
pub fn list_rejected(
    conn: &Connection,
    company_id: i64,
    limit: i64,
) -> rusqlite::Result<Vec<Value>> {
    conn.prepare(
        "SELECT r.id, r.created_at, r.user_id, u.name, r.event_id, r.type, r.amount, r.profile,
                r.reason, r.payload
         FROM rejected_events r JOIN users u ON u.id = r.user_id
         WHERE r.company_id = ?1 ORDER BY r.id DESC LIMIT ?2",
    )?
    .query_map([company_id, limit], |r| {
        let payload: String = r.get(9)?;
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "created_at": r.get::<_, i64>(1)?,
            "user_id": r.get::<_, i64>(2)?, "user": r.get::<_, String>(3)?,
            "event_id": r.get::<_, String>(4)?, "type": r.get::<_, String>(5)?,
            "amount": r.get::<_, i64>(6)?, "profile": r.get::<_, Option<String>>(7)?,
            "reason": r.get::<_, String>(8)?,
            "payload": serde_json::from_str::<Value>(&payload).unwrap_or(Value::Null)
        }))
    })?
    .collect()
}
