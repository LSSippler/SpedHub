//! Werte im Textformat: Zahlen und Zeichenketten.

/// Liest eine Kommazahl. Das Spiel schreibt sie als `&` plus die Bits eines 32-Bit-Floats in
/// Hex (`&3dd0332a`) oder, bei glatten Werten, dezimal (`0`, `1`).
pub fn parse_float(value: &str) -> Option<f32> {
    let v = value.trim();
    let f = match v.strip_prefix('&') {
        Some(hex) if hex.len() == 8 => f32::from_bits(u32::from_str_radix(hex, 16).ok()?),
        Some(_) => return None,
        None => v.parse().ok()?,
    };
    f.is_finite().then_some(f)
}

/// Schreibt eine Kommazahl so, wie das Spiel es tut: glatte Werte dezimal, alles andere als Hex.
pub fn format_float(f: f32) -> String {
    if f == 0.0 {
        "0".into()
    } else if f.fract() == 0.0 && f.abs() < 16_777_216.0 {
        format!("{}", f as i64)
    } else {
        format!("&{:08x}", f.to_bits())
    }
}

pub fn parse_int(value: &str) -> Option<i64> {
    value.trim().parse().ok()
}

/// Inhalt einer Zeichenkette `"..."` ohne die Anführungszeichen. Escapes bleiben, wie sie sind.
pub fn unquote(value: &str) -> Option<&str> {
    value.strip_prefix('"')?.strip_suffix('"')
}

pub fn quote(content: &str) -> String {
    format!("\"{content}\"")
}

/// Darf das so zwischen Anführungszeichen stehen? Keine Anführungszeichen, keine Backslashes,
/// keine Steuerzeichen.
pub fn safe_string_content(content: &str) -> bool {
    !content
        .chars()
        .any(|c| c == '"' || c == '\\' || c.is_control())
}
