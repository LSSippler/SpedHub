//! Spielstand im Textformat (SiiN): einlesen, bearbeiten, wieder ausgeben.
//!
//! Was der Parser annimmt, gibt [`Save::dump`] byte-genau so wieder aus, wie es eingelesen
//! wurde. Tolerant ist er bei drei Dingen, die sich je nach Herkunft der Datei unterscheiden:
//!
//! - Zeilenende LF oder CRLF. Gemischte Dateien werden gelesen und beim Schreiben auf das
//!   häufigere Zeilenende vereinheitlicht.
//! - Ein Zeilenumbruch nach der letzten `}` oder keiner.
//! - Einrückung der Felder mit beliebig vielen Leerzeichen oder Tabs. Sie bleibt je Zeile erhalten.
//!
//! Alles andere muss genau dem bekannten Aufbau entsprechen, sonst gibt es einen Fehler mit
//! Zeilennummer.

use crate::error::{Error, Result};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

const HEADER: &str = "SiiNunit\n{\n";
const NAMELESS: &str = "_nameless.";
const DEFAULT_INDENT: &str = " ";

/// Ein Feld einer Unit, also eine Zeile ` key: value`. Listen-Einträge haben den Index im
/// Schlüssel, zum Beispiel `accessories[3]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    pub value: String,
    /// Leerzeichen und Tabs vor dem Schlüssel, so wie sie in der Datei standen
    pub indent: String,
}

impl Field {
    /// Ein Feld mit der üblichen Einrückung von einem Leerzeichen.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Field {
        Field::with_indent(key, value, DEFAULT_INDENT)
    }

    pub fn with_indent(
        key: impl Into<String>,
        value: impl Into<String>,
        indent: impl Into<String>,
    ) -> Field {
        Field {
            key: key.into(),
            value: value.into(),
            indent: indent.into(),
        }
    }
}

/// Zeilenende einer Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// `\n`, so schreibt `decrypt_truck` den Text aus BSII
    #[default]
    Lf,
    /// `\r\n`, so schreibt das Spiel `info.sii`
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// Wie der Text in der Datei aussah. Beim Schreiben wird er genauso wiederhergestellt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextStyle {
    pub line_ending: LineEnding,
    /// Steht nach der letzten `}` noch ein Zeilenumbruch?
    pub final_newline: bool,
}

/// Eine Unit `typ : name { ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub kind: String,
    pub name: String,
    pub fields: Vec<Field>,
}

/// Das Format, in dem eine Datei auf der Platte lag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `SiiN`, Klartext
    Text,
    /// `ScsC`, verschlüsselt und komprimiert
    Encrypted,
    /// `BSII`, binär
    Binary,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Text => "Klartext (SiiN)",
            Format::Encrypted => "verschlüsselt (ScsC)",
            Format::Binary => "binär (BSII)",
        }
    }
}

/// Wandelt eine Datei jedes Formats in Klartext um. Für ScsC und BSII übernimmt das
/// `decrypt_truck`. Falls die Bibliothek bei kaputten Daten in Panik gerät, wird daraus
/// ein normaler Fehler.
pub fn decode(data: &[u8]) -> Result<(Format, Vec<u8>)> {
    let format = match data.get(..4) {
        Some(b"SiiN") => return Ok((Format::Text, data.to_vec())),
        Some(b"ScsC") => Format::Encrypted,
        Some(b"BSII") => Format::Binary,
        Some(_) => {
            return Err(Error::Decode(
                "unbekanntes Format, erwartet wird SiiN, ScsC oder BSII".into(),
            ));
        }
        None => return Err(Error::Decode("die Datei ist zu kurz".into())),
    };
    let input = data.to_vec();
    let result = std::panic::catch_unwind(|| decrypt_truck::decrypt_bin_file(&input));
    match result {
        Ok(Ok(text)) if text.starts_with(b"SiiN") => Ok((format, text)),
        Ok(Ok(_)) => Err(Error::Decode(
            "nach dem Entschlüsseln kam kein Text heraus".into(),
        )),
        Ok(Err(e)) => Err(Error::Decode(e)),
        Err(_) => Err(Error::Decode("die Daten sind beschädigt".into())),
    }
}

/// Ein ganzer Spielstand: die Units in der Reihenfolge der Datei, dazu Zeilenende und
/// Dateiende. Zwei Spielstände sind gleich, wenn ihre Units gleich sind.
#[derive(Debug, Clone)]
pub struct Save {
    units: Vec<Unit>,
    index: HashMap<String, usize>,
    style: TextStyle,
    /// Die häufigste Einrückung der Datei, für neue Felder
    indent: String,
}

impl Default for Save {
    fn default() -> Self {
        Save {
            units: Vec::new(),
            index: HashMap::new(),
            style: TextStyle::default(),
            indent: DEFAULT_INDENT.into(),
        }
    }
}

impl PartialEq for Save {
    fn eq(&self, other: &Self) -> bool {
        self.units == other.units
    }
}

impl Save {
    /// Liest eine Datei in jedem Format und gibt auch das Format zurück.
    pub fn read(path: impl AsRef<Path>) -> Result<(Save, Format)> {
        let path = path.as_ref();
        let data = std::fs::read(path).map_err(|e| Error::io(path, &e))?;
        Save::from_bytes(&data)
    }

    pub fn from_bytes(data: &[u8]) -> Result<(Save, Format)> {
        let (format, text) = decode(data)?;
        let text = String::from_utf8(text)
            .map_err(|_| Error::Decode("der Text ist kein gültiges UTF-8".into()))?;
        Ok((Save::parse(&text)?, format))
    }

    /// Liest Klartext. Gelingt das, gibt [`Save::dump`] genau diesen Text wieder aus, außer
    /// bei gemischten Zeilenenden.
    pub fn parse(text: &str) -> Result<Save> {
        let (normalized, line_ending) = normalize_line_endings(text)?;
        let text = normalized.as_str();
        if !text.starts_with(HEADER) {
            return Err(Error::Parse {
                line: 1,
                message: if text.starts_with("SiiNunit") {
                    "nach SiiNunit muss eine Zeile mit { folgen".into()
                } else {
                    "kein Spielstand im Textformat, SiiNunit fehlt".into()
                },
            });
        }
        let (text, final_newline) = if let Some(t) = text.strip_suffix("}\n") {
            (t, true)
        } else if let Some(t) = text.strip_suffix('}') {
            (t, false)
        } else {
            return Err(end_error(text));
        };
        let mut save = Save::parse_body(text)?;
        save.style = TextStyle {
            line_ending,
            final_newline,
        };
        Ok(save)
    }

    /// Liest den Text mit LF, ohne die letzte `}`.
    fn parse_body(text: &str) -> Result<Save> {
        let body = text.strip_prefix(HEADER).ok_or_else(|| end_error(text))?;
        let end_error = || end_error(text);
        let mut save = Save::default();
        if body.is_empty() {
            return Ok(save);
        }
        let body = body.strip_suffix('\n').ok_or_else(end_error)?;
        let mut indents: HashMap<String, usize> = HashMap::new();

        enum State {
            Between,
            InUnit(Unit, usize),
            AfterUnit,
        }
        let mut state = State::Between;
        for (i, line) in body.split('\n').enumerate() {
            let ln = i + 3;
            let err = |message: String| Error::Parse { line: ln, message };
            state = match state {
                State::Between => {
                    let (kind, name) = parse_unit_header(line).ok_or_else(|| {
                        err(if line.is_empty() {
                            "zu viele Leerzeilen zwischen zwei Units".into()
                        } else {
                            format!("erwartet wird der Kopf einer Unit (typ : name {{), gefunden: {line}")
                        })
                    })?;
                    let unit = Unit {
                        kind: kind.into(),
                        name: name.into(),
                        fields: Vec::new(),
                    };
                    State::InUnit(unit, ln)
                }
                State::InUnit(mut unit, start) => {
                    if line == "}" {
                        if save.index.contains_key(&unit.name) {
                            return Err(Error::Parse {
                                line: start,
                                message: format!("den Unit-Namen {} gibt es doppelt", unit.name),
                            });
                        }
                        save.index.insert(unit.name.clone(), save.units.len());
                        save.units.push(unit);
                        State::AfterUnit
                    } else {
                        let field = parse_field(line).ok_or_else(|| {
                            err(format!(
                                "erwartet wird ein Feld ( name: wert) oder }}, gefunden: {line}"
                            ))
                        })?;
                        *indents.entry(field.indent.clone()).or_default() += 1;
                        unit.fields.push(field);
                        State::InUnit(unit, start)
                    }
                }
                State::AfterUnit => {
                    if !line.is_empty() {
                        return Err(err(
                            "nach dem Ende einer Unit muss eine Leerzeile folgen".into()
                        ));
                    }
                    State::Between
                }
            };
        }
        if let Some((indent, _)) = indents
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        {
            save.indent = indent;
        }
        match state {
            State::Between => Ok(save),
            State::InUnit(unit, start) => Err(Error::Parse {
                line: start,
                message: format!("die Unit {} wird nicht geschlossen", unit.name),
            }),
            State::AfterUnit => Err(end_error()),
        }
    }

    /// Gibt den Spielstand als Klartext aus, mit Zeilenende, Einrückung und Dateiende wie
    /// beim Einlesen.
    pub fn dump(&self) -> String {
        let nl = self.style.line_ending.as_str();
        let size: usize = self
            .units
            .iter()
            .map(|u| {
                u.kind.len()
                    + u.name.len()
                    + 10
                    + u.fields
                        .iter()
                        .map(|f| f.indent.len() + f.key.len() + f.value.len() + 4)
                        .sum::<usize>()
            })
            .sum();
        let mut out = String::with_capacity(size + HEADER.len() + 4);
        out.push_str("SiiNunit");
        out.push_str(nl);
        out.push('{');
        out.push_str(nl);
        for u in &self.units {
            out.push_str(&u.kind);
            out.push_str(" : ");
            out.push_str(&u.name);
            out.push_str(" {");
            out.push_str(nl);
            for f in &u.fields {
                out.push_str(&f.indent);
                out.push_str(&f.key);
                out.push_str(": ");
                out.push_str(&f.value);
                out.push_str(nl);
            }
            out.push('}');
            out.push_str(nl);
            out.push_str(nl);
        }
        out.push('}');
        if self.style.final_newline {
            out.push_str(nl);
        }
        out
    }

    /// Zeilenende und Dateiende, mit denen [`Save::dump`] schreibt.
    pub fn style(&self) -> TextStyle {
        self.style
    }

    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Die häufigste Einrückung der Datei. Neue Felder bekommen diese.
    pub fn indent(&self) -> &str {
        &self.indent
    }

    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    pub fn unit(&self, name: &str) -> Option<&Unit> {
        self.index.get(name).and_then(|&i| self.units.get(i))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub(crate) fn require(&self, name: &str) -> Result<&Unit> {
        self.unit(name)
            .ok_or_else(|| Error::MissingUnit(name.into()))
    }

    pub(crate) fn require_mut(&mut self, name: &str) -> Result<&mut Unit> {
        let i = *self
            .index
            .get(name)
            .ok_or_else(|| Error::MissingUnit(name.into()))?;
        self.units
            .get_mut(i)
            .ok_or_else(|| Error::MissingUnit(name.into()))
    }

    /// Position einer Unit in der Datei.
    pub fn position(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    pub fn get(&self, unit: &str, key: &str) -> Result<&str> {
        self.require(unit)?.require(key)
    }

    pub fn set(&mut self, unit: &str, key: &str, value: &str) -> Result<()> {
        self.require_mut(unit)?.set(key, value)
    }

    pub fn array(&self, unit: &str, key: &str) -> Result<Vec<String>> {
        self.require(unit)?.array(key)
    }

    pub fn set_array(&mut self, unit: &str, key: &str, values: &[String]) -> Result<()> {
        self.require_mut(unit)?.set_array(key, values)
    }

    /// Namen aller Units eines Typs, in Dateireihenfolge.
    pub fn names_of_kind(&self, kind: &str) -> Vec<String> {
        self.units
            .iter()
            .filter(|u| u.kind == kind)
            .map(|u| u.name.clone())
            .collect()
    }

    /// Die eine Unit eines Typs. Fehler, wenn es keine oder mehrere gibt.
    pub fn single(&self, kind: &str) -> Result<String> {
        let mut names = self.names_of_kind(kind).into_iter();
        match (names.next(), names.next()) {
            (Some(n), None) => Ok(n),
            (None, _) => Err(Error::Structure(format!("keine Unit vom Typ {kind}"))),
            (Some(_), Some(_)) => Err(Error::Structure(format!(
                "mehr als eine Unit vom Typ {kind}"
            ))),
        }
    }

    /// Ist der Wert ein Verweis, also genau der Name einer Unit in der Datei?
    pub fn resolves(&self, value: &str) -> Option<&str> {
        let v = value.trim();
        self.index.get_key_value(v).map(|(k, _)| k.as_str())
    }

    /// Alle Verweise einer Unit, in Feldreihenfolge.
    pub fn refs(&self, unit: &Unit) -> Vec<String> {
        unit.fields
            .iter()
            .filter_map(|f| self.resolves(&f.value).map(str::to_string))
            .collect()
    }

    /// Alles, was von `start` aus über Verweise erreichbar ist, `start` eingeschlossen.
    /// Mit `nameless_only` folgt die Suche nur Verweisen auf `_nameless`-Units, also nicht
    /// in Garagen, Fahrer oder Städte hinein. Die Reihenfolge ist die einer Tiefensuche mit
    /// Stapel und entspricht der von `tools/transplant.py`.
    pub fn closure(&self, start: &str, nameless_only: bool) -> Vec<String> {
        let mut seen = Vec::new();
        let mut done = HashSet::new();
        let mut stack = vec![start.to_string()];
        while let Some(n) = stack.pop() {
            if !done.insert(n.clone()) {
                continue;
            }
            let Some(unit) = self.unit(&n) else { continue };
            seen.push(n);
            stack.extend(
                self.refs(unit)
                    .into_iter()
                    .filter(|r| !nameless_only || is_nameless(r)),
            );
        }
        seen
    }

    /// Alle Stellen, die auf eine der Units in `targets` verweisen, in einem Durchlauf:
    /// (Ziel, verweisende Unit, Feld).
    pub fn references_into(&self, targets: &HashSet<String>) -> Vec<(String, String, String)> {
        self.units
            .iter()
            .flat_map(|u| {
                u.fields.iter().filter_map(|f| {
                    let v = f.value.trim();
                    targets
                        .contains(v)
                        .then(|| (v.to_string(), u.name.clone(), f.key.clone()))
                })
            })
            .collect()
    }

    /// Fügt Units an einer Position ein. Namen müssen neu sein.
    pub(crate) fn insert_units(&mut self, pos: usize, units: Vec<Unit>) -> Result<()> {
        let mut new_names = HashSet::new();
        for u in &units {
            if self.contains(&u.name) || !new_names.insert(u.name.clone()) {
                return Err(Error::NameCollision(u.name.clone()));
            }
        }
        let pos = pos.min(self.units.len());
        self.units.splice(pos..pos, units);
        self.reindex();
        Ok(())
    }

    /// Entfernt Units. Verweise auf sie räumt der Aufrufer selbst auf.
    pub(crate) fn remove_units(&mut self, names: &HashSet<String>) {
        self.units.retain(|u| !names.contains(&u.name));
        self.reindex();
    }

    fn reindex(&mut self) {
        self.index = self
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| (u.name.clone(), i))
            .collect();
    }
}

impl Unit {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.as_str())
    }

    pub(crate) fn require(&self, key: &str) -> Result<&str> {
        self.get(key)
            .ok_or_else(|| Error::missing_field(&self.name, key))
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let name = self.name.clone();
        let field = self
            .fields
            .iter_mut()
            .find(|f| f.key == key)
            .ok_or_else(|| Error::missing_field(&name, key))?;
        field.value = value.to_string();
        Ok(())
    }

    /// Liest eine Liste: Zählzeile `key: N`, danach `key[0]` bis `key[N-1]`.
    pub fn array(&self, key: &str) -> Result<Vec<String>> {
        let count = self.require(key)?;
        let n: usize = count
            .parse()
            .map_err(|_| Error::invalid_value(&self.name, key, count))?;
        (0..n)
            .map(|i| {
                let k = format!("{key}[{i}]");
                self.require(&k).map(str::to_string)
            })
            .collect()
    }

    /// Ersetzt eine Liste ganz. Zählzeile und Einträge müssen zusammenhängend stehen.
    pub fn set_array(&mut self, key: &str, values: &[String]) -> Result<()> {
        let positions: Vec<usize> = self
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.key == key || array_base(&f.key).is_some_and(|(b, _)| b == key))
            .map(|(i, _)| i)
            .collect();
        let (Some(&first), Some(&last)) = (positions.first(), positions.last()) else {
            return Err(Error::missing_field(&self.name, key));
        };
        let contiguous = positions.iter().enumerate().all(|(n, &p)| p == first + n);
        if !contiguous || self.fields.get(first).is_none_or(|f| f.key != key) {
            return Err(Error::Structure(format!(
                "die Liste {key} in {} steht nicht zusammenhängend",
                self.name
            )));
        }
        let indent = self
            .fields
            .get(first)
            .map_or(DEFAULT_INDENT, |f| f.indent.as_str())
            .to_string();
        let mut new = Vec::with_capacity(values.len() + 1);
        new.push(Field::with_indent(key, values.len().to_string(), &indent));
        new.extend(
            values
                .iter()
                .enumerate()
                .map(|(i, v)| Field::with_indent(format!("{key}[{i}]"), v.clone(), &indent)),
        );
        self.fields.splice(first..=last, new);
        Ok(())
    }
}

pub fn is_nameless(name: &str) -> bool {
    name.starts_with(NAMELESS)
}

/// Zerlegt `key[3]` in (`key`, 3).
pub(crate) fn array_base(key: &str) -> Option<(&str, usize)> {
    let inner = key.strip_suffix(']')?;
    let (base, index) = inner.rsplit_once('[')?;
    if base.is_empty() || index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((base, index.parse().ok()?))
}

fn parse_unit_header(line: &str) -> Option<(&str, &str)> {
    let head = line.strip_suffix(" {")?;
    let (kind, name) = head.split_once(" : ")?;
    (valid_kind(kind) && valid_name(name)).then_some((kind, name))
}

fn end_error(text: &str) -> Error {
    Error::Parse {
        line: text.matches('\n').count() + 1,
        message: "die Datei muss mit einer Zeile } enden, danach höchstens ein Zeilenumbruch"
            .into(),
    }
}

/// Feldzeile: Einrückung aus mindestens einem Leerzeichen oder Tab, dann `key: value`.
fn parse_field(line: &str) -> Option<Field> {
    let rest = line.trim_start_matches([' ', '\t']);
    let indent = line.get(..line.len() - rest.len())?;
    if indent.is_empty() {
        return None;
    }
    let (key, value) = rest.split_once(": ")?;
    valid_key(key).then(|| Field::with_indent(key, value, indent))
}

/// Ersetzt CRLF durch LF und stellt fest, welches Zeilenende häufiger ist. Ein CR ohne LF
/// dahinter ist ein Fehler.
fn normalize_line_endings(text: &str) -> Result<(String, LineEnding)> {
    let bytes = text.as_bytes();
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut line = 1usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => crlf += 1,
            b'\r' => {
                return Err(Error::Parse {
                    line,
                    message: "ein CR ohne LF dahinter, erlaubt sind nur LF und CRLF".into(),
                });
            }
            b'\n' => {
                line += 1;
                if i == 0 || bytes.get(i - 1) != Some(&b'\r') {
                    lf += 1;
                }
            }
            _ => {}
        }
    }
    if crlf == 0 {
        return Ok((text.to_string(), LineEnding::Lf));
    }
    let ending = if crlf > lf {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    Ok((text.replace("\r\n", "\n"), ending))
}

pub(crate) fn valid_kind(kind: &str) -> bool {
    !kind.is_empty() && kind.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '{' || c == '}')
}

pub(crate) fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && !key.contains(": ")
        && !key.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Ein Wert darf keinen Zeilenumbruch enthalten, sonst zerfällt die Datei.
pub(crate) fn valid_value(value: &str) -> bool {
    !value.contains(['\n', '\r'])
}
