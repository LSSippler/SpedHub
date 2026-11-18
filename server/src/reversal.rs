//! Storno: Der Chef nimmt falsche Buchungen zurück, ohne das Journal zu ändern.
//!
//! Ein Storno ist eine neue Buchung `storno` mit umgekehrtem Betrag, auf demselben Konto und mit
//! derselben `user_id` wie das Original, dazu `reverses_id` (das Original), `booked_by` (der
//! Chef) und als Notiz der Grund. Das Original bleibt, wie es ist. Kasse und Lohnkonto sind
//! Summen, ein Storno wirkt dort also von selbst.
//!
//! Regeln:
//!
//! - Jede Buchung lässt sich je Konto nur einmal stornieren (eindeutiger Index auf
//!   `reverses_id` und `account`), ein Storno selbst gar nicht.
//! - `betriebskosten_korrektur` lässt sich nicht stornieren: Korrekturen rechnet der Server aus
//!   dem Geldabgleich. Storniert wird das Ereignis oder die Betriebskosten dahinter.
//! - Die Summe einer Lohnabrechnung in der Kasse lässt sich nicht stornieren, sie gehört zu
//!   mehreren Fahrern. Storniert wird die Lohnbuchung eines Fahrers. Dazu kommt eine
//!   Gegenbuchung `storno` in der Kasse mit demselben Verweis, das Geld geht also an die Kasse
//!   zurück, statt zu verschwinden.
//! - Überweisung und Haftung stehen als Paar auf beiden Konten. Wer eine Seite storniert,
//!   storniert die andere mit, sonst entstünde Geld aus dem Nichts oder ginge verloren.
//! - Alles oder nichts: Ist eine Buchung unbekannt, aus einer anderen Firma, schon storniert
//!   oder nicht stornierbar, wird nichts gebucht, und die Meldung nennt die Buchung.
//!
//! Wirkung auf den Geldabgleich (siehe [`crate::reconcile`]):
//!
//! - Ein storniertes Ereignis im offenen Fenster zählt beim nächsten Abgleich nicht mehr.
//! - Liegt es in einem schon abgeglichenen Fenster, hat es dort mitgezählt, obwohl es nicht
//!   hätte zählen dürfen. Das Fenster wird mit [`reconcile::rebook`] neu gerechnet, die
//!   Differenz steigt um den Spielbetrag. Zu viel gebuchte Betriebskosten kommen als
//!   `betriebskosten_korrektur` zurück, ein ungeklärter Betrag wird angepasst. Mehrere
//!   stornierte Ereignisse im selben Fenster werden zusammen gerechnet.
//! - Storniert der Chef Betriebskosten aus einem Abgleich, erlässt die Firma sie. Der
//!   Abgleichpunkt merkt sich das (`costs_booked` sinkt, `costs_waived` steigt), damit eine
//!   spätere Korrektur dieses Fensters sie weder ein zweites Mal zurückgibt noch wieder anlegt.
//!   Erklären stornierte oder spät gemeldete Ereignisse den Fehlbetrag später, erklären sie
//!   zuerst den erlassenen Teil (siehe [`reconcile::rebook`]).
//! - Ist ein Teil der Betriebskosten schon korrigiert, etwa weil die Ereignisse davor storniert
//!   oder spät gemeldet wurden, erlässt das Storno nur, was für das Fenster noch gebucht ist
//!   (`costs_booked`). Den schon korrigierten Teil gleicht eine Buchung
//!   `betriebskosten_korrektur` mit Verweis auf den Abgleich aus, er käme sonst ein zweites Mal
//!   zurück. So zahlt die Firma am Ende dasselbe, egal ob der Chef erst die Ereignisse oder
//!   erst die Betriebskosten storniert, und die Betriebskosten lassen sich immer stornieren.
//!   In einem Aufruf kommen erst die Ereignisse dran, dann die Betriebskosten.
//!
//! Lohn: Stornierte Aufträge zählen in noch nicht abgerechneten Zeiträumen nicht mehr (siehe
//! [`crate::payroll`]). Schon gezahlten Lohn holt der Server nicht von selbst zurück, das tut
//! der Chef mit einem Storno der Lohnbuchung. Eine offene Lohnforderung aus einer Abrechnung
//! bleibt bei jedem Storno stehen, auch beim Storno der Lohnbuchung derselben Abrechnung. Die
//! erlässt der Chef getrennt (siehe [`crate::payroll::waive_claim`]). Die Antwort nennt deshalb
//! je Fahrer die offene Forderung.
use crate::{
    clock::format_local,
    error::{ApiError, ApiResult},
    reconcile::{self, Rebook},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Höchstens so viele Buchungen je Aufruf.
pub const MAX_ENTRIES: usize = 500;
/// Länge des Grunds in Zeichen, nach dem Trimmen.
pub const REASON_MIN: usize = 3;
pub const REASON_MAX: usize = 200;

/// Prüft den Grund eines Stornos oder einer erlassenen Lohnforderung: getrimmt, 3 bis 200
/// Zeichen, keine Steuerzeichen.
pub fn clean_reason(reason: Option<&str>) -> Result<String, String> {
    let r = reason.unwrap_or_default().trim();
    let n = r.chars().count();
    if !(REASON_MIN..=REASON_MAX).contains(&n) {
        return Err(format!(
            "Der Grund fehlt. Er braucht {REASON_MIN} bis {REASON_MAX} Zeichen"
        ));
    }
    if r.chars().any(char::is_control) {
        return Err("Der Grund darf keine Zeilenumbrüche oder Steuerzeichen enthalten".into());
    }
    Ok(r.to_string())
}

/// Deutsche Bezeichnung einer Buchungsart, für Meldungen und Notizen.
pub fn kind_name(kind: &str) -> &str {
    match kind {
        "auftrag" => "Auftrag",
        "auftrag_abgebrochen" => "Auftrag abgebrochen",
        "strafe" => "Strafe",
        "maut" => "Maut",
        "faehre" => "Fähre",
        "zug" => "Zug",
        "ueberweisung" => "Überweisung",
        "betriebskosten" => "Betriebskosten",
        "betriebskosten_korrektur" => "Betriebskosten, Korrektur",
        "werkstatt" => "Werkstatt",
        "haftung" => "Haftung",
        "lkw_verkauf" => "Lkw-Verkauf",
        "lohn" => "Lohn",
        "storno" => "Storno",
        other => other,
    }
}

/// Warum sich eine Buchung dieser Art grundsätzlich nicht stornieren lässt, oder `None`.
/// Ob sie schon storniert ist, prüft der Aufrufer getrennt.
pub fn not_reversible(kind: &str, account: &str, payroll_id: Option<i64>) -> Option<&'static str> {
    match (kind, account, payroll_id) {
        ("storno", _, _) => Some("ist selbst ein Storno und lässt sich nicht stornieren"),
        ("betriebskosten_korrektur", _, _) => Some(
            "ist eine Korrektur aus dem Geldabgleich. Die rechnet der Server selbst, storniere stattdessen das Ereignis oder die Betriebskosten dahinter",
        ),
        ("lohn", "firma", Some(_)) => Some(
            "ist die Summe einer Lohnabrechnung. Storniere die Lohnbuchungen der einzelnen Fahrer, das Geld kommt dabei in die Kasse zurück",
        ),
        _ => None,
    }
}

/// Eine Buchung, so weit das Storno sie braucht.
#[derive(Debug, Clone)]
struct Entry {
    id: i64,
    account: String,
    user_id: Option<i64>,
    kind: String,
    amount: i64,
    event_id: Option<String>,
    game_amount: Option<i64>,
    occurred_at: Option<i64>,
    payroll_id: Option<i64>,
    created_at: i64,
}

impl Entry {
    /// "Buchung 12 (Auftrag vom 16.11.2026 10:30)"
    fn describe(&self) -> String {
        format!(
            "Buchung {} ({} vom {})",
            self.id,
            kind_name(&self.kind),
            format_local(self.created_at)
        )
    }

    fn is_event(&self) -> bool {
        self.event_id.is_some() && self.game_amount.is_some() && self.user_id.is_some()
    }
}

fn load(conn: &Connection, company_id: i64, id: i64) -> rusqlite::Result<Option<Entry>> {
    conn.query_row(
        "SELECT id, account, user_id, kind, amount, event_id, game_amount, occurred_at,
                payroll_id, created_at
         FROM ledger WHERE id = ?1 AND company_id = ?2",
        [id, company_id],
        |r| {
            Ok(Entry {
                id: r.get(0)?,
                account: r.get(1)?,
                user_id: r.get(2)?,
                kind: r.get(3)?,
                amount: r.get(4)?,
                event_id: r.get(5)?,
                game_amount: r.get(6)?,
                occurred_at: r.get(7)?,
                payroll_id: r.get(8)?,
                created_at: r.get(9)?,
            })
        },
    )
    .optional()
}

/// Das Storno einer Buchung auf ihrem eigenen Konto: id und Zeitpunkt, oder `None`.
fn reversed_by(conn: &Connection, e: &Entry) -> rusqlite::Result<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT id, created_at FROM ledger WHERE reverses_id = ?1 AND account = ?2",
        params![e.id, e.account],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// Bucht sich diese Art als Paar auf beiden Konten (Überweisung, Haftung)?
fn is_pair(e: &Entry) -> bool {
    matches!(e.kind.as_str(), "ueberweisung" | "haftung")
}

/// Die andere Seite einer Überweisung oder Haftung. Beide Seiten werden in einer Transaktion
/// direkt nacheinander gebucht: Überweisung erst Kasse, dann Lohnkonto, Haftung erst
/// Lohnkonto, dann Kasse. Geprüft werden außerdem Art, Nutzer und Gegenbetrag. Der Zeitpunkt
/// zählt nicht: Ältere Server haben ihn je Seite neu gelesen, die beiden Seiten können also
/// eine Sekunde auseinander liegen.
fn partner(conn: &Connection, company_id: i64, e: &Entry) -> rusqlite::Result<Option<Entry>> {
    let id = match (e.kind.as_str(), e.account.as_str()) {
        ("ueberweisung", "firma") | ("haftung", "fahrer") => e.id + 1,
        ("ueberweisung", "fahrer") | ("haftung", "firma") => e.id - 1,
        _ => return Ok(None),
    };
    Ok(load(conn, company_id, id)?.filter(|p| {
        p.kind == e.kind
            && p.account != e.account
            && p.user_id == e.user_id
            && p.amount == -e.amount
    }))
}

/// Der Abgleich, zu dessen Fenster eine Betriebskosten-Buchung gehört. Seit dem Storno steht
/// er in `sync_id`. Ältere Betriebskosten, die ein Abgleich selbst gebucht hat, haben dort
/// nichts. Der Abgleich hat sie aber als letzte Buchung vor sich gebucht und deren id als
/// `last_ledger_id` gespeichert, zur selben Zeit und für denselben Fahrer. So lassen sie sich
/// ohne Änderung an der Buchung zuordnen. Passt nichts, kommt `None`.
pub fn cost_sync(
    conn: &Connection,
    company_id: i64,
    entry_id: i64,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT COALESCE(l.sync_id,
                         (SELECT s.id FROM sync_points s
                          WHERE s.company_id = l.company_id AND s.user_id = l.user_id
                            AND s.last_ledger_id = l.id AND s.created_at = l.created_at
                          ORDER BY s.id LIMIT 1))
         FROM ledger l WHERE l.id = ?1 AND l.company_id = ?2",
        [entry_id, company_id],
        |r| r.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// Was ein Storno für einen Fahrer im Geldabgleich bewirkt hat.
#[derive(Default)]
struct DriverEffect {
    /// Neu gerechnete Fenster, wie [`reconcile::rebook`] sie meldet, mit `events`
    corrections: Vec<Value>,
    /// Stornierte Ereignisse im offenen Fenster: Sie zählen beim nächsten Abgleich nicht mehr
    open_window: i64,
    /// Stornierte Betriebskosten aus einem Abgleich: `{ sync_id, amount }`
    waived: Vec<Value>,
}

/// Storniert die Buchungen `ids` der Firma, mit Grund `reason`, gebucht vom Chef `chef_id`.
/// Läuft in der Transaktion des Aufrufers. Bei einem Fehler bucht der Aufrufer nichts, er
/// darf die Transaktion dann nicht festschreiben.
///
/// Liefert die neuen Buchungen (`entries`) und je betroffenem Fahrer die Wirkung auf den
/// Geldabgleich (`drivers`, ohne Name und Lohnkonto, die ergänzt der Aufrufer).
pub fn reverse(
    conn: &Connection,
    company_id: i64,
    chef_id: i64,
    ids: &[i64],
    reason: &str,
    now: i64,
) -> ApiResult<(Vec<Value>, BTreeMap<i64, Value>)> {
    if ids.is_empty() {
        return Err(ApiError::bad("Es ist keine Buchung ausgewählt"));
    }
    if ids.len() > MAX_ENTRIES {
        return Err(ApiError::bad(&format!(
            "Höchstens {MAX_ENTRIES} Buchungen auf einmal stornieren"
        )));
    }
    let wanted: BTreeSet<i64> = ids.iter().copied().collect();

    // Erst alles prüfen, dann buchen
    let mut targets: BTreeMap<i64, Entry> = BTreeMap::new();
    for &id in &wanted {
        let e = load(conn, company_id, id)?.ok_or_else(|| {
            ApiError::not_found(&format!("Buchung {id} gibt es in dieser Firma nicht"))
        })?;
        if let Some(why) = not_reversible(&e.kind, &e.account, e.payroll_id) {
            return Err(ApiError::conflict(&format!("{} {why}", e.describe())));
        }
        if let Some((sid, at)) = reversed_by(conn, &e)? {
            return Err(ApiError::conflict(&format!(
                "{} ist schon storniert, am {} mit Buchung {sid}",
                e.describe(),
                format_local(at)
            )));
        }
        targets.insert(id, e);
    }
    // Die andere Seite von Überweisung und Haftung kommt mit. Ohne sie wird nichts gebucht,
    // sonst entstünde Geld aus dem Nichts. Ist sie schon storniert, bleibt nur diese Seite.
    let mut pairs = Vec::new();
    for e in targets.values().filter(|e| is_pair(e)) {
        match partner(conn, company_id, e)? {
            Some(p) => pairs.push(p),
            None => {
                return Err(ApiError::conflict(&format!(
                    "{}: Die Gegenbuchung auf dem anderen Konto ist nicht zu finden. Storniert wird nur beides zusammen",
                    e.describe()
                )));
            }
        }
    }
    for p in pairs {
        if !targets.contains_key(&p.id) && reversed_by(conn, &p)?.is_none() {
            targets.insert(p.id, p);
        }
    }

    // Buchen, älteste zuerst
    let mut entries = Vec::new();
    let mut insert = |e: &Entry, account: &str, amount: i64| -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at,
                                 reverses_id, booked_by)
             VALUES (?1, ?2, ?3, 'storno', ?4, ?5, ?6, ?7, ?8)",
            params![
                company_id, account, e.user_id, amount, reason, now, e.id, chef_id
            ],
        )?;
        entries.push(json!({
            "id": conn.last_insert_rowid(), "account": account, "user_id": e.user_id,
            "kind": "storno", "amount": amount, "reverses_id": e.id,
            "reverses_kind": e.kind, "note": reason, "created_at": now
        }));
        Ok(())
    };
    for e in targets.values() {
        insert(e, &e.account, -e.amount)?;
        // Lohn aus einer Abrechnung: Das Geld geht an die Kasse zurück
        if e.kind == "lohn" && e.account == "fahrer" && e.payroll_id.is_some() {
            insert(e, "firma", e.amount)?;
        }
    }

    let mut effects: BTreeMap<i64, DriverEffect> = BTreeMap::new();
    for uid in targets.values().filter_map(|e| e.user_id) {
        effects.entry(uid).or_default();
    }

    // Ereignisse: je Fahrer und abgeglichenem Fenster zusammen neu rechnen
    let mut windows: BTreeMap<(i64, i64), Vec<&Entry>> = BTreeMap::new();
    for e in targets.values().filter(|e| e.is_event()) {
        let uid = e.user_id.unwrap_or_default();
        let window = match e.occurred_at {
            Some(t) => reconcile::window_at(conn, company_id, uid, t)?,
            None => reconcile::window_of_undated(conn, company_id, uid, e.id)?,
        };
        match window {
            Some(sync_id) => windows.entry((uid, sync_id)).or_default().push(e),
            None => {
                if in_open_window(conn, company_id, uid, e)? {
                    effects.entry(uid).or_default().open_window += 1;
                }
            }
        }
    }
    for ((uid, sync_id), events) in &windows {
        let sum: i64 = events.iter().filter_map(|e| e.game_amount).sum();
        if sum == 0 {
            continue;
        }
        let prefix = match events.as_slice() {
            [one] => format!("Storno: {}", kind_name(&one.kind)),
            many => format!("Storno: {} Ereignisse", many.len()),
        };
        let mut c = reconcile::rebook(
            conn,
            &Rebook {
                company_id,
                user_id: *uid,
                sync_id: *sync_id,
                diff_change: sum,
                note_prefix: &prefix,
                explained_note: reconcile::REVERSED_NOTE,
                now,
            },
        )?;
        c["events"] = json!(events.len());
        effects.entry(*uid).or_default().corrections.push(c);
    }

    // Betriebskosten aus einem Abgleich: erlassen. Erst nach den Ereignissen, damit ein Storno
    // nie mehr zurückgibt, als für das Fenster noch gebucht ist.
    for e in targets
        .values()
        .filter(|e| e.kind == "betriebskosten" && e.account == "firma" && e.amount < 0)
    {
        let Some(sync_id) = cost_sync(conn, company_id, e.id)? else {
            // Ohne Abgleich (etwa von Hand in die Datenbank geschrieben): nur die Gegenbuchung
            continue;
        };
        let (booked, synced_at): (i64, i64) = conn.query_row(
            "SELECT costs_booked, created_at FROM sync_points WHERE id = ?1",
            [sync_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let c = -e.amount;
        // Erlassen wird, was für das Fenster noch gebucht ist. Den Rest hat eine Korrektur
        // schon zurückgegeben, etwa für stornierte oder spät gemeldete Ereignisse. Den gleicht
        // eine Buchung aus, sonst käme er ein zweites Mal zurück.
        let waive = c.min(booked.max(0));
        let corrected = c - waive;
        if corrected > 0 {
            let note = format!(
                "Ausgleich zum Storno von Buchung {}: {corrected} € davon waren schon korrigiert, Abgleich vom {}",
                e.id,
                format_local(synced_at)
            );
            conn.execute(
                "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at,
                                     sync_id)
                 VALUES (?1, 'firma', ?2, 'betriebskosten_korrektur', ?3, ?4, ?5, ?6)",
                params![company_id, e.user_id, -corrected, note, now, sync_id],
            )?;
            entries.push(json!({
                "id": conn.last_insert_rowid(), "account": "firma", "user_id": e.user_id,
                "kind": "betriebskosten_korrektur", "amount": -corrected, "note": note,
                "created_at": now, "sync_id": sync_id
            }));
        }
        conn.execute(
            "UPDATE sync_points SET costs_booked = costs_booked - ?1,
                                    costs_waived = costs_waived + ?1
             WHERE id = ?2",
            params![waive, sync_id],
        )?;
        if let Some(uid) = e.user_id {
            effects.entry(uid).or_default().waived.push(json!({
                "sync_id": sync_id, "entry_id": e.id, "amount": waive,
                "already_corrected": corrected
            }));
        }
    }

    let drivers = effects
        .into_iter()
        .map(|(uid, d)| {
            (
                uid,
                json!({
                    "user_id": uid, "corrections": d.corrections,
                    "open_window": d.open_window, "waived": d.waived
                }),
            )
        })
        .collect();
    Ok((entries, drivers))
}

/// Liegt ein Ereignis im noch offenen Fenster nach dem letzten Abgleich des Fahrers? Dann
/// hätte es beim nächsten Abgleich gezählt.
fn in_open_window(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    e: &Entry,
) -> rusqlite::Result<bool> {
    let Some(last) = reconcile::last_point(conn, company_id, user_id)? else {
        // Vor dem ersten Abgleich: Der erste Abgleich zählt keine Ereignisse
        return Ok(false);
    };
    Ok(match e.occurred_at {
        Some(t) => t > last.base(),
        None => e.id > last.last_ledger_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grund_pruefen() {
        assert_eq!(clean_reason(Some("  privat  ")).unwrap(), "privat");
        assert!(clean_reason(None).is_err());
        assert!(clean_reason(Some("  ab ")).is_err());
        assert_eq!(clean_reason(Some("abc")).unwrap(), "abc");
        assert!(clean_reason(Some(&"x".repeat(200))).is_ok());
        assert!(clean_reason(Some(&"x".repeat(201))).is_err());
        // Zeichen, nicht Bytes
        assert!(clean_reason(Some(&"ü".repeat(200))).is_ok());
        assert!(clean_reason(Some("zwei\nZeilen")).is_err());
    }

    #[test]
    fn was_sich_nicht_stornieren_laesst() {
        assert!(not_reversible("storno", "firma", None).is_some());
        assert!(not_reversible("betriebskosten_korrektur", "firma", None).is_some());
        assert!(not_reversible("lohn", "firma", Some(3)).is_some());
        // Lohnbuchung eines Fahrers und Lohnanteil aus dem alten Modell gehen
        assert!(not_reversible("lohn", "fahrer", Some(3)).is_none());
        assert!(not_reversible("lohn", "fahrer", None).is_none());
        for kind in [
            "auftrag",
            "strafe",
            "maut",
            "betriebskosten",
            "ueberweisung",
        ] {
            assert!(not_reversible(kind, "firma", None).is_none(), "{kind}");
        }
        assert_eq!(kind_name("faehre"), "Fähre");
        assert_eq!(kind_name("neu"), "neu");
    }
}
