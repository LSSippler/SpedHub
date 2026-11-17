//! Kennzeichen. Zwei Formen kommen vor:
//!
//! - vom Spiel vergeben, mit Markup:
//!   `DD<offset hshift=4 vshift=-5><img src=...><offset hshift=4 vshift=5>RI 772|germany`
//! - selbst vergeben, schlicht: `SPED 001|_germany`
//!
//! Gemeint ist hier immer der Inhalt der Zeichenkette, also ohne Anführungszeichen. Hinter
//! dem letzten `|` steht das Land, bei selbst vergebenen Kennzeichen mit Unterstrich davor.

use crate::error::{Error, Result};
use crate::value::safe_string_content;

/// Der lesbare Text ohne Markup und ohne Land, zum Beispiel `DD RI 772` oder `SPED 001`.
pub fn plate_text(plate: &str) -> String {
    let text = plate.rsplit_once('|').map_or(plate, |(t, _)| t);
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Das Land ohne Unterstrich, zum Beispiel `germany`. Leer, wenn keins dabei ist.
pub fn plate_country(plate: &str) -> &str {
    plate
        .rsplit_once('|')
        .map_or("", |(_, c)| c.trim_start_matches('_'))
}

/// Ist das ein selbst vergebenes Kennzeichen (Land mit Unterstrich)?
pub fn is_custom_plate(plate: &str) -> bool {
    plate
        .rsplit_once('|')
        .is_some_and(|(_, c)| c.starts_with('_'))
}

/// Vergleicht ein Kennzeichen mit einer Eingabe wie `SPED 001` oder `dd ri 772`. Groß- und
/// Kleinschreibung, Markup und ein angehängtes Land spielen keine Rolle.
pub fn plate_matches(plate: &str, query: &str) -> bool {
    let a = plate_text(plate).to_uppercase();
    !a.is_empty() && a == plate_text(query).to_uppercase()
}

/// Baut ein selbst vergebenes Kennzeichen, zum Beispiel `SPED 001|_germany`.
pub fn custom_plate(text: &str, country: &str) -> Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || text.chars().count() > 16 {
        return Err(Error::InvalidInput(
            "Das Kennzeichen braucht 1 bis 16 Zeichen".into(),
        ));
    }
    if !text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '.'))
    {
        return Err(Error::InvalidInput(
            "Im Kennzeichen sind nur Buchstaben A bis Z, Ziffern, Leerzeichen, Punkt und Bindestrich erlaubt".into(),
        ));
    }
    let country = country.trim_start_matches('_');
    if country.is_empty()
        || !country
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(Error::InvalidInput(
            "Das Land muss klein geschrieben sein, zum Beispiel germany".into(),
        ));
    }
    Ok(format!("{}|_{country}", text.to_uppercase()))
}

/// Prüft ein Kennzeichen aus einem Paket, bevor es in den Spielstand geht.
pub(crate) fn validate_plate(plate: &str) -> Result<()> {
    if plate_text(plate).is_empty() || !safe_string_content(plate) {
        return Err(Error::InvalidPackage(format!(
            "das Kennzeichen {plate:?} ist leer oder enthält unerlaubte Zeichen"
        )));
    }
    Ok(())
}
