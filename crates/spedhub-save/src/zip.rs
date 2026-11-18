//! Ein kleiner ZIP-Schreiber und -Leser für unkomprimierte Einträge (Methode "stored").
//!
//! Eine Mod für ETS2 ist ein ZIP-Archiv mit der Endung `.scs`. Für die Wirtschafts-Mod reichen
//! drei, vier kleine Dateien, Kompression lohnt sich nicht. Statt der Crate `zip` mit ihren
//! Abhängigkeiten gibt es deshalb nur das hier: lokale Kopfzeilen, Zentralverzeichnis,
//! Abschluss, CRC-32. Kein ZIP64, keine Verschlüsselung, keine Datenbeschreibung nach den Daten.
//!
//! Der Leser versteht genau das, was der Schreiber erzeugt, und prüft dabei alles, was er
//! liest (Signaturen, Größen, CRC-32, Namen in Kopf und Verzeichnis gleich). Er dient dazu, ein
//! frisch gebautes Archiv vor dem Schreiben zu prüfen, und den Tests.

use crate::error::{Error, Result};

const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const END_SIGNATURE: u32 = 0x0605_4b50;
const LOCAL_HEADER_LEN: usize = 30;
const END_LEN: usize = 22;
/// ZIP 1.0 reicht für unkomprimierte Einträge ohne Ordner-Einträge.
const VERSION_NEEDED: u16 = 10;
/// Oberes Byte 0: Attribute wie bei MS-DOS. Unteres Byte: Version 2.0 der Spezifikation.
const VERSION_MADE_BY: u16 = 20;
const METHOD_STORED: u16 = 0;
/// Feste Zeit 00:00 und festes Datum 2026-01-01 im MS-DOS-Format. So ergibt dieselbe Eingabe
/// immer dieselbe Datei.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;

/// Ein Eintrag im Archiv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// Pfad im Archiv mit `/`, ohne führenden Schrägstrich, zum Beispiel `def/economy_data.sii`
    pub name: String,
    pub data: Vec<u8>,
}

impl ZipEntry {
    pub fn new(name: impl Into<String>, data: impl Into<Vec<u8>>) -> ZipEntry {
        ZipEntry {
            name: name.into(),
            data: data.into(),
        }
    }
}

/// CRC-32 nach ISO-HDLC, wie ZIP es verlangt (Polynom 0xEDB88320, gespiegelt).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}

/// Erlaubt sind relative Pfade aus ASCII-Zeichen ohne Steuerzeichen, mit `/` getrennt, ohne
/// leere Teile, `.` oder `..`.
fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= usize::from(u16::MAX)
        && name.is_ascii()
        && !name.chars().any(|c| c.is_ascii_control() || c == '\\')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if ok {
        Ok(())
    } else {
        Err(invalid(format!("ungültiger Name im Archiv: {name:?}")))
    }
}

fn u16_of(n: usize, what: &str) -> Result<u16> {
    u16::try_from(n).map_err(|_| invalid(format!("{what} ist zu groß für ein ZIP ohne ZIP64")))
}

fn u32_of(n: usize, what: &str) -> Result<u32> {
    u32::try_from(n).map_err(|_| invalid(format!("{what} ist zu groß für ein ZIP ohne ZIP64")))
}

/// Baut ein ZIP-Archiv mit unkomprimierten Einträgen in der angegebenen Reihenfolge.
pub fn write_stored(entries: &[ZipEntry]) -> Result<Vec<u8>> {
    let mut seen = std::collections::HashSet::new();
    for e in entries {
        check_name(&e.name)?;
        if !seen.insert(e.name.as_str()) {
            return Err(invalid(format!("doppelter Name im Archiv: {}", e.name)));
        }
    }
    let count = u16_of(entries.len(), "Die Zahl der Einträge")?;
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for e in entries {
        let offset = u32_of(out.len(), "Das Archiv")?;
        let size = u32_of(e.data.len(), &e.name)?;
        let name_len = u16_of(e.name.len(), &e.name)?;
        let crc = crc32(&e.data);

        put32(&mut out, LOCAL_SIGNATURE);
        put16(&mut out, VERSION_NEEDED);
        put16(&mut out, 0); // Flags
        put16(&mut out, METHOD_STORED);
        put16(&mut out, DOS_TIME);
        put16(&mut out, DOS_DATE);
        put32(&mut out, crc);
        put32(&mut out, size); // komprimiert
        put32(&mut out, size); // unkomprimiert
        put16(&mut out, name_len);
        put16(&mut out, 0); // Zusatzfeld
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(&e.data);

        put32(&mut central, CENTRAL_SIGNATURE);
        put16(&mut central, VERSION_MADE_BY);
        put16(&mut central, VERSION_NEEDED);
        put16(&mut central, 0); // Flags
        put16(&mut central, METHOD_STORED);
        put16(&mut central, DOS_TIME);
        put16(&mut central, DOS_DATE);
        put32(&mut central, crc);
        put32(&mut central, size);
        put32(&mut central, size);
        put16(&mut central, name_len);
        put16(&mut central, 0); // Zusatzfeld
        put16(&mut central, 0); // Kommentar
        put16(&mut central, 0); // Datenträger
        put16(&mut central, 0); // interne Attribute
        put32(&mut central, 0); // externe Attribute
        put32(&mut central, offset);
        central.extend_from_slice(e.name.as_bytes());
    }
    let central_offset = u32_of(out.len(), "Das Archiv")?;
    let central_size = u32_of(central.len(), "Das Verzeichnis")?;
    out.extend_from_slice(&central);
    put32(&mut out, END_SIGNATURE);
    put16(&mut out, 0); // dieser Datenträger
    put16(&mut out, 0); // Datenträger mit dem Verzeichnis
    put16(&mut out, count);
    put16(&mut out, count);
    put32(&mut out, central_size);
    put32(&mut out, central_offset);
    put16(&mut out, 0); // Kommentar
    Ok(out)
}

/// Liest ein Archiv, wie [`write_stored`] es schreibt, und prüft es dabei. Andere Archive
/// (komprimiert, ZIP64, mit Kommentar) werden abgelehnt.
pub fn read_stored(data: &[u8]) -> Result<Vec<ZipEntry>> {
    let bad = |m: &str| invalid(format!("Archiv ungültig: {m}"));
    let end_at = data
        .len()
        .checked_sub(END_LEN)
        .ok_or_else(|| bad("zu kurz"))?;
    let mut end = Reader::at(data, end_at);
    if end.u32()? != END_SIGNATURE {
        return Err(bad("kein Abschluss am Ende (Kommentar oder kein ZIP)"));
    }
    let (disk, central_disk) = (end.u16()?, end.u16()?);
    let (on_disk, total) = (end.u16()?, end.u16()?);
    let central_size = end.u32()? as usize;
    let central_offset = end.u32()? as usize;
    if disk != 0 || central_disk != 0 || on_disk != total {
        return Err(bad("mehrteilige Archive werden nicht unterstützt"));
    }
    if central_offset.checked_add(central_size) != Some(end_at) {
        return Err(bad("das Verzeichnis liegt nicht direkt vor dem Abschluss"));
    }

    let mut entries = Vec::with_capacity(usize::from(total));
    let mut dir = Reader::at(data, central_offset);
    let mut expected_offset = 0usize;
    for _ in 0..total {
        if dir.u32()? != CENTRAL_SIGNATURE {
            return Err(bad("Eintrag im Verzeichnis ohne Signatur"));
        }
        let _made_by = dir.u16()?;
        let _needed = dir.u16()?;
        let flags = dir.u16()?;
        let method = dir.u16()?;
        let _time = dir.u16()?;
        let _date = dir.u16()?;
        let crc = dir.u32()?;
        let packed = dir.u32()? as usize;
        let size = dir.u32()? as usize;
        let name_len = usize::from(dir.u16()?);
        let extra_len = usize::from(dir.u16()?);
        let comment_len = usize::from(dir.u16()?);
        dir.skip(8)?; // Datenträger, interne und externe Attribute
        let offset = dir.u32()? as usize;
        let name = dir.bytes(name_len)?.to_vec();
        dir.skip(extra_len + comment_len)?;
        if flags != 0 || method != METHOD_STORED || packed != size {
            return Err(bad("nur unkomprimierte Einträge ohne Flags werden gelesen"));
        }
        if offset != expected_offset {
            return Err(bad("Einträge liegen nicht lückenlos hintereinander"));
        }
        let name = String::from_utf8(name).map_err(|_| bad("Name ist kein UTF-8"))?;
        check_name(&name)?;

        let mut local = Reader::at(data, offset);
        if local.u32()? != LOCAL_SIGNATURE {
            return Err(bad("lokaler Kopf ohne Signatur"));
        }
        local.skip(10)?; // Version, Flags, Methode, Zeit, Datum
        let (l_crc, l_packed, l_size) =
            (local.u32()?, local.u32()? as usize, local.u32()? as usize);
        let l_name_len = usize::from(local.u16()?);
        let l_extra_len = usize::from(local.u16()?);
        let l_name = local.bytes(l_name_len)?;
        local.skip(l_extra_len)?;
        if l_crc != crc || l_packed != packed || l_size != size || l_name != name.as_bytes() {
            return Err(bad("lokaler Kopf und Verzeichnis passen nicht zusammen"));
        }
        let content = local.bytes(size)?.to_vec();
        if crc32(&content) != crc {
            return Err(bad(&format!("CRC-32 von {name} stimmt nicht")));
        }
        expected_offset = offset + LOCAL_HEADER_LEN + l_name_len + l_extra_len + size;
        entries.push(ZipEntry {
            name,
            data: content,
        });
    }
    if expected_offset != central_offset {
        return Err(bad(
            "zwischen den Einträgen und dem Verzeichnis liegen Daten",
        ));
    }
    if dir.pos != end_at {
        return Err(bad("das Verzeichnis hat eine falsche Größe"));
    }
    Ok(entries)
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn at(data: &'a [u8], pos: usize) -> Reader<'a> {
        Reader { data, pos }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| invalid("Archiv ungültig: Größe läuft über"))?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| invalid("Archiv ungültig: endet zu früh"))?;
        self.pos = end;
        Ok(slice)
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([
            b.first().copied().unwrap_or(0),
            b.get(1).copied().unwrap_or(0),
        ]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        let mut a = [0u8; 4];
        a.copy_from_slice(b);
        Ok(u32::from_le_bytes(a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_bekannte_werte() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414f_a339
        );
    }

    #[test]
    fn schreiben_und_lesen() {
        let entries = vec![
            ZipEntry::new("manifest.sii", b"SiiNunit\n{\n}\n".to_vec()),
            ZipEntry::new("def/economy_data.sii", b"abc".to_vec()),
            ZipEntry::new("leer.txt", Vec::new()),
        ];
        let zip = write_stored(&entries).unwrap();
        assert_eq!(read_stored(&zip).unwrap(), entries);
        // Gleiche Eingabe, gleiche Datei
        assert_eq!(write_stored(&entries).unwrap(), zip);
    }

    #[test]
    fn kaputte_archive_werden_erkannt() {
        let zip = write_stored(&[ZipEntry::new("a.txt", b"hallo".to_vec())]).unwrap();
        let mut kaputt = zip.clone();
        // Ein Byte des Inhalts ändern: die CRC-32 passt nicht mehr
        let i = LOCAL_HEADER_LEN + "a.txt".len();
        kaputt[i] ^= 1;
        assert!(read_stored(&kaputt).is_err());
        assert!(read_stored(&zip[..zip.len() - 1]).is_err());
        assert!(read_stored(b"").is_err());
    }

    #[test]
    fn namen_werden_geprueft() {
        for name in ["", "/a", "a//b", "../a", "a\\b", "ä.txt", "a/./b"] {
            assert!(
                write_stored(&[ZipEntry::new(name, Vec::new())]).is_err(),
                "{name}"
            );
        }
        assert!(
            write_stored(&[
                ZipEntry::new("a", Vec::new()),
                ZipEntry::new("a", Vec::new())
            ])
            .is_err()
        );
    }
}
