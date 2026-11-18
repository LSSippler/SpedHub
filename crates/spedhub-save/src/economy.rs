//! Wirtschafts-Mod: Aufträge zahlen im Spiel nur einen Bruchteil, der Server rechnet mit dem
//! Erlösfaktor zurück.
//!
//! Die Originaldatei `def/economy_data.sii` gehört SCS und liegt nicht im Repo. Der Nutzer
//! entpackt sie selbst aus `def.scs`, [`scale_economy`] macht daraus die geänderte Fassung und
//! [`build_mod`] die Mod `spedhub_economy.scs` (ein ZIP mit `manifest.sii`,
//! `description.txt`, `def/economy_data.sii` und wahlweise einem Vorschaubild). Anleitung in
//! `docs/wirtschafts-mod.md`.
//!
//! Bearbeitet wird textuell und minimal: Die Ausgabe ist Byte für Byte die Eingabe, nur die
//! Zahlen der skalierten Werte sind ersetzt. Zeilenenden, Einrückung, Kommentare, Reihenfolge,
//! `@include`-Zeilen und unbekannte Units bleiben, wie sie sind. Es gibt bewusst keinen Parser,
//! der die Datei neu schreibt.
//!
//! Zahlen im SII-Format:
//!
//! - Dezimal mit Punkt, wahlweise mit Vorzeichen und Exponent: `12`, `1.5`, `-0.25`, `2e3`. Das
//!   Ergebnis ist wieder eine Dezimalzahl, ohne Exponent und exakt gerechnet (`13.5` mal `0.01`
//!   ergibt `0.135`). Eine ganze Zahl bleibt ganz, wenn das Ergebnis ganz ist (`600` wird `6`),
//!   sonst wird sie zur Kommazahl (`650` wird `6.5`). Das meldet [`Change::became_fraction`].
//! - Hex mit `&` davor, die Bits eines 32-Bit-Floats: `&3f800000`. Das Ergebnis bleibt Hex, auf
//!   den nächsten 32-Bit-Float gerundet.
//! - Vektoren in Klammern, `(1, 2, 3)` oder `(1; 2, 3, 4)`: Jede Komponente wird skaliert.
//!
//! Eine Liste steht als `schluessel: N` und `schluessel[0]: ...` oder als `schluessel[]: ...`
//! in der Datei. Skaliert werden die Einträge, nie die Zählzeile.

use crate::error::{Error, Result};
use crate::layout::economy_data::{CANDIDATE_WORDS, REVENUE_KEYS};
use crate::write::{sibling, write_and_verify};
use crate::zip::{ZipEntry, read_stored, write_stored};
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};

/// Name der Mod-Datei, die `spedhub-save economy-mod` ohne `-o` schreibt.
pub const MOD_FILE_NAME: &str = "spedhub_economy.scs";
/// Pfad der geänderten Datei in der Mod. Sie ersetzt die gleichnamige Datei aus `def.scs`.
pub const SII_PATH: &str = "def/economy_data.sii";
pub const MANIFEST_PATH: &str = "manifest.sii";
pub const DESCRIPTION_PATH: &str = "description.txt";
/// Vorschaubild, nur wenn eins übergeben wird. Laut SCS-Wiki ein JPG mit 276 x 162 Pixeln.
pub const ICON_PATH: &str = "icon.jpg";
pub const DISPLAY_NAME: &str = "SpedHub Wirtschaft";
pub const AUTHOR: &str = "SpedHub";
/// Kategorie im Mod-Manager. Eine Kategorie für Wirtschaft gibt es nicht, siehe
/// `docs/wirtschafts-mod.md`.
pub const CATEGORY: &str = "other";
pub const DEFAULT_FACTOR: &str = "0.01";
pub const DEFAULT_COMPATIBLE: &str = "1.62.*";
pub const REPO_URL: &str = "https://github.com/LSSippler/SpedHub";
/// Kleinster und größter erlaubter Faktor. Das Dashboard nimmt Erlösfaktoren von 1 bis 10000.
pub const MIN_FACTOR: f64 = 0.0001;
pub const MAX_FACTOR: f64 = 1.0;

/// Kurze Anleitung, wie man an die Datei kommt. Die Kommandozeile zeigt sie, wenn die Datei
/// fehlt.
pub const GUIDE: &str = "So kommst du an economy_data.sii:
  1. Den SCS Extractor (Game Archive Extractor) von modding.scssoft.com laden.
  2. def.scs aus dem Spielordner entpacken, zum Beispiel:
       scs_extractor.exe \"C:\\Program Files (x86)\\Steam\\steamapps\\common\\Euro Truck Simulator 2\\def.scs\" C:\\SpedHub\\entpackt
  3. Gebraucht wird nur def\\economy_data.sii, im Beispiel C:\\SpedHub\\entpackt\\def\\economy_data.sii.
  4. Die Mod bauen:
       spedhub-save economy-mod C:\\SpedHub\\entpackt\\def\\economy_data.sii
Die ganze Anleitung steht in docs/wirtschafts-mod.md.";

// ---------------------------------------------------------------------------------------------
// Zahlen

/// Eine Dezimalzahl `mantissa * 10^exponent`, exakt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Decimal {
    negative: bool,
    mantissa: u128,
    exponent: i32,
}

impl Decimal {
    /// `12`, `-1.5`, `.25`, `3.`, `2e-3`, `+1E2`. Höchstens 30 Ziffern.
    fn parse(s: &str) -> Option<Decimal> {
        let (negative, rest) = if let Some(r) = s.strip_prefix('-') {
            (true, r)
        } else if let Some(r) = s.strip_prefix('+') {
            (false, r)
        } else {
            (false, s)
        };
        let (number, exp) = match rest.split_once(['e', 'E']) {
            Some((n, e)) => {
                let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                (n, e.parse::<i32>().ok()?)
            }
            None => (rest, 0),
        };
        let (int, frac) = number.split_once('.').unwrap_or((number, ""));
        if int.len() + frac.len() == 0
            || int.len() + frac.len() > 30
            || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let mantissa: u128 = format!("{int}{frac}").parse().ok()?;
        let frac_len = i32::try_from(frac.len()).ok()?;
        Some(
            Decimal {
                negative,
                mantissa,
                exponent: exp.checked_sub(frac_len)?,
            }
            .normalized(),
        )
    }

    /// Ohne Nullen am Ende der Mantisse, Null ohne Vorzeichen.
    fn normalized(mut self) -> Decimal {
        if self.mantissa == 0 {
            return Decimal {
                negative: false,
                mantissa: 0,
                exponent: 0,
            };
        }
        while self.mantissa.is_multiple_of(10) {
            self.mantissa /= 10;
            self.exponent += 1;
        }
        self
    }

    fn mul(self, other: Decimal) -> Option<Decimal> {
        Some(
            Decimal {
                negative: self.negative != other.negative,
                mantissa: self.mantissa.checked_mul(other.mantissa)?,
                exponent: self.exponent.checked_add(other.exponent)?,
            }
            .normalized(),
        )
    }

    fn is_integer(self) -> bool {
        self.exponent >= 0
    }

    /// Als Dezimalzahl ohne Exponent, zum Beispiel `0.135` oder `600`.
    fn plain(self) -> Option<String> {
        let d = self.normalized();
        let digits = d.mantissa.to_string();
        let body = if d.exponent >= 0 {
            if d.exponent > 40 {
                return None;
            }
            format!("{digits}{}", "0".repeat(d.exponent.unsigned_abs() as usize))
        } else {
            let k = d.exponent.unsigned_abs() as usize;
            if k > 60 {
                return None;
            }
            if digits.len() > k {
                let (a, b) = digits.split_at(digits.len() - k);
                format!("{a}.{b}")
            } else {
                format!("0.{}{digits}", "0".repeat(k - digits.len()))
            }
        };
        Some(if d.negative { format!("-{body}") } else { body })
    }

    fn to_f64(self) -> Option<f64> {
        self.plain()?.parse().ok()
    }
}

/// Der Faktor, mit dem die Erlös-Werte multipliziert werden, Standard 0.01.
#[derive(Debug, Clone, PartialEq)]
pub struct Factor {
    decimal: Decimal,
    value: f64,
}

impl Factor {
    /// Liest den Faktor als Dezimalzahl (`0.01`, `1e-2`). Erlaubt ist 0.0001 bis 1.
    pub fn parse(s: &str) -> Result<Factor> {
        let bad = || {
            Error::InvalidInput(format!(
                "{s} ist kein gültiger Faktor. Erwartet wird eine Dezimalzahl mit Punkt zwischen 0.0001 und 1, zum Beispiel 0.01"
            ))
        };
        let decimal = Decimal::parse(s.trim()).ok_or_else(bad)?;
        let value = decimal.to_f64().ok_or_else(bad)?;
        if decimal.negative || !(MIN_FACTOR..=MAX_FACTOR).contains(&value) {
            return Err(bad());
        }
        Ok(Factor { decimal, value })
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    /// Der Faktor als Dezimalzahl, zum Beispiel `0.01`.
    pub fn text(&self) -> String {
        self.decimal.plain().unwrap_or_default()
    }

    /// Der Faktor in Prozent, zum Beispiel `1` für 0.01.
    pub fn percent(&self) -> String {
        self.decimal
            .mul(Decimal {
                negative: false,
                mantissa: 100,
                exponent: 0,
            })
            .and_then(Decimal::plain)
            .unwrap_or_default()
    }

    /// Der Erlösfaktor, der im Dashboard dazu passt: 1 durch den Faktor, also 100 für 0.01.
    /// Geht die Division nicht auf, auf vier Stellen gerundet mit "etwa" davor.
    pub fn revenue_factor(&self) -> String {
        let d = self.decimal;
        // d = m * 10^e mit e <= 0, weil der Faktor höchstens 1 ist. 1/d = 10^-e / m.
        let shift = d.exponent.unsigned_abs();
        if d.exponent <= 0
            && let Some(p) = 10u128.checked_pow(shift)
            && d.mantissa != 0
            && p.is_multiple_of(d.mantissa)
        {
            return (p / d.mantissa).to_string();
        }
        let s = format!("{:.4}", 1.0 / self.value);
        format!("etwa {}", s.trim_end_matches('0').trim_end_matches('.'))
    }
}

/// Ist das eine Zahl im SII-Format? Gibt die Art zurück.
fn number_kind(token: &str) -> Option<NumberKind> {
    if let Some(hex) = token.strip_prefix('&') {
        return (hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then_some(NumberKind::Hex);
    }
    Decimal::parse(token).map(|_| NumberKind::Decimal)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumberKind {
    Decimal,
    Hex,
}

/// Wert einer Zahl, für die Prüfung nach dem Ersetzen.
fn number_value(token: &str) -> Option<f64> {
    match number_kind(token)? {
        NumberKind::Hex => {
            let bits = u32::from_str_radix(token.get(1..)?, 16).ok()?;
            Some(f64::from(f32::from_bits(bits)))
        }
        NumberKind::Decimal => Decimal::parse(token)?.to_f64(),
    }
}

/// Multipliziert eine Zahl. Gibt die neue Zahl zurück und ob aus einer ganzen Zahl eine
/// Kommazahl wurde. Fehler, wenn das Ergebnis im Spiel nicht darstellbar wäre.
fn scale_number(token: &str, factor: &Factor) -> std::result::Result<(String, bool), String> {
    match number_kind(token) {
        Some(NumberKind::Hex) => {
            let bits = token
                .get(1..)
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .ok_or_else(|| format!("{token} ist keine Zahl"))?;
            let old = f32::from_bits(bits);
            if !old.is_finite() {
                return Err(format!("{token} ist keine endliche Zahl"));
            }
            let new = (f64::from(old) * factor.value) as f32;
            if !new.is_finite() || (old != 0.0 && new == 0.0) {
                return Err(format!(
                    "{token} mal {} lässt sich nicht als 32-Bit-Float schreiben",
                    factor.text()
                ));
            }
            let upper = token.bytes().any(|b| b.is_ascii_uppercase());
            let text = if upper {
                format!("&{:08X}", new.to_bits())
            } else {
                format!("&{:08x}", new.to_bits())
            };
            Ok((text, false))
        }
        Some(NumberKind::Decimal) => {
            let old = Decimal::parse(token).ok_or_else(|| format!("{token} ist keine Zahl"))?;
            let new = old
                .mul(factor.decimal)
                .ok_or_else(|| format!("{token} hat zu viele Stellen"))?;
            let text = new
                .plain()
                .ok_or_else(|| format!("{token} mal {} ist zu groß", factor.text()))?;
            let as_f32: f32 = text.parse().map_err(|_| format!("{text} ist keine Zahl"))?;
            if !as_f32.is_finite() || (old.mantissa != 0 && as_f32 == 0.0) {
                return Err(format!(
                    "{token} mal {} lässt sich nicht als 32-Bit-Float schreiben",
                    factor.text()
                ));
            }
            let was_integer = !token.contains(['.', 'e', 'E']);
            Ok((text, was_integer && !new.is_integer()))
        }
        None => Err(format!("{token} ist keine Zahl")),
    }
}

// ---------------------------------------------------------------------------------------------
// Datei lesen

/// Ein Feld `schluessel: wert` oder `schluessel[i]: wert` in einer Unit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawField {
    /// Zeile, ab 1
    line: usize,
    /// Kopf der Unit, zum Beispiel `economy_data : economy.data`
    unit: String,
    /// Schlüssel wie in der Datei, zum Beispiel `fixed_revenue` oder `xp[2]`
    key: String,
    /// Schlüssel ohne Index
    base: String,
    indexed: bool,
    /// Wert ohne Leerzeichen und Kommentar, als Byte-Bereich der Datei
    start: usize,
    end: usize,
}

/// Eine Zeile, die nicht verstanden und deshalb nicht verändert wurde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub line: usize,
    pub text: String,
}

struct Scan {
    fields: Vec<RawField>,
    includes: Vec<Note>,
    skipped: Vec<Note>,
}

/// Prüft den Anfang der Datei. Klartext (`SiiNunit`, auch mit UTF-8-BOM davor) geht weiter,
/// alles andere bricht mit einer Meldung ab.
fn check_text_format(data: &[u8]) -> Result<()> {
    let body = data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(data);
    let start = body
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(body.len());
    let head = body.get(start..).unwrap_or_default();
    if head.starts_with(b"SiiNunit") {
        return Ok(());
    }
    let what = if data.starts_with(b"ScsC") {
        "verschlüsselt (ScsC)"
    } else if data.starts_with(b"BSII") {
        "binär (BSII)"
    } else if data.starts_with(b"3nK") {
        "mit 3nK kodiert"
    } else {
        "keine SII-Datei im Textformat, SiiNunit fehlt am Anfang"
    };
    Err(Error::InvalidInput(format!(
        "Die Datei ist {what}. Aus def.scs entpackt ist economy_data.sii Klartext und beginnt mit SiiNunit. \
         Bitte die Datei mit dem SCS Extractor aus def.scs holen, siehe docs/wirtschafts-mod.md"
    )))
}

/// Ersetzt Kommentare (`#`, `//`, `/* */`) durch Leerzeichen. Zeilenumbrüche bleiben, die
/// Länge auch, Positionen passen also weiter zur Datei.
fn mask_comments(data: &[u8]) -> Vec<u8> {
    #[derive(PartialEq)]
    enum S {
        Code,
        Str,
        Line,
        Block,
    }
    let mut out = data.to_vec();
    let mut state = S::Code;
    let mut i = 0;
    while let Some(&b) = data.get(i) {
        let next = data.get(i + 1).copied();
        match state {
            S::Code => match b {
                b'"' => state = S::Str,
                b'#' => state = S::Line,
                b'/' if next == Some(b'/') => state = S::Line,
                b'/' if next == Some(b'*') => {
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    i += 2;
                    state = S::Block;
                    continue;
                }
                _ => {}
            },
            S::Str => match b {
                b'\\' if next.is_some_and(|n| n != b'\n') => {
                    i += 2;
                    continue;
                }
                b'"' | b'\n' => state = S::Code,
                _ => {}
            },
            S::Line => {
                if b == b'\n' {
                    state = S::Code;
                }
            }
            S::Block => {
                if b == b'*' && next == Some(b'/') {
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    i += 2;
                    state = S::Code;
                    continue;
                }
            }
        }
        if matches!(state, S::Line | S::Block) {
            blank(&mut out, i);
        }
        i += 1;
    }
    out
}

fn blank(out: &mut [u8], i: usize) {
    if let Some(b) = out.get_mut(i)
        && *b != b'\n'
        && *b != b'\r'
    {
        *b = b' ';
    }
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c')
}

/// Bereich ohne Leerzeichen am Anfang und Ende.
fn trim_range(line: &[u8], mut a: usize, mut b: usize) -> (usize, usize) {
    while a < b && line.get(a).copied().is_some_and(is_space) {
        a += 1;
    }
    while b > a && line.get(b - 1).copied().is_some_and(is_space) {
        b -= 1;
    }
    (a, b)
}

fn text_of(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `schluessel: wert` mit optionalem Index `[3]` oder `[]`. Gibt Schlüssel, Basis, ob mit
/// Index und den Bereich des Werts in der Zeile zurück.
fn parse_field(line: &[u8]) -> Option<(String, String, bool, usize, usize)> {
    let mut i = 0;
    while line.get(i).copied().is_some_and(is_space) {
        i += 1;
    }
    let key_start = i;
    while line
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        i += 1;
    }
    if i == key_start {
        return None;
    }
    let base_end = i;
    let mut indexed = false;
    if line.get(i) == Some(&b'[') {
        i += 1;
        while line.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if line.get(i) != Some(&b']') {
            return None;
        }
        i += 1;
        indexed = true;
    }
    let key_end = i;
    while line.get(i).is_some_and(|b| *b == b' ' || *b == b'\t') {
        i += 1;
    }
    if line.get(i) != Some(&b':') {
        return None;
    }
    let (a, b) = trim_range(line, i + 1, line.len());
    Some((
        text_of(line.get(key_start..key_end)?),
        text_of(line.get(key_start..base_end)?),
        indexed,
        a,
        b,
    ))
}

/// Zählt `{` und `}` außerhalb von Zeichenketten.
fn braces(line: &[u8]) -> (usize, usize, Option<usize>) {
    let (mut open, mut close, mut first_open) = (0, 0, None);
    let mut in_str = false;
    let mut i = 0;
    while let Some(&b) = line.get(i) {
        match b {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b'{' if !in_str => {
                open += 1;
                first_open.get_or_insert(i);
            }
            b'}' if !in_str => close += 1,
            _ => {}
        }
        i += 1;
    }
    (open, close, first_open)
}

/// Liest alle Felder, `@include`-Zeilen und Zeilen, die nicht verstanden wurden.
fn scan(data: &[u8]) -> Result<Scan> {
    check_text_format(data)?;
    let masked = mask_comments(data);
    let mut scan = Scan {
        fields: Vec::new(),
        includes: Vec::new(),
        skipped: Vec::new(),
    };
    let mut depth = 0usize;
    let mut pending = String::new();
    let mut unit = String::new();
    let mut offset = 0usize;
    for (i, raw) in masked.split_inclusive(|b| *b == b'\n').enumerate() {
        let line_no = i + 1;
        let start = offset;
        offset += raw.len();
        let line = raw.strip_suffix(b"\n").unwrap_or(raw);
        let (a, b) = trim_range(line, 0, line.len());
        let trimmed = line.get(a..b).unwrap_or_default();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with(b"@include") {
            scan.includes.push(Note {
                line: line_no,
                text: text_of(trimmed),
            });
            continue;
        }
        let (open, close, first_open) = braces(line);
        if depth == 2 && !trimmed.starts_with(b"}") {
            match parse_field(line) {
                Some((key, base, indexed, va, vb)) => scan.fields.push(RawField {
                    line: line_no,
                    unit: unit.clone(),
                    key,
                    base,
                    indexed,
                    start: start + va,
                    end: start + vb,
                }),
                None => scan.skipped.push(Note {
                    line: line_no,
                    text: text_of(trimmed),
                }),
            }
        }
        if depth == 1 {
            // Kopf einer Unit: `typ : name {` oder `typ : name` und `{` in der nächsten Zeile
            let before = match first_open {
                Some(p) => line.get(..p).unwrap_or_default(),
                None => line,
            };
            let (ha, hb) = trim_range(before, 0, before.len());
            let head = text_of(before.get(ha..hb).unwrap_or_default());
            if !head.is_empty() {
                pending = head.split_whitespace().collect::<Vec<_>>().join(" ");
            }
        }
        if open > 0 && depth == 1 {
            unit = std::mem::take(&mut pending);
        }
        depth += open;
        depth = depth.checked_sub(close).ok_or_else(|| Error::Parse {
            line: line_no,
            message: "hier wird eine Klammer geschlossen, die nie geöffnet wurde".into(),
        })?;
        if depth > 2 {
            return Err(Error::Parse {
                line: line_no,
                message: "Klammern sind tiefer verschachtelt als in einer SII-Datei erwartet"
                    .into(),
            });
        }
    }
    if depth != 0 {
        return Err(Error::Parse {
            line: masked.split(|b| *b == b'\n').count(),
            message: "am Ende der Datei sind noch Klammern offen".into(),
        });
    }
    Ok(scan)
}

/// Bereiche der Zahlen in einem Wert: eine Zahl oder ein Vektor in Klammern.
fn number_ranges(value: &[u8]) -> Option<Vec<(usize, usize)>> {
    let is_number = |a: usize, b: usize| {
        value
            .get(a..b)
            .and_then(|t| std::str::from_utf8(t).ok())
            .and_then(number_kind)
            .is_some()
    };
    if value.first() == Some(&b'(') && value.last() == Some(&b')') && value.len() >= 2 {
        let mut ranges = Vec::new();
        let mut a = 1;
        for i in 1..value.len() {
            if matches!(value.get(i), Some(b',' | b';' | b')')) {
                let (ta, tb) = trim_range(value, a, i);
                if !is_number(ta, tb) {
                    return None;
                }
                ranges.push((ta, tb));
                a = i + 1;
            }
        }
        return Some(ranges);
    }
    is_number(0, value.len()).then(|| vec![(0, value.len())])
}

// ---------------------------------------------------------------------------------------------
// Skalieren

/// Was skaliert wird.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleOptions {
    pub factor: Factor,
    /// Weitere Schlüssel (ohne Index), die zusätzlich zu [`REVENUE_KEYS`] skaliert werden
    pub also: Vec<String>,
}

impl Default for ScaleOptions {
    fn default() -> Self {
        ScaleOptions {
            factor: Factor {
                decimal: Decimal {
                    negative: false,
                    mantissa: 1,
                    exponent: -2,
                },
                value: 0.01,
            },
            also: Vec::new(),
        }
    }
}

/// Eine geänderte Zahl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub line: usize,
    pub unit: String,
    /// Schlüssel wie in der Datei, mit Index
    pub key: String,
    /// Bei einem Vektor die Komponente, ab 0
    pub component: Option<usize>,
    pub old: String,
    pub new: String,
    /// Aus einer ganzen Zahl wurde eine Kommazahl. Ob das Spiel an der Stelle eine Kommazahl
    /// annimmt, ist ungeprüft.
    pub became_fraction: bool,
}

/// Ein Schlüssel, der nach Geld für Aufträge klingt, aber nicht skaliert wurde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub line: usize,
    pub unit: String,
    pub key: String,
    pub value: String,
}

/// Ergebnis von [`scale_economy`].
#[derive(Debug, Clone, PartialEq)]
pub struct EconomyEdit {
    /// Die geänderte Datei
    pub text: Vec<u8>,
    pub changes: Vec<Change>,
    /// Bekannte Schlüssel, die in der Datei nicht vorkommen
    pub missing: Vec<String>,
    /// Nicht verändert, bitte prüfen. Mit `--also` lassen sie sich mitskalieren.
    pub candidates: Vec<Candidate>,
    /// `@include`-Zeilen. Was in den eingebundenen Dateien steht, ist nicht geprüft und nicht
    /// verändert.
    pub includes: Vec<Note>,
    /// Zeilen in einer Unit, die kein Feld waren. Sie sind unverändert.
    pub skipped: Vec<Note>,
    pub factor: Factor,
}

impl EconomyEdit {
    /// Die Schlüssel (ohne Index), die geändert wurden, in Dateireihenfolge, jeder einmal.
    pub fn changed_keys(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.changes
            .iter()
            .map(|c| base_of(&c.key).to_string())
            .filter(|k| seen.insert(k.clone()))
            .collect()
    }
}

fn base_of(key: &str) -> &str {
    key.split_once('[').map_or(key, |(b, _)| b)
}

fn valid_key_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn is_candidate(base: &str) -> bool {
    let lower = base.to_ascii_lowercase();
    CANDIDATE_WORDS.iter().any(|w| lower.contains(w))
}

/// Multipliziert in einer `economy_data.sii` alle Werte, die den Erlös eines Auftrags
/// bestimmen, mit dem Faktor. Alles andere bleibt Byte für Byte gleich, siehe
/// Modulbeschreibung.
///
/// Fehler, wenn die Datei kein Klartext ist, keiner der bekannten Schlüssel vorkommt, ein mit
/// `also` verlangter Schlüssel fehlt oder ein zu skalierender Wert keine Zahl ist.
pub fn scale_economy(data: &[u8], options: &ScaleOptions) -> Result<EconomyEdit> {
    for name in &options.also {
        if !valid_key_name(name) {
            return Err(Error::InvalidInput(format!(
                "--also {name}: erwartet wird ein Schlüssel ohne Index, zum Beispiel job_bonus"
            )));
        }
    }
    let scan = scan(data)?;
    let scaled: BTreeSet<&str> = REVENUE_KEYS
        .iter()
        .copied()
        .chain(options.also.iter().map(String::as_str))
        .collect();

    // Schlüssel, die in einer Unit als Liste vorkommen: deren Zeile ohne Index ist die Zählzeile
    let lists: HashSet<(&str, &str)> = scan
        .fields
        .iter()
        .filter(|f| f.indexed)
        .map(|f| (f.unit.as_str(), f.base.as_str()))
        .collect();
    let found: HashSet<&str> = scan.fields.iter().map(|f| f.base.as_str()).collect();

    // Eine Zeile, die nicht verstanden wurde, darf keinen der Schlüssel enthalten
    for note in &scan.skipped {
        if let Some(k) = scaled.iter().find(|k| note.text.contains(*k)) {
            return Err(Error::Parse {
                line: note.line,
                message: format!(
                    "die Zeile enthält {k}, ist aber kein Feld der Form name: wert: {}",
                    note.text
                ),
            });
        }
    }
    if !REVENUE_KEYS.iter().any(|k| found.contains(k)) {
        return Err(Error::InvalidInput(format!(
            "In der Datei steht keiner der Schlüssel {}. Das ist wohl nicht def/economy_data.sii aus def.scs",
            REVENUE_KEYS.join(", ")
        )));
    }
    for name in &options.also {
        if !found.contains(name.as_str()) {
            return Err(Error::InvalidInput(format!(
                "--also {name}: den Schlüssel gibt es in der Datei nicht"
            )));
        }
    }

    let mut changes = Vec::new();
    let mut candidates = Vec::new();
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    for f in &scan.fields {
        let value = data.get(f.start..f.end).unwrap_or_default();
        if !scaled.contains(f.base.as_str()) {
            if is_candidate(&f.base)
                && !(lists.contains(&(f.unit.as_str(), f.base.as_str())) && !f.indexed)
            {
                candidates.push(Candidate {
                    line: f.line,
                    unit: f.unit.clone(),
                    key: f.key.clone(),
                    value: text_of(value),
                });
            }
            continue;
        }
        if !f.indexed && lists.contains(&(f.unit.as_str(), f.base.as_str())) {
            continue; // Zählzeile einer Liste
        }
        let ranges = number_ranges(value).ok_or_else(|| Error::InvalidValue {
            unit: f.unit.clone(),
            field: format!("{} (Zeile {})", f.key, f.line),
            value: format!(
                "{}, erwartet wird eine Zahl oder ein Vektor aus Zahlen",
                text_of(value)
            ),
        })?;
        let vector = value.first() == Some(&b'(');
        for (n, (a, b)) in ranges.into_iter().enumerate() {
            let token = text_of(value.get(a..b).unwrap_or_default());
            let (new, became_fraction) =
                scale_number(&token, &options.factor).map_err(|message| Error::InvalidValue {
                    unit: f.unit.clone(),
                    field: format!("{} (Zeile {})", f.key, f.line),
                    value: message,
                })?;
            replacements.push((f.start + a, f.start + b, new.clone()));
            changes.push(Change {
                line: f.line,
                unit: f.unit.clone(),
                key: f.key.clone(),
                component: vector.then_some(n),
                old: token,
                new,
                became_fraction,
            });
        }
    }
    if changes.is_empty() {
        return Err(Error::InvalidInput(
            "Es gibt keinen Wert zu ändern, die bekannten Schlüssel stehen nur als Zählzeile einer Liste in der Datei".into(),
        ));
    }

    let mut text = Vec::with_capacity(data.len() + 16 * replacements.len());
    let mut pos = 0;
    for (a, b, new) in &replacements {
        text.extend_from_slice(data.get(pos..*a).unwrap_or_default());
        text.extend_from_slice(new.as_bytes());
        pos = *b;
    }
    text.extend_from_slice(data.get(pos..).unwrap_or_default());

    let missing = REVENUE_KEYS
        .iter()
        .filter(|k| !found.contains(*k))
        .map(|k| k.to_string())
        .collect();
    let edit = EconomyEdit {
        text,
        changes,
        missing,
        candidates,
        includes: scan.includes,
        skipped: scan.skipped,
        factor: options.factor.clone(),
    };
    verify_edit(data, &edit)?;
    Ok(edit)
}

/// Prüft die geänderte Datei: dieselben Felder in derselben Reihenfolge, unveränderte Werte
/// gleich, geänderte Werte genau die neuen Zahlen mit dem erwarteten Wert, und außerhalb der
/// Zahlen derselbe Text.
fn verify_edit(original: &[u8], edit: &EconomyEdit) -> Result<()> {
    let fail = |m: String| Error::Structure(format!("Prüfung der geänderten Datei: {m}"));
    let before = scan(original)?;
    let after = scan(&edit.text)?;
    if before.fields.len() != after.fields.len()
        || before.includes != after.includes
        || before.skipped != after.skipped
    {
        return Err(fail("die Felder haben sich verändert".into()));
    }
    let mut changes = edit.changes.iter().peekable();
    let mut skeleton_before = Vec::new();
    let mut skeleton_after = Vec::new();
    let (mut pb, mut pa) = (0, 0);
    for (b, a) in before.fields.iter().zip(&after.fields) {
        if (b.line, &b.unit, &b.key) != (a.line, &a.unit, &a.key) {
            return Err(fail(format!("Zeile {}: {} ist anders", b.line, b.key)));
        }
        let old = original.get(b.start..b.end).unwrap_or_default();
        let new = edit.text.get(a.start..a.end).unwrap_or_default();
        skeleton_before.extend_from_slice(original.get(pb..b.start).unwrap_or_default());
        skeleton_after.extend_from_slice(edit.text.get(pa..a.start).unwrap_or_default());
        (pb, pa) = (b.end, a.end);
        let mine: Vec<&Change> =
            std::iter::from_fn(|| changes.next_if(|c| c.line == b.line && c.key == b.key))
                .collect();
        if mine.is_empty() {
            if old != new {
                return Err(fail(format!(
                    "Zeile {}: {} hat sich verändert",
                    b.line, b.key
                )));
            }
            continue;
        }
        let new_ranges = number_ranges(new).unwrap_or_default();
        let old_ranges = number_ranges(old).unwrap_or_default();
        if new_ranges.len() != mine.len() || old_ranges.len() != mine.len() {
            return Err(fail(format!(
                "Zeile {}: {} hat einen anderen Aufbau",
                b.line, b.key
            )));
        }
        let mut o_pos = 0;
        let mut n_pos = 0;
        for ((c, (na, nb)), (oa, ob)) in mine.iter().zip(&new_ranges).zip(&old_ranges) {
            let token = text_of(new.get(*na..*nb).unwrap_or_default());
            let old_token = text_of(old.get(*oa..*ob).unwrap_or_default());
            let expected = number_value(&c.old).map(|v| v * edit.factor.value);
            let got = number_value(&token);
            let close = match (expected, got) {
                (Some(e), Some(g)) => {
                    (e - g).abs() <= e.abs() * 1e-6 + f64::from(f32::MIN_POSITIVE)
                }
                _ => false,
            };
            if token != c.new || old_token != c.old || !close {
                return Err(fail(format!(
                    "Zeile {}: {} ist nicht {}",
                    b.line, b.key, c.new
                )));
            }
            // Zwischen den Zahlen eines Vektors steht derselbe Text
            if old.get(o_pos..*oa) != new.get(n_pos..*na) {
                return Err(fail(format!(
                    "Zeile {}: {} hat einen anderen Aufbau",
                    b.line, b.key
                )));
            }
            (o_pos, n_pos) = (*ob, *nb);
        }
        if old.get(o_pos..) != new.get(n_pos..) {
            return Err(fail(format!(
                "Zeile {}: {} hat einen anderen Aufbau",
                b.line, b.key
            )));
        }
    }
    skeleton_before.extend_from_slice(original.get(pb..).unwrap_or_default());
    skeleton_after.extend_from_slice(edit.text.get(pa..).unwrap_or_default());
    if changes.next().is_some() || skeleton_before != skeleton_after {
        return Err(fail("außerhalb der Werte hat sich etwas verändert".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Mod bauen

/// Angaben für `manifest.sii`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModOptions {
    /// `package_version`, Standard die Version von SpedHub
    pub version: String,
    /// `compatible_versions[]`, Standard `1.62.*`
    pub compatible: String,
    /// Vorschaubild als JPG. Nur dann gibt es `icon` im Manifest.
    pub icon: Option<Vec<u8>>,
}

impl Default for ModOptions {
    fn default() -> Self {
        ModOptions {
            version: env!("CARGO_PKG_VERSION").into(),
            compatible: DEFAULT_COMPATIBLE.into(),
            icon: None,
        }
    }
}

fn check_manifest_text(what: &str, value: &str) -> Result<()> {
    let ok = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\');
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "{what} {value:?} geht nicht: erlaubt sind bis zu 64 Zeichen ohne Leerzeichen, Anführungszeichen und Backslash"
        )))
    }
}

/// Liest ein Vorschaubild und prüft, dass es ein JPG ist.
pub fn read_icon(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let data = fs::read(path).map_err(|e| Error::io(path, &e))?;
    if !data.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err(Error::InvalidInput(format!(
            "{} ist kein JPG. Das Vorschaubild muss ein JPG sein (laut SCS-Wiki 276 x 162 Pixel)",
            path.display()
        )));
    }
    Ok(data)
}

/// `manifest.sii` der Mod. `mp_mod_optional: false` macht die Mod im Convoy zur Pflicht.
pub fn manifest(options: &ModOptions) -> Result<String> {
    check_manifest_text("Die Version", &options.version)?;
    check_manifest_text("Die Spielversion", &options.compatible)?;
    let icon = if options.icon.is_some() {
        format!("\ticon: \"{ICON_PATH}\"\n")
    } else {
        String::new()
    };
    Ok(format!(
        "SiiNunit\n{{\nmod_package : .package_name\n{{\n\
         \tpackage_version: \"{version}\"\n\
         \tdisplay_name: \"{DISPLAY_NAME}\"\n\
         \tauthor: \"{AUTHOR}\"\n\
         \n\
         \tcategory[]: \"{CATEGORY}\"\n\
         \n\
         \tcompatible_versions[]: \"{compatible}\"\n\
         \n\
         {icon}\
         \tdescription_file: \"{DESCRIPTION_PATH}\"\n\
         \n\
         \tmp_mod_optional: false\n\
         }}\n}}\n",
        version = options.version,
        compatible = options.compatible,
    ))
}

/// `description.txt` der Mod, deutsch.
pub fn description(edit: &EconomyEdit, options: &ModOptions) -> String {
    let factor = &edit.factor;
    format!(
        "{DISPLAY_NAME} {version}\n\
         \n\
         Aufträge zahlen im Spiel nur noch {percent} Prozent des üblichen Erlöses. Dafür sind \
         in def/economy_data.sii die Werte für den Erlös mit {factor_text} multipliziert, alles \
         andere ist unverändert.\n\
         \n\
         Geänderte Werte: {keys}\n\
         \n\
         Gedacht ist die Mod für Firmen mit SpedHub. Der SpedHub-Server rechnet den gemeldeten \
         Erlös mit dem Erlösfaktor {revenue} zurück, in der Firmenkasse kommt also wieder der \
         volle Erlös an. Dazu im Dashboard im Büro den Erlösfaktor auf {revenue} stellen.\n\
         \n\
         Im Convoy ist die Mod Pflicht: Alle Fahrer brauchen sie.\n\
         \n\
         Gebaut für ETS2 {compatible} aus der economy_data.sii des Spiels. Nach einem Update \
         des Spiels die Mod neu bauen.\n\
         \n\
         SpedHub: {REPO_URL}\n",
        version = options.version,
        percent = factor.percent(),
        factor_text = factor.text(),
        keys = edit.changed_keys().join(", "),
        revenue = factor.revenue_factor(),
        compatible = options.compatible,
    )
}

/// Baut die Mod als ZIP und liest sie zur Prüfung wieder ein.
pub fn build_mod(edit: &EconomyEdit, options: &ModOptions) -> Result<Vec<u8>> {
    let mut entries = vec![
        ZipEntry::new(MANIFEST_PATH, manifest(options)?.into_bytes()),
        ZipEntry::new(DESCRIPTION_PATH, description(edit, options).into_bytes()),
    ];
    if let Some(icon) = &options.icon {
        entries.push(ZipEntry::new(ICON_PATH, icon.clone()));
    }
    entries.push(ZipEntry::new(SII_PATH, edit.text.clone()));
    let zip = write_stored(&entries)?;
    if read_stored(&zip)? != entries {
        return Err(Error::Structure(
            "die gebaute Mod ließ sich nicht unverändert wieder lesen".into(),
        ));
    }
    Ok(zip)
}

/// Schreibt die Mod. Eine vorhandene Datei wird nur mit `force` ersetzt. Geschrieben wird über
/// eine temporäre Datei daneben, die vorher zurückgelesen wird.
pub fn write_mod(path: impl AsRef<Path>, data: &[u8], force: bool) -> Result<PathBuf> {
    let path = path.as_ref();
    if path.is_dir() {
        return Err(Error::InvalidInput(format!(
            "{} ist ein Ordner. Bitte einen Dateinamen wie {MOD_FILE_NAME} angeben",
            path.display()
        )));
    }
    if path.exists() && !force {
        return Err(Error::InvalidInput(format!(
            "{} gibt es schon. Mit --force wird die Datei ersetzt",
            path.display()
        )));
    }
    let tmp = sibling(path, "spedhub-tmp");
    if let Err(e) = write_and_verify(&tmp, data) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Unter Windows ersetzt rename eine vorhandene Datei ebenfalls.
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(path, &e)
    })?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> Factor {
        Factor::parse(s).unwrap()
    }

    #[test]
    fn dezimalzahlen() {
        let p = |s: &str| Decimal::parse(s).and_then(Decimal::plain);
        assert_eq!(p("12").as_deref(), Some("12"));
        assert_eq!(p("1.50").as_deref(), Some("1.5"));
        assert_eq!(p("-.25").as_deref(), Some("-0.25"));
        assert_eq!(p("3.").as_deref(), Some("3"));
        assert_eq!(p("2e3").as_deref(), Some("2000"));
        assert_eq!(p("2.5E-3").as_deref(), Some("0.0025"));
        assert_eq!(p("-0").as_deref(), Some("0"));
        for bad in [
            "",
            ".",
            "-",
            "1e",
            "1e+",
            "a",
            "1.2.3",
            "&3f800000",
            "1,5",
            " 1",
        ] {
            assert!(Decimal::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn zahlen_skalieren() {
        let s = |t: &str| scale_number(t, &f("0.01")).unwrap();
        assert_eq!(s("600"), ("6".into(), false));
        assert_eq!(s("650"), ("6.5".into(), true));
        assert_eq!(s("13.5"), ("0.135".into(), false));
        assert_eq!(s("1.5e3"), ("15".into(), false));
        assert_eq!(s("-2"), ("-0.02".into(), true));
        assert_eq!(s("0"), ("0".into(), false));
        // 1.0 mal 0.01, auf den nächsten 32-Bit-Float gerundet
        assert_eq!(
            s("&3f800000"),
            (format!("&{:08x}", 0.01f32.to_bits()), false)
        );
        assert_eq!(s("&3F800000").0, format!("&{:08X}", 0.01f32.to_bits()));
        assert!(scale_number("&7f800000", &f("0.01")).is_err());
        assert!(scale_number("abc", &f("0.01")).is_err());
    }

    #[test]
    fn faktor() {
        assert_eq!(f("0.01").revenue_factor(), "100");
        assert_eq!(f("1e-2").percent(), "1");
        assert_eq!(f("0.5").revenue_factor(), "2");
        assert_eq!(f("0.03").revenue_factor(), "etwa 33.3333");
        assert_eq!(f("1").revenue_factor(), "1");
        assert_eq!(f("0.0001").revenue_factor(), "10000");
        for bad in ["0", "2", "-0.01", "0.00001", "x", "1,5"] {
            assert!(Factor::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(ScaleOptions::default().factor, f(DEFAULT_FACTOR));
    }

    #[test]
    fn kommentare_werden_maskiert() {
        let src = b"a: 1 # x: 2\nb: \"#kein\" // c: 3\n/* d: 4\ne: 5 */ f: 6\r\n";
        let m = mask_comments(src);
        assert_eq!(m.len(), src.len());
        let text = String::from_utf8(m).unwrap();
        assert!(!text.contains("x: 2") && !text.contains("c: 3") && !text.contains("e: 5"));
        assert!(text.contains("\"#kein\"") && text.contains(" f: 6\r\n"));
    }

    #[test]
    fn felder_lesen() {
        assert_eq!(
            parse_field(b"\tfixed_revenue: 600 \r"),
            Some((
                "fixed_revenue".into(),
                "fixed_revenue".into(),
                false,
                16,
                19
            ))
        );
        let (key, base, indexed, _, _) = parse_field(b" xp[3] : 7").unwrap();
        assert_eq!(
            (key.as_str(), base.as_str(), indexed),
            ("xp[3]", "xp", true)
        );
        assert!(parse_field(b" xp[]: 7").unwrap().2);
        assert!(parse_field(b"@include \"a.sui\"").is_none());
    }
}
