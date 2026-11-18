//! Geldabgleich: Abgleichfenster und spät gemeldete Ereignisse.
//!
//! Alle Zeitpunkte, die über ein Fenster entscheiden, kommen möglichst von derselben Uhr, der
//! des Fahrer-PCs:
//!
//! - `occurred_at` eines Ereignisses: Systemzeit des PCs, auf dem das Plugin läuft
//! - `cutoff` eines Abgleichpunkts: Änderungszeit des Spielstands, den der Launcher gelesen hat
//! - `written_at`: wann der Launcher den neuen Spielstand geschrieben hat
//!
//! Fehlt `cutoff` oder `written_at` oder ist er unplausibel, gilt der Empfangszeitpunkt beim
//! Server. Ein `cutoff` liegt nie vor der Grundlage des vorigen Abgleichs, sonst überlappten
//! die Fenster, wenn Abgleiche mit verschiedenen Uhren kommen (Launcher und Dashboard).
//!
//! Jeder Abgleichpunkt k hat eine Grundlage `base_k`: `written_at_k`, wenn der Launcher das
//! Schreiben bestätigt hat, sonst `cutoff_k`. Das Fenster von Abgleich k ist
//! `(base_{k-1}, cutoff_k]`. Nur Ereignisse darin zählen für den erwarteten Stand von k.
//! Ereignisse zwischen `cutoff_k` und `base_k` zählen in keinem Fenster: Da war das Spiel zu.
//!
//! Ein Ereignis ohne plausiblen Zeitpunkt vom Client (älteres Plugin, Dashboard) hat in
//! `ledger.occurred_at` nichts stehen. Es zählt nach der Reihenfolge des Empfangs beim Server:
//! im ersten Abgleich, der nach ihm ankommt. Die Serveruhr wird dabei nie mit der Uhr des
//! Fahrer-PCs verglichen, und ein solches Ereignis korrigiert nie einen alten Abgleich.
//!
//! Kommt ein Ereignis mit Zeitpunkt erst an, nachdem sein Fenster abgeglichen ist, steckte sein
//! Betrag schon im damaligen Kontostand. [`correct_late`] rechnet das Fenster dann neu und
//! bucht nur die Differenz, alte Buchungen bleiben unverändert.
//!
//! Storniert der Chef ein Ereignis aus einem schon abgeglichenen Fenster, hat es damals
//! mitgezählt, obwohl es nicht hätte zählen dürfen. Dann rechnet [`rebook`] das Fenster
//! genauso neu, nur mit umgekehrtem Vorzeichen (siehe [`crate::reversal`]). Im offenen Fenster
//! zählt ein storniertes Ereignis einfach nicht mehr ([`window_sum`]).
use crate::clock::format_local;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// 2020-01-01 00:00 UTC. Frühere Zeitpunkte sind sicher falsch.
pub const EARLIEST: i64 = 1_577_836_800;
/// So weit darf ein Zeitpunkt vor der Uhr des Servers liegen.
pub const MAX_AHEAD: i64 = 86_400;

/// Notiz eines ungeklärten Betrags, den spät gemeldete Ereignisse erklärt haben.
pub const EXPLAINED_NOTE: &str = "Durch spät gemeldete Ereignisse erklärt";

/// Notiz eines ungeklärten Betrags, den stornierte Ereignisse erklärt haben.
pub const REVERSED_NOTE: &str = "Durch stornierte Ereignisse erklärt";

/// Der Zeitpunkt vom Client, wenn er plausibel ist, sonst `None`. Unplausible Zeitpunkte
/// werden nicht abgelehnt, sonst hinge ein Ereignis für immer im Postausgang des Plugins.
pub fn client_time(t: Option<i64>, now: i64) -> Option<i64> {
    t.filter(|t| (EARLIEST..=now.saturating_add(MAX_AHEAD)).contains(t))
}

/// Der Zeitpunkt vom Client, wenn er plausibel ist, sonst `now`.
pub fn plausible_time(t: Option<i64>, now: i64) -> i64 {
    client_time(t, now).unwrap_or(now)
}

/// Grundlage eines Abgleichpunkts für das nächste Fenster.
pub fn base(written: bool, written_at: Option<i64>, cutoff: i64) -> i64 {
    match (written, written_at) {
        (true, Some(w)) => w,
        _ => cutoff,
    }
}

/// Der letzte Abgleichpunkt eines Fahrers.
pub struct LastPoint {
    pub ingame_money: i64,
    pub write_money: i64,
    pub written: bool,
    pub cutoff: i64,
    pub written_at: Option<i64>,
    /// Größte Buchung beim Abgleich. Ereignisse ohne Zeitpunkt danach gehören zum nächsten.
    pub last_ledger_id: i64,
}

impl LastPoint {
    /// Von diesem Stand rechnet der nächste Abgleich.
    pub fn baseline(&self) -> i64 {
        if self.written {
            self.write_money
        } else {
            self.ingame_money
        }
    }

    pub fn base(&self) -> i64 {
        base(self.written, self.written_at, self.cutoff)
    }
}

pub fn last_point(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
) -> rusqlite::Result<Option<LastPoint>> {
    conn.query_row(
        "SELECT ingame_money, write_money, written, COALESCE(cutoff, created_at), written_at,
                last_ledger_id
         FROM sync_points WHERE company_id = ?1 AND user_id = ?2 ORDER BY id DESC LIMIT 1",
        [company_id, user_id],
        |r| {
            Ok(LastPoint {
                ingame_money: r.get(0)?,
                write_money: r.get(1)?,
                written: r.get(2)?,
                cutoff: r.get(3)?,
                written_at: r.get(4)?,
                last_ledger_id: r.get(5)?,
            })
        },
    )
    .optional()
}

/// Summe der Spielbeträge und Zahl der Ereignisse eines Fahrers im Fenster `(from, to]`.
/// Ereignisse ohne Zeitpunkt vom Client zählen, wenn sie nach der Buchung `after_id`
/// angekommen sind, also nach dem vorigen Abgleich. Stornierte Ereignisse zählen nicht.
pub fn window_sum(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    from: i64,
    to: i64,
    after_id: i64,
) -> rusqlite::Result<(i64, i64)> {
    conn.query_row(
        "SELECT COALESCE(SUM(l.game_amount), 0), COUNT(*) FROM ledger l
         WHERE l.company_id = ?1 AND l.user_id = ?2 AND l.game_amount IS NOT NULL
           AND l.event_id IS NOT NULL
           AND CASE WHEN l.occurred_at IS NULL THEN l.id > ?5
                    ELSE l.occurred_at > ?3 AND l.occurred_at <= ?4 END
           AND NOT EXISTS (SELECT 1 FROM ledger s WHERE s.reverses_id = l.id)",
        params![company_id, user_id, from, to, after_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Der schon abgeglichene Abgleich eines Fahrers, dessen Fenster den Zeitpunkt `t` enthält:
/// `(base des Vorgängers, cutoff]`, nie der erste Abgleich. `None`, wenn `t` vor dem ersten
/// Abgleich liegt, zwischen gelesenem und neu geschriebenem Spielstand oder im noch offenen
/// Fenster nach dem letzten Abgleich.
pub fn window_at(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    t: i64,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT s.id
         FROM sync_points s
         JOIN sync_points p ON p.id = (
             SELECT MAX(q.id) FROM sync_points q
             WHERE q.company_id = s.company_id AND q.user_id = s.user_id AND q.id < s.id)
         WHERE s.company_id = ?1 AND s.user_id = ?2 AND s.diff IS NOT NULL
           AND ?3 > CASE WHEN p.written = 1
                         THEN COALESCE(p.written_at, p.cutoff, p.created_at)
                         ELSE COALESCE(p.cutoff, p.created_at) END
           AND ?3 <= COALESCE(s.cutoff, s.created_at)
         ORDER BY s.id DESC LIMIT 1",
        params![company_id, user_id, t],
        |r| r.get(0),
    )
    .optional()
}

/// Der Abgleich, der ein Ereignis ohne Zeitpunkt vom Client (Buchung `ledger_id`) gezählt hat:
/// der erste Abgleich des Fahrers, der nach dem Ereignis angekommen ist (`last_ledger_id`
/// mindestens `ledger_id`). War das sein erster Abgleich, steckt das Ereignis im Anfangsstand
/// und es kommt `None`, ebenso, wenn es noch keinen solchen Abgleich gibt (offenes Fenster).
pub fn window_of_undated(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    ledger_id: i64,
) -> rusqlite::Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id, diff FROM sync_points
             WHERE company_id = ?1 AND user_id = ?2 AND last_ledger_id >= ?3
             ORDER BY id LIMIT 1",
            params![company_id, user_id, ledger_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?
        .and_then(|(id, diff)| diff.map(|_| id)))
}

/// Ein spät gemeldetes Ereignis: Es ist laut Client zum Zeitpunkt `t` im Spiel passiert und hat den
/// Kontostand im Spiel um `game_amount` verändert, kam aber erst nach dem Abgleich an, dessen
/// Fenster `t` enthält. Dann steckte sein Betrag schon im damaligen Kontostand, und das Fenster
/// wird mit [`rebook`] neu gerechnet: `diff` sinkt um `game_amount`.
///
/// Gibt es kein solches Fenster, etwa weil `t` vor dem ersten Abgleich liegt oder zwischen
/// gelesenem und neu geschriebenem Spielstand, passiert nichts und es kommt `None` zurück.
/// Die normale Buchung des Ereignisses macht der Aufrufer. Aufgerufen wird das nur mit einem
/// Zeitpunkt vom Client: Ein Ereignis ohne ihn ist nachweislich nach jedem bisherigen
/// Abgleich angekommen und gehört ins nächste Fenster.
pub fn correct_late(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    t: i64,
    game_amount: i64,
    label: &str,
    now: i64,
) -> rusqlite::Result<Option<Value>> {
    let Some(sync_id) = window_at(conn, company_id, user_id, t)? else {
        return Ok(None);
    };
    if game_amount == 0 {
        return Ok(None);
    }
    rebook(
        conn,
        &Rebook {
            company_id,
            user_id,
            sync_id,
            diff_change: -game_amount,
            note_prefix: &format!("Spät gemeldet: {label}"),
            explained_note: EXPLAINED_NOTE,
            now,
        },
    )
    .map(Some)
}

/// Was [`rebook`] mit einem Abgleich tun soll.
pub struct Rebook<'a> {
    pub company_id: i64,
    pub user_id: i64,
    pub sync_id: i64,
    /// Um so viel ändert sich die Differenz (im Spiel minus erwartet) des Abgleichs. Ein spät
    /// gemeldetes Ereignis hätte mitgezählt werden müssen: minus sein Spielbetrag. Ein
    /// storniertes Ereignis hätte nicht mitzählen dürfen: plus sein Spielbetrag.
    pub diff_change: i64,
    /// Anfang der Notiz an den Buchungen, dahinter kommt ", Abgleich vom ..."
    pub note_prefix: &'a str,
    /// Notiz an einem ungeklärten Betrag, der dadurch ganz erklärt ist
    pub explained_note: &'a str,
    pub now: i64,
}

/// Rechnet das Fenster eines schon abgeglichenen Abgleichs neu und bucht nur den Unterschied,
/// alte Buchungen bleiben unverändert:
///
/// - `diff` des Abgleichs ändert sich um `diff_change`
/// - Erlassen: Was der Chef an Betriebskosten dieses Fensters storniert hat (`costs_waived`),
///   bleibt erlassen. Sinkt die fehlende Summe `max(0, -diff)`, weil Ereignisse sie erklären,
///   erklären sie zuerst den erlassenen Teil, `costs_waived` sinkt um denselben Betrag (nie
///   unter 0). Steigt sie, ist das neu entdeckter Aufwand, `costs_waived` bleibt. So kommt
///   dasselbe heraus, ob der Chef erst die Betriebskosten oder erst die Ereignisse storniert.
/// - Betriebskosten: Ziel ist `max(0, max(0, -diff) - costs_waived)`, also die fehlende Summe
///   ohne den erlassenen Teil. Zu viel gebucht ergibt eine Gutschrift
///   `betriebskosten_korrektur`, zu wenig eine neue Buchung `betriebskosten`. Beide tragen den
///   Abgleich in `sync_id`, damit das Anschaffungsbudget sie dem richtigen Abgleich zuordnet.
/// - Ungeklärt: Ziel ist `max(0, diff)`. Ein offener Eintrag bekommt den Rest, den der Chef
///   nicht schon mit einem geschlossenen Eintrag hingenommen hat. Bleibt nichts, wird er
///   geschlossen. Gibt es keinen offenen und bleibt etwas, kommt ein neuer dazu.
pub fn rebook(conn: &Connection, r: &Rebook<'_>) -> rusqlite::Result<Value> {
    let (diff, costs_booked, waived, synced_at): (i64, i64, i64, i64) = conn.query_row(
        "SELECT diff, costs_booked, costs_waived, created_at FROM sync_points WHERE id = ?1",
        [r.sync_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let (company_id, user_id, sync_id, now) = (r.company_id, r.user_id, r.sync_id, r.now);
    let new_diff = diff + r.diff_change;
    let note = format!(
        "{}, Abgleich vom {}",
        r.note_prefix,
        format_local(synced_at)
    );

    // Erlassen: Erklärte Fehlbeträge erklären zuerst den erlassenen Teil
    let (missing_before, missing) = ((-diff).max(0), (-new_diff).max(0));
    let waived = (waived - (missing_before - missing).max(0)).max(0);

    // Betriebskosten
    let mut booked = Vec::new();
    let costs = (missing - waived).max(0);
    let delta = costs - costs_booked;
    if delta != 0 {
        // Zu viel gebucht: Gutschrift. Zu wenig: weitere Betriebskosten.
        let kind = if delta < 0 {
            "betriebskosten_korrektur"
        } else {
            "betriebskosten"
        };
        conn.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at,
                                 sync_id)
             VALUES (?1, 'firma', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![company_id, user_id, kind, -delta, note, now, sync_id],
        )?;
        booked.push(json!({ "account": "firma", "kind": kind, "amount": -delta }));
    }

    // Ungeklärt. Was der Chef mit einem geschlossenen Eintrag hingenommen hat, bleibt dabei.
    let surplus = new_diff.max(0);
    let accepted: i64 = conn.query_row(
        "SELECT COALESCE(SUM(amount), 0) FROM unexplained
         WHERE sync_id = ?1 AND closed_at IS NOT NULL AND closed_by IS NOT NULL",
        [sync_id],
        |r| r.get(0),
    )?;
    let rest = surplus - accepted;
    let open: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, amount FROM unexplained WHERE sync_id = ?1 AND closed_at IS NULL
             ORDER BY id DESC LIMIT 1",
            [sync_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let unexplained = match open {
        Some((id, _)) if rest > 0 => {
            conn.execute(
                "UPDATE unexplained SET amount = ?1 WHERE id = ?2",
                params![rest, id],
            )?;
            json!({ "id": id, "amount": rest, "closed": false })
        }
        Some((id, amount)) => {
            // Der Betrag bleibt stehen, damit der Chef sieht, was erklärt wurde
            conn.execute(
                "UPDATE unexplained SET closed_at = ?1, closed_by = NULL, close_note = ?2
                 WHERE id = ?3",
                params![now, r.explained_note, id],
            )?;
            json!({ "id": id, "amount": amount, "closed": true })
        }
        None if rest > 0 => {
            conn.execute(
                "INSERT INTO unexplained (company_id, user_id, amount, sync_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![company_id, user_id, rest, sync_id, now],
            )?;
            json!({ "id": conn.last_insert_rowid(), "amount": rest, "closed": false })
        }
        None => Value::Null,
    };

    conn.execute(
        "UPDATE sync_points SET diff = ?1, costs_booked = ?2, unexplained_attributed = ?3,
                                costs_waived = ?4
         WHERE id = ?5",
        params![new_diff, costs, surplus, waived, sync_id],
    )?;
    Ok(json!({
        "sync_id": sync_id, "diff_before": diff, "diff": new_diff,
        "booked": booked, "unexplained": unexplained
    }))
}

/// Deutsche Bezeichnung einer Buchungsart aus einem Ereignis, für Notizen.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "auftrag" => "Auftrag",
        "auftrag_abgebrochen" => "Auftrag abgebrochen",
        "strafe" => "Strafe",
        "maut" => "Maut",
        "faehre" => "Fähre",
        "zug" => "Zug",
        _ => "Ereignis",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeitpunkte_pruefen() {
        let now = 1_795_000_000;
        assert_eq!(client_time(None, now), None);
        assert_eq!(client_time(Some(now - 3600), now), Some(now - 3600));
        assert_eq!(client_time(Some(EARLIEST - 1), now), None);
        assert_eq!(client_time(Some(now + MAX_AHEAD + 1), now), None);
        assert_eq!(plausible_time(None, now), now);
        assert_eq!(plausible_time(Some(now - 3600), now), now - 3600);
        assert_eq!(plausible_time(Some(EARLIEST), now), EARLIEST);
        assert_eq!(plausible_time(Some(EARLIEST - 1), now), now);
        assert_eq!(plausible_time(Some(0), now), now);
        assert_eq!(plausible_time(Some(-5), now), now);
        assert_eq!(plausible_time(Some(now + MAX_AHEAD), now), now + MAX_AHEAD);
        assert_eq!(plausible_time(Some(now + MAX_AHEAD + 1), now), now);
        assert_eq!(plausible_time(Some(i64::MAX), now), now);
    }

    #[test]
    fn grundlage_eines_abgleichs() {
        assert_eq!(base(false, None, 10), 10);
        assert_eq!(base(false, Some(20), 10), 10);
        assert_eq!(base(true, Some(20), 10), 20);
        assert_eq!(base(true, None, 10), 10);
    }
}
