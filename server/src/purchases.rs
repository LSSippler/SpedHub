//! Anschaffungen für die Firma: Anschaffungsbudget, Lkw-Kauf verknüpfen.
//!
//! Kauft der Chef im Spiel einen Lkw, fließt das Geld über den Geldabgleich als Betriebskosten
//! aus der Firmenkasse. Damit er im Spiel überhaupt genug Geld hat, gibt er vorher ein
//! Anschaffungsbudget frei. Das bucht nichts: Es erhöht nur den Betrag, den der Geldabgleich in
//! seinen Spielstand schreibt.
//!
//! Restbetrag eines offenen Budgets:
//!
//! ```text
//! verbraucht = -(Summe betriebskosten + betriebskosten_korrektur des Chefs nach der Freigabe),
//!              begrenzt auf 0 bis Betrag
//! Rest       = Betrag - verbraucht
//! ```
//!
//! "Nach der Freigabe" heißt: Buchungen mit einer größeren id als die größte bei der Freigabe
//! (`ledger_from`). Beim Schließen merkt sich das Budget die größte id (`ledger_to`), danach
//! zählt nichts mehr dazu. Auch Tanken und Reparaturen des Chefs verbrauchen das Budget, und
//! zwar alle Betriebskosten, die ein Abgleich nach der Freigabe bucht, auch für Geld, das im
//! Spiel schon vor der Freigabe ausgegeben wurde.
//!
//! Spät gemeldete Ereignisse rechnen das Fenster eines früheren Abgleichs neu und buchen dafür
//! weitere `betriebskosten` oder eine Gutschrift `betriebskosten_korrektur` (siehe
//! [`crate::reconcile::correct_late`]). Diese Buchungen tragen den Abgleich in `sync_id` und
//! zählen nur, wenn der Abgleich nach der Freigabe lag (`sync_id` über `sync_from`). Eine
//! Gutschrift für einen Abgleich von vor der Freigabe gibt also kein Geld ins Budget zurück,
//! das nie davon abgezogen war. Das Budget wird dadurch nie zu groß, höchstens zu klein.
//!
//! Storniert der Chef eigene Betriebskosten (siehe [`crate::reversal`]), gibt das Storno das
//! Budget wieder frei, aber nur, wenn die stornierte Buchung es verbraucht hatte: nach der
//! Freigabe gebucht, ihr Abgleich nach der Freigabe, und das Storno kommt vor dem Schließen.
//! Die Firma hat das Geld dann zurück, im Spiel steht es dem Chef wieder zur Verfügung.
//! Betriebskosten, die ein Abgleich selbst bucht, tragen seit dem Storno ebenfalls ihren
//! Abgleich in `sync_id`. Weil der Abgleich nach der Freigabe kommt, ändert das nichts.
//!
//! Kein Doppelabzug: Der Abgleich schreibt Lohnkonto + Vorschuss + Rest. Gibt der Chef im Spiel
//! X aus, fehlen beim nächsten Abgleich X gegenüber dem geschriebenen Stand, die Firma bucht X
//! Betriebskosten (einmal), und der Rest sinkt um X. Was danach geschrieben wird, enthält nur
//! noch den neuen Rest. Freigabe und Schließen buchen nichts. Nach dem Schließen ist der
//! geschriebene Stand des letzten Abgleichs die Grundlage, das Spiel hat also genau so viel
//! wie erwartet, und erst der neu geschriebene Stand lässt den Rest weg.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// Ein Anschaffungsbudget mit dem, was davon übrig ist.
#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    pub id: i64,
    pub user_id: i64,
    pub amount: i64,
    pub note: String,
    pub created_at: i64,
    pub closed_at: Option<i64>,
    pub closed_by: Option<String>,
    /// Davon schon als Betriebskosten gebucht, 0 bis `amount`
    pub consumed: i64,
}

impl Budget {
    /// Was noch in den Spielstand kommt. Bei einem geschlossenen Budget, was beim Schließen
    /// übrig war und entfallen ist.
    pub fn rest(&self) -> i64 {
        self.amount - self.consumed
    }

    pub fn json(&self) -> Value {
        json!({
            "id": self.id, "user_id": self.user_id, "amount": self.amount, "note": self.note,
            "created_at": self.created_at, "closed_at": self.closed_at,
            "closed_by": self.closed_by, "consumed": self.consumed, "rest": self.rest()
        })
    }
}

/// Netto-Betriebskosten eines Nutzers in der Firmenkasse mit Buchungs-id in `(from, to]`,
/// positiv. Ohne `to` bis jetzt. Beträge mit `sync_id` zählen nur, wenn ihr Abgleich nach
/// `sync_from` liegt. Ein Storno von Betriebskosten zählt, wenn die stornierte Buchung selbst
/// gezählt hat, und gibt das Budget so wieder frei.
fn net_costs(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    (from, sync_from): (i64, i64),
    to: Option<i64>,
) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(-SUM(l.amount), 0) FROM ledger l
         LEFT JOIN ledger o ON o.id = l.reverses_id
         WHERE l.company_id = ?1 AND l.account = 'firma' AND l.user_id = ?2
           AND l.id > ?3 AND (?4 IS NULL OR l.id <= ?4)
           AND ((l.kind IN ('betriebskosten', 'betriebskosten_korrektur')
                 AND (l.sync_id IS NULL OR l.sync_id > ?5))
                OR (l.kind = 'storno'
                    AND o.kind IN ('betriebskosten', 'betriebskosten_korrektur')
                    AND o.id > ?3 AND (o.sync_id IS NULL OR o.sync_id > ?5)))",
        params![company_id, user_id, from, to, sync_from],
        |r| r.get(0),
    )
}

const SELECT: &str = "SELECT b.id, b.company_id, b.user_id, b.amount, b.note, b.created_at,
        b.ledger_from, b.closed_at, u.name, b.ledger_to, b.sync_from
     FROM purchase_budgets b LEFT JOIN users u ON u.id = b.closed_by";

fn from_row(conn: &Connection, r: &rusqlite::Row) -> rusqlite::Result<Budget> {
    let company_id: i64 = r.get(1)?;
    let user_id: i64 = r.get(2)?;
    let amount: i64 = r.get(3)?;
    let from: i64 = r.get(6)?;
    let to: Option<i64> = r.get(9)?;
    let sync_from: i64 = r.get(10)?;
    let consumed = net_costs(conn, company_id, user_id, (from, sync_from), to)?.clamp(0, amount);
    Ok(Budget {
        id: r.get(0)?,
        user_id,
        amount,
        note: r.get(4)?,
        created_at: r.get(5)?,
        closed_at: r.get(7)?,
        closed_by: r.get(8)?,
        consumed,
    })
}

/// Das offene Budget der Firma, falls es eins gibt.
pub fn open_budget(conn: &Connection, company_id: i64) -> rusqlite::Result<Option<Budget>> {
    conn.query_row(
        &format!("{SELECT} WHERE b.company_id = ?1 AND b.closed_at IS NULL"),
        [company_id],
        |r| from_row(conn, r),
    )
    .optional()
}

/// Ein Budget der Firma.
pub fn budget(conn: &Connection, company_id: i64, id: i64) -> rusqlite::Result<Option<Budget>> {
    conn.query_row(
        &format!("{SELECT} WHERE b.company_id = ?1 AND b.id = ?2"),
        [company_id, id],
        |r| from_row(conn, r),
    )
    .optional()
}

/// Alle Budgets der Firma, neueste zuerst.
pub fn budgets(conn: &Connection, company_id: i64) -> rusqlite::Result<Vec<Budget>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE b.company_id = ?1 ORDER BY b.id DESC LIMIT 200"
    ))?;
    stmt.query_map([company_id], |r| from_row(conn, r))?
        .collect()
}

/// Restbetrag aller offenen Budgets eines Nutzers. So viel schreibt der Geldabgleich
/// zusätzlich in seinen Spielstand.
pub fn open_rest_for(conn: &Connection, company_id: i64, user_id: i64) -> rusqlite::Result<i64> {
    Ok(open_budget(conn, company_id)?
        .filter(|b| b.user_id == user_id)
        .map_or(0, |b| b.rest()))
}

/// Größte Buchungs-id überhaupt. Grenze für "nach der Freigabe" und "bis zum Schließen".
pub fn max_ledger_id(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COALESCE(MAX(id), 0) FROM ledger", [], |r| r.get(0))
}

/// Größter Abgleich überhaupt. Grenze für spät gebuchte Betriebskosten "nach der Freigabe".
pub fn max_sync_id(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COALESCE(MAX(id), 0) FROM sync_points", [], |r| {
        r.get(0)
    })
}

/// Notiz einer Verkaufsbuchung.
pub fn sale_note(plate: &str) -> String {
    format!("Verkauf {plate}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_ist_betrag_minus_verbraucht() {
        let b = Budget {
            id: 1,
            user_id: 1,
            amount: 50_000,
            note: String::new(),
            created_at: 0,
            closed_at: None,
            closed_by: None,
            consumed: 40_000,
        };
        assert_eq!(b.rest(), 10_000);
        assert_eq!(b.json()["rest"], 10_000);
        assert_eq!(sale_note("SPED 004"), "Verkauf SPED 004");
    }
}
