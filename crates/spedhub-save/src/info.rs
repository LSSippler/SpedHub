//! Name und Zeitstempel eines Speicherplatzes in `info.sii`.

use crate::error::{Error, Result};
use crate::layout::info as i;
use crate::sii::Save;
use crate::value::{parse_int, quote, safe_string_content, unquote};

/// Angezeigter Name des Speicherplatzes. Leer bei Autosaves.
pub fn info_name(info: &Save) -> Result<String> {
    let unit = info.single(i::KIND)?;
    let raw = info.get(&unit, i::NAME)?;
    unquote(raw)
        .map(str::to_string)
        .ok_or_else(|| Error::invalid_value(&unit, i::NAME, raw))
}

/// Setzt den angezeigten Namen. Erlaubt sind druckbare ASCII-Zeichen ohne `"` und `\`,
/// höchstens 64. Wie das Spiel andere Zeichen in `info.sii` kodiert, ist nicht geprüft.
pub fn set_info_name(info: &mut Save, name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 || !name.is_ascii() || !safe_string_content(name) {
        return Err(Error::InvalidInput(format!(
            "Der Name {name:?} passt nicht: 1 bis 64 Zeichen, nur ASCII, keine Anführungszeichen"
        )));
    }
    let unit = info.single(i::KIND)?;
    info.get(&unit, i::NAME)?;
    info.set(&unit, i::NAME, &quote(name))
}

/// Setzt den Zeitpunkt des Speicherns, Sekunden seit 1970.
pub fn set_info_file_time(info: &mut Save, unix_seconds: u64) -> Result<()> {
    let unit = info.single(i::KIND)?;
    let raw = info.get(&unit, i::FILE_TIME)?;
    parse_int(raw).ok_or_else(|| Error::invalid_value(&unit, i::FILE_TIME, raw))?;
    info.set(&unit, i::FILE_TIME, &unix_seconds.to_string())
}
