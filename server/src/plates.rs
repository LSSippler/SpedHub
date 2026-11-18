//! Kennzeichen der Firmen-Lkw: prüfen, vorschlagen, auf Doppelte prüfen.
//!
//! Jeder Firmen-Lkw hat ein eigenes Kennzeichen, das der Launcher beim Einsetzen als selbst
//! vergebenes Kennzeichen in den Spielstand schreibt (`<TEXT>|_<land>`). Deshalb gelten
//! dieselben Regeln wie beim Setzen in `spedhub-save`: Buchstaben A bis Z, Ziffern,
//! Leerzeichen, Punkt und Bindestrich, 1 bis 16 Zeichen. Kleinbuchstaben werden groß.
//!
//! Fehlt beim Anlegen ein Kennzeichen, schlägt der Server eins vor: das Kürzel der Firma
//! (siehe [`company_prefix`]) und die kleinste dreistellige Nummer, die in der Firma noch nie
//! vergeben war, auch nicht als früheres Kennzeichen und nicht bei ausgemusterten Lkw.
//! Beispiel: Firma "Nordfracht Logistik", vergeben sind `NL 001` und früher `NL 002`, dann
//! ist der Vorschlag `NL 003`.

use crate::presence::plate_key;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;

/// Kürzel, wenn der Firmenname keine zwei Buchstaben hergibt.
pub const FALLBACK_PREFIX: &str = "SH";

/// Ein Kennzeichen für einen Firmen-Lkw, so wie es gespeichert wird: Leerraum zusammengefasst,
/// groß geschrieben. Erlaubt ist, was `spedhub_save::custom_plate` beim Setzen erlaubt. Sonst
/// eine Meldung für die Antwort 400.
pub fn company_plate(raw: &str) -> Result<String, String> {
    // Das Land spielt für die Prüfung keine Rolle, es kommt beim Einsetzen aus dem Bauplan
    spedhub_save::custom_plate(raw, "germany")
        .map(|p| spedhub_save::plate_text(&p))
        .map_err(|e| e.to_string())
}

/// Ein Buchstabe für das Kürzel: A bis Z, Umlaute ohne Punkte, sonst nichts.
fn letter(c: char) -> Option<char> {
    match c {
        'a'..='z' | 'A'..='Z' => Some(c.to_ascii_uppercase()),
        'ä' | 'Ä' => Some('A'),
        'ö' | 'Ö' => Some('O'),
        'ü' | 'Ü' => Some('U'),
        'ß' => Some('S'),
        _ => None,
    }
}

/// Kürzel der Firma für Kennzeichen: 2 bis 3 Großbuchstaben A bis Z.
///
/// - Die Anfangsbuchstaben der Wörter, höchstens drei: "Nordfracht Logistik" wird `NL`,
///   "Spedition Müller und Söhne" wird `SMU`.
/// - Hat der Name nur ein Wort mit Buchstaben, dessen erste drei: "Nordfracht" wird `NOR`.
/// - Gibt der Name keine zwei Buchstaben her, [`FALLBACK_PREFIX`].
pub fn company_prefix(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.chars().filter_map(letter).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
    let prefix: String = if initials.len() >= 2 {
        initials.chars().take(3).collect()
    } else {
        words.concat().chars().take(3).collect()
    };
    if prefix.len() >= 2 {
        prefix
    } else {
        FALLBACK_PREFIX.to_string()
    }
}

/// Alle Kennzeichen, die in der Firma schon vergeben sind oder waren, als [`plate_key`]:
/// aktuelle und frühere, auch von ausgemusterten und verkauften Lkw.
fn used_plates(conn: &Connection, company_id: i64) -> rusqlite::Result<HashSet<String>> {
    let mut used = HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT plate FROM trucks WHERE company_id = ?1
         UNION ALL
         SELECT p.plate FROM truck_plates p JOIN trucks t ON t.id = p.truck_id
         WHERE t.company_id = ?1",
    )?;
    for p in stmt.query_map([company_id], |r| r.get::<_, String>(0))? {
        used.insert(plate_key(&p?));
    }
    Ok(used)
}

/// Vorschlag für das Kennzeichen eines neuen Firmen-Lkw: Kürzel der Firma und die kleinste
/// Nummer ab 001, die in der Firma noch nie vergeben war.
pub fn suggest(conn: &Connection, company_id: i64) -> rusqlite::Result<String> {
    let name: String = conn.query_row(
        "SELECT name FROM companies WHERE id = ?1",
        [company_id],
        |r| r.get(0),
    )?;
    let prefix = company_prefix(&name);
    let used = used_plates(conn, company_id)?;
    // Nach 999 geht es vierstellig weiter, das passt noch in 16 Zeichen
    let plate = (1..)
        .map(|n: u32| format!("{prefix} {n:03}"))
        .find(|p| !used.contains(&plate_key(p)))
        .unwrap_or_else(|| format!("{prefix} 001"));
    Ok(plate)
}

/// Der aktive Lkw der Firma (nicht ausgemustert, nicht verkauft), der das Kennzeichen gerade
/// trägt, ohne Rücksicht auf Schreibweise. `except` ist ein Lkw, der nicht zählt.
pub fn active_holder(
    conn: &Connection,
    company_id: i64,
    plate: &str,
    except: Option<i64>,
) -> rusqlite::Result<Option<i64>> {
    let key = plate_key(plate);
    let mut stmt = conn.prepare(
        "SELECT id, plate FROM trucks WHERE company_id = ?1 AND retired_at IS NULL
         AND (?2 IS NULL OR id <> ?2)",
    )?;
    let rows = stmt.query_map(params![company_id, except], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, p) = row?;
        if plate_key(&p) == key {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Firma eines Lkw, `None`, wenn es ihn nicht gibt.
pub fn truck_company(conn: &Connection, truck_id: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT company_id FROM trucks WHERE id = ?1",
        [truck_id],
        |r| r.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kuerzel_aus_dem_firmennamen() {
        assert_eq!(company_prefix("Nordfracht Logistik"), "NL");
        assert_eq!(company_prefix("Nordfracht"), "NOR");
        assert_eq!(company_prefix("Spedition Müller und Söhne"), "SMU");
        assert_eq!(company_prefix("Übersee-Transporte"), "UT");
        assert_eq!(company_prefix("ab"), "AB");
        assert_eq!(company_prefix("A 1 B"), "AB");
        assert_eq!(company_prefix("X 99"), FALLBACK_PREFIX);
        assert_eq!(company_prefix("123"), FALLBACK_PREFIX);
        assert_eq!(company_prefix("Ölöl"), "OLO");
        for name in [
            "Nordfracht Logistik",
            "Spedition Müller und Söhne",
            "Ölöl",
            "X 99",
        ] {
            let p = company_prefix(name);
            assert!((2..=3).contains(&p.len()), "{name}: {p}");
            assert!(p.chars().all(|c| c.is_ascii_uppercase()), "{name}: {p}");
        }
    }

    #[test]
    fn kennzeichen_pruefen() {
        assert_eq!(company_plate(" nl   001 ").unwrap(), "NL 001");
        assert_eq!(company_plate("A-1.B").unwrap(), "A-1.B");
        assert_eq!(
            company_plate("X".repeat(16).as_str()).unwrap(),
            "X".repeat(16)
        );
        for bad in [
            "",
            "  ",
            "X".repeat(17).as_str(),
            "MÜNCHEN 1",
            "SPED 001|_germany",
            "A<b>",
        ] {
            assert!(company_plate(bad).is_err(), "{bad:?}");
        }
    }
}
