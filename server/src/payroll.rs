//! Tägliche Lohnabrechnung.
//!
//! Einmal am Tag zur eingestellten Uhrzeit (Europe/Berlin) rechnet der Server den Vortag ab: je
//! Fahrer Kilometer mal Cent je km seiner Stufe plus Tagespauschale für jeden Tag mit mindestens
//! einem abgelieferten Auftrag. Das Geld geht aus der Firmenkasse auf das Lohnkonto.
//!
//! Reicht die Kasse nicht, bekommen alle denselben Anteil, der Rest wird offene Lohnforderung.
//! Offene Forderungen werden bei der nächsten Abrechnung zuerst bedient.
//!
//! Stornierte Aufträge zählen nicht, weder mit Kilometern noch für die Pauschale. Das wirkt nur
//! auf Zeiträume, die noch nicht abgerechnet sind: Schon gezahlten Lohn holt der Server nicht
//! zurück. Das kann der Chef, indem er die Lohnbuchung storniert (siehe [`crate::reversal`]).
//! Auch eine offene Forderung aus einer schon abgerechneten Zeit bleibt beim Storno stehen und
//! würde bei der nächsten Abrechnung zuerst gezahlt. Die erlässt der Chef mit [`waive_claim`],
//! ganz oder zum Teil.
//!
//! Jeder Tag wird genau einmal abgerechnet: `payroll_runs` hat einen eindeutigen Index auf Firma
//! und Tag, und die Abrechnung läuft in einer Transaktion. Nach einem Neustart holt
//! [`run_due`] verpasste Tage der Reihe nach nach.
use crate::{
    clock::{day_start, format_day, local_day, local_to_utc},
    levels::{standing, wage_levels},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn company_balance(conn: &Connection, company_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(amount), 0) FROM ledger WHERE company_id = ?1 AND account = 'firma'",
        [company_id],
        |r| r.get(0),
    )
}

/// Offene Lohnforderung eines Fahrers: was Abrechnungen nicht zahlen konnten, abzüglich dessen,
/// was spätere Abrechnungen bedient haben und was der Chef erlassen hat.
pub fn open_claim(conn: &Connection, company_id: i64, user_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT (SELECT COALESCE(SUM(amount), 0) FROM wage_claims
                 WHERE company_id = ?1 AND user_id = ?2)
              - (SELECT COALESCE(SUM(amount), 0) FROM wage_claim_waivers
                 WHERE company_id = ?1 AND user_id = ?2)",
        [company_id, user_id],
        |r| r.get(0),
    )
}

/// Warum sich eine Forderung nicht erlassen lässt.
#[derive(Debug, PartialEq)]
pub enum WaiveError {
    /// Betrag 0 oder negativ
    NotPositive,
    /// Mehr als offen ist: die offene Forderung
    TooMuch(i64),
}

/// Erlässt `amount` von der offenen Lohnforderung eines Fahrers, mit Grund, ausgelöst vom Chef
/// `chef_id`. Bucht nichts im Kassenbuch, es fließt kein Geld: Die Forderung wird nur nicht
/// mehr ausgezahlt. Gibt die offene Forderung danach zurück.
pub fn waive_claim(
    conn: &Connection,
    company_id: i64,
    user_id: i64,
    amount: i64,
    (reason, chef_id): (&str, i64),
    now: i64,
) -> rusqlite::Result<Result<i64, WaiveError>> {
    if amount <= 0 {
        return Ok(Err(WaiveError::NotPositive));
    }
    let open = open_claim(conn, company_id, user_id)?;
    if amount > open {
        return Ok(Err(WaiveError::TooMuch(open)));
    }
    conn.execute(
        "INSERT INTO wage_claim_waivers (company_id, user_id, amount, reason, booked_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![company_id, user_id, amount, reason, chef_id, now],
    )?;
    Ok(Ok(open - amount))
}

/// Erlassene Forderungen der Firma, neueste zuerst, mit `created_at` in `[from, to)`.
pub fn waivers_json(
    conn: &Connection,
    company_id: i64,
    (from, to): (Option<i64>, Option<i64>),
    limit: i64,
) -> rusqlite::Result<Vec<Value>> {
    conn.prepare(
        "SELECT w.id, w.user_id, u.name, w.amount, w.reason, b.name, w.created_at
         FROM wage_claim_waivers w JOIN users u ON u.id = w.user_id
         JOIN users b ON b.id = w.booked_by
         WHERE w.company_id = ?1 AND (?2 IS NULL OR w.created_at >= ?2)
           AND (?3 IS NULL OR w.created_at < ?3)
         ORDER BY w.id DESC LIMIT ?4",
    )?
    .query_map(params![company_id, from, to, limit], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "user_id": r.get::<_, i64>(1)?,
            "user": r.get::<_, String>(2)?, "amount": r.get::<_, i64>(3)?,
            "reason": r.get::<_, String>(4)?, "booked_by_user": r.get::<_, String>(5)?,
            "created_at": r.get::<_, i64>(6)?
        }))
    })?
    .collect()
}

/// Ganze Euro aus Cent, kaufmännisch gerundet.
fn euro_from_cents(cents: i64) -> i64 {
    (cents + 50).div_euclid(100)
}

/// Teilt `available` anteilig auf die Beträge in `due` auf. Reicht es für alle, bekommt jeder
/// seinen Betrag, sonst jeder denselben Anteil (abgerundet).
fn share(due: &[i64], available: i64) -> Vec<i64> {
    let total: i64 = due.iter().sum();
    if total <= available {
        return due.to_vec();
    }
    if available <= 0 || total <= 0 {
        return vec![0; due.len()];
    }
    due.iter()
        .map(|d| ((*d as i128 * available as i128) / total as i128) as i64)
        .collect()
}

struct Line {
    user_id: i64,
    km: i64,
    stage: Option<i64>,
    cents_per_km: i64,
    days: BTreeSet<i64>,
    flat: i64,
    wage: i64,
    claim_before: i64,
}

/// Rechnet die nicht stornierten Aufträge mit `created_at` in `[from, to)` ab und bucht. Läuft in der Transaktion
/// des Aufrufers. Gibt die id der Abrechnung zurück.
pub fn run_period(
    tx: &Transaction,
    company_id: i64,
    day: i64,
    kind: &str,
    from: i64,
    to: i64,
    now: i64,
) -> rusqlite::Result<i64> {
    let to = to.max(from);
    tx.execute(
        "INSERT INTO payroll_runs (company_id, day, kind, period_from, period_to, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![company_id, day, kind, from, to, now],
    )?;
    let pid = tx.last_insert_rowid();
    let levels = wage_levels(tx, company_id)?;
    let daily_flat: i64 = tx.query_row(
        "SELECT daily_flat FROM companies WHERE id = ?1",
        [company_id],
        |r| r.get(0),
    )?;
    let members: Vec<i64> = tx
        .prepare("SELECT user_id FROM memberships WHERE company_id = ?1 ORDER BY user_id")?
        .query_map([company_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;

    let mut lines = Vec::new();
    for user_id in members {
        let jobs: Vec<(i64, Option<i64>)> = tx
            .prepare(
                "SELECT l.created_at, l.distance_km FROM ledger l
                 WHERE l.company_id = ?1 AND l.user_id = ?2 AND l.kind = 'auftrag'
                   AND l.account = 'firma' AND l.created_at >= ?3 AND l.created_at < ?4
                   AND NOT EXISTS (SELECT 1 FROM ledger s WHERE s.reverses_id = l.id)",
            )?
            .query_map(params![company_id, user_id, from, to], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
        let km: i64 = jobs.iter().filter_map(|(_, d)| *d).sum();
        let mut days = BTreeSet::new();
        for (t, _) in &jobs {
            let d = local_day(*t);
            let paid: bool = tx
                .prepare(
                    "SELECT 1 FROM flat_days WHERE company_id = ?1 AND user_id = ?2 AND day = ?3",
                )?
                .exists(params![company_id, user_id, d])?;
            if !paid {
                days.insert(d);
            }
        }
        let s = standing(tx, &levels, company_id, user_id)?;
        let flat = daily_flat * days.len() as i64;
        let wage = euro_from_cents(km * s.cents_per_km()) + flat;
        let claim_before = open_claim(tx, company_id, user_id)?;
        if jobs.is_empty() && claim_before == 0 {
            continue;
        }
        lines.push(Line {
            user_id,
            km,
            stage: s.stage.map(|w| w.position),
            cents_per_km: s.cents_per_km(),
            days,
            flat,
            wage,
            claim_before,
        });
    }

    // Erst die offenen Forderungen, dann der neue Lohn
    let mut available = company_balance(tx, company_id)?.max(0);
    let claims: Vec<i64> = lines.iter().map(|l| l.claim_before.max(0)).collect();
    let paid_claims = share(&claims, available);
    available -= paid_claims.iter().sum::<i64>();
    let wages: Vec<i64> = lines.iter().map(|l| l.wage).collect();
    let paid_wages = share(&wages, available);

    let label = format_day(day);
    let mut total = 0;
    for (i, l) in lines.iter().enumerate() {
        let paid = paid_claims[i] + paid_wages[i];
        let claim_after = l.claim_before - paid_claims[i] + (l.wage - paid_wages[i]);
        total += paid;
        if paid != 0 {
            tx.execute(
                "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at, payroll_id)
                 VALUES (?1, 'fahrer', ?2, 'lohn', ?3, ?4, ?5, ?6)",
                params![company_id, l.user_id, paid, format!("Lohnabrechnung {label}"), now, pid],
            )?;
        }
        if claim_after != l.claim_before {
            tx.execute(
                "INSERT INTO wage_claims (company_id, user_id, amount, payroll_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    company_id,
                    l.user_id,
                    claim_after - l.claim_before,
                    pid,
                    now
                ],
            )?;
        }
        for d in &l.days {
            tx.execute(
                "INSERT INTO flat_days (company_id, user_id, day, payroll_id) VALUES (?1, ?2, ?3, ?4)",
                params![company_id, l.user_id, d, pid],
            )?;
        }
        tx.execute(
            "INSERT INTO payroll_lines (payroll_id, user_id, km, stage, cents_per_km, flat_days, flat,
                                        wage, claim_before, paid, claim_after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                pid,
                l.user_id,
                l.km,
                l.stage,
                l.cents_per_km,
                l.days.len() as i64,
                l.flat,
                l.wage,
                l.claim_before,
                paid,
                claim_after
            ],
        )?;
    }
    if total != 0 {
        tx.execute(
            "INSERT INTO ledger (company_id, account, user_id, kind, amount, note, created_at, payroll_id)
             VALUES (?1, 'firma', NULL, 'lohn', ?2, ?3, ?4, ?5)",
            params![company_id, -total, format!("Lohnabrechnung {label}"), now, pid],
        )?;
    }
    Ok(pid)
}

/// Ende der zuletzt abgerechneten Zeitspanne, oder der Beginn der Abrechnung in dieser Firma.
fn last_period_end(conn: &Connection, company_id: i64) -> rusqlite::Result<i64> {
    let last: Option<i64> = conn.query_row(
        "SELECT MAX(period_to) FROM payroll_runs WHERE company_id = ?1",
        [company_id],
        |r| r.get(0),
    )?;
    match last {
        Some(t) => Ok(t),
        None => conn.query_row(
            "SELECT COALESCE(payroll_from, created_at) FROM companies WHERE id = ?1",
            [company_id],
            |r| r.get(0),
        ),
    }
}

/// Holt alle fälligen täglichen Abrechnungen einer Firma nach, ältester Tag zuerst. Ein Tag ist
/// fällig, wenn am Folgetag die eingestellte Uhrzeit erreicht ist. Gibt die ids zurück.
pub fn run_due(conn: &mut Connection, company_id: i64, now: i64) -> rusqlite::Result<Vec<i64>> {
    let mut done = Vec::new();
    loop {
        let tx = conn.transaction()?;
        let (minute, from_ts): (i64, i64) = tx.query_row(
            "SELECT payroll_minute, COALESCE(payroll_from, created_at) FROM companies WHERE id = ?1",
            [company_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let last_day: Option<i64> = tx
            .query_row(
                "SELECT MAX(day) FROM payroll_runs WHERE company_id = ?1 AND kind = 'taeglich'",
                [company_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let day = match last_day {
            Some(d) => d + 1,
            None => local_day(from_ts),
        };
        if now < local_to_utc(day + 1, minute) {
            return Ok(done);
        }
        let from = last_period_end(&tx, company_id)?;
        let pid = run_period(
            &tx,
            company_id,
            day,
            "taeglich",
            from,
            day_start(day + 1),
            now,
        )?;
        tx.commit()?;
        done.push(pid);
    }
}

/// Fällige Abrechnungen aller Firmen. Läuft im Hintergrund des Servers.
pub fn run_due_all(conn: &mut Connection, now: i64) -> rusqlite::Result<usize> {
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM companies ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut n = 0;
    for id in ids {
        n += run_due(conn, id, now)?.len();
    }
    Ok(n)
}

/// Abrechnung sofort, für den Testbereich im Dashboard: erst die fälligen Tage, dann alles seit
/// der letzten Abrechnung bis jetzt. Die nächste tägliche Abrechnung macht danach dort weiter.
pub fn run_now(conn: &mut Connection, company_id: i64, now: i64) -> rusqlite::Result<Vec<i64>> {
    let mut ids = run_due(conn, company_id, now)?;
    let tx = conn.transaction()?;
    let from = last_period_end(&tx, company_id)?;
    ids.push(run_period(
        &tx,
        company_id,
        local_day(now),
        "hand",
        from,
        now + 1,
        now,
    )?);
    tx.commit()?;
    Ok(ids)
}

/// Eine Abrechnung mit allen Zeilen, für API und Dashboard.
pub fn run_json(conn: &Connection, pid: i64) -> rusqlite::Result<Value> {
    let (day, kind, from, to, created_at): (i64, String, i64, i64, i64) = conn.query_row(
        "SELECT day, kind, period_from, period_to, created_at FROM payroll_runs WHERE id = ?1",
        [pid],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let lines: Vec<Value> = conn
        .prepare(
            "SELECT p.user_id, u.name, p.km, p.stage, p.cents_per_km, p.flat_days, p.flat, p.wage,
                    p.claim_before, p.paid, p.claim_after, e.id, r.id, r.created_at, r.note,
                    ru.name
             FROM payroll_lines p JOIN users u ON u.id = p.user_id
             LEFT JOIN ledger e ON e.payroll_id = p.payroll_id AND e.account = 'fahrer'
                              AND e.user_id = p.user_id AND e.kind = 'lohn'
             LEFT JOIN ledger r ON r.reverses_id = e.id AND r.account = 'fahrer'
             LEFT JOIN users ru ON ru.id = r.booked_by
             WHERE p.payroll_id = ?1 ORDER BY u.name",
        )?
        .query_map([pid], |r| {
            Ok(json!({
                "user_id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?,
                "km": r.get::<_, i64>(2)?, "stage": r.get::<_, Option<i64>>(3)?,
                "cents_per_km": r.get::<_, i64>(4)?, "flat_days": r.get::<_, i64>(5)?,
                "flat": r.get::<_, i64>(6)?, "wage": r.get::<_, i64>(7)?,
                "claim_before": r.get::<_, i64>(8)?, "paid": r.get::<_, i64>(9)?,
                "claim_after": r.get::<_, i64>(10)?,
                // Die Lohnbuchung auf dem Lohnkonto, leer, wenn nichts gezahlt wurde, und ihr Storno
                "entry_id": r.get::<_, Option<i64>>(11)?,
                "reversed_by": r.get::<_, Option<i64>>(12)?,
                "reversed_at": r.get::<_, Option<i64>>(13)?,
                "reverse_reason": r.get::<_, Option<String>>(14)?,
                "reversed_by_user": r.get::<_, Option<String>>(15)?
            }))
        })?
        .collect::<Result<_, _>>()?;
    let paid: i64 = lines.iter().map(|l| l["paid"].as_i64().unwrap_or(0)).sum();
    Ok(json!({
        "id": pid, "day": format_day(day), "kind": kind, "period_from": from, "period_to": to,
        "created_at": created_at, "paid": paid, "lines": lines
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anteile() {
        assert_eq!(share(&[100, 50], 500), vec![100, 50]);
        assert_eq!(share(&[100, 50], 150), vec![100, 50]);
        // 60 von 150: jeder 40 Prozent
        assert_eq!(share(&[100, 50], 60), vec![40, 20]);
        assert_eq!(share(&[100, 50], 0), vec![0, 0]);
        assert_eq!(share(&[100, 50], -10), vec![0, 0]);
        assert_eq!(share(&[], 10), Vec::<i64>::new());
        // Abgerundet, nie mehr als da ist
        assert_eq!(share(&[1, 1, 1], 2), vec![0, 0, 0]);
        assert_eq!(share(&[10, 10, 10], 20), vec![6, 6, 6]);
    }

    #[test]
    fn cent_zu_euro() {
        assert_eq!(euro_from_cents(20_920), 209);
        assert_eq!(euro_from_cents(20_950), 210);
        assert_eq!(euro_from_cents(0), 0);
    }
}
