//! Alles, was das Betriebssystem betrifft: Ordner, laufendes Spiel, Steam starten, Token
//! verschlüsseln. Unter Windows über die Windows-API, sonst mit einfachen Ersatzwegen, damit
//! der Launcher auch unter Linux baut und sich testen lässt.

use std::path::PathBuf;

/// Name des Spielprozesses unter Windows (64 Bit).
pub const GAME_EXE: &str = "eurotrucks2.exe";
/// Steam-Link zum Starten von ETS2.
pub const STEAM_RUN_URL: &str = "steam://rungameid/227300";

/// Prüft, ob das Spiel läuft. Austauschbar, damit sich die Schreibsperre testen lässt.
pub trait GameCheck: Send + Sync {
    /// `Err`, wenn sich das nicht feststellen lässt. Dann wird ebenfalls nicht geschrieben.
    fn is_running(&self) -> Result<bool, String>;
}

/// Die echte Prüfung über die Prozessliste.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessCheck;

impl GameCheck for ProcessCheck {
    fn is_running(&self) -> Result<bool, String> {
        imp::game_running()
    }
}

/// `Dokumente\Euro Truck Simulator 2`. Unter Windows über den Bekannten Ordner "Dokumente",
/// der auch auf OneDrive umgeleitet sein kann.
pub fn ets2_documents_dir() -> Option<PathBuf> {
    imp::documents_dir().map(|d| d.join("Euro Truck Simulator 2"))
}

/// `%LOCALAPPDATA%\SpedHub`, hier liegen Einstellungen, `plugin.json` und Backups.
pub fn app_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".local").join("share"))
        })?;
    Some(base.join("SpedHub"))
}

/// Öffnet `steam://rungameid/227300`. Steam startet dann das Spiel.
pub fn start_game() -> Result<(), String> {
    imp::open_url(STEAM_RUN_URL)
}

/// Verschlüsselt Daten für den angemeldeten Windows-Nutzer (DPAPI). Liefert die Daten und die
/// Kennung des Verfahrens.
pub fn protect(data: &[u8]) -> Result<(Vec<u8>, &'static str), String> {
    imp::protect(data)
}

pub fn unprotect(data: &[u8], method: &str) -> Result<Vec<u8>, String> {
    imp::unprotect(data, method)
}

#[cfg(windows)]
mod imp {
    use std::{
        ffi::OsString, os::windows::ffi::OsStrExt, os::windows::ffi::OsStringExt, path::PathBuf,
        ptr,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, ShellExecuteW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    pub const DPAPI: &str = "dpapi";

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain([0]).collect()
    }

    pub fn documents_dir() -> Option<PathBuf> {
        let mut path: *mut u16 = ptr::null_mut();
        // SAFETY: FOLDERID_Documents ist eine gültige GUID. Bei Erfolg gehört der Puffer uns und
        // wird mit CoTaskMemFree freigegeben, auch bei Fehlern laut Dokumentation.
        let hr =
            unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, 0, ptr::null_mut(), &mut path) };
        let result = if hr >= 0 && !path.is_null() {
            // SAFETY: path zeigt auf eine nullterminierte UTF-16-Zeichenkette.
            let len = unsafe { (0..).take_while(|&i| *path.add(i) != 0).count() };
            // SAFETY: len Zeichen ab path sind gültig.
            let slice = unsafe { std::slice::from_raw_parts(path, len) };
            Some(PathBuf::from(OsString::from_wide(slice)))
        } else {
            None
        };
        // SAFETY: CoTaskMemFree akzeptiert auch einen Nullzeiger.
        unsafe { CoTaskMemFree(path.cast()) };
        result
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Documents")))
    }

    pub fn game_running() -> Result<bool, String> {
        // SAFETY: Schnappschuss der Prozessliste, das Handle wird unten geschlossen.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE {
            return Err("Die Prozessliste ließ sich nicht lesen".into());
        }
        // SAFETY: PROCESSENTRY32W ist ein reines Datenstruct, Nullen sind ein gültiger Anfang.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        // SAFETY: snap ist gültig, entry hat die richtige Größe.
        let mut ok = unsafe { Process32FirstW(snap, &mut entry) } != 0;
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(entry.szExeFile.get(..len).unwrap_or_default());
            if name.eq_ignore_ascii_case(super::GAME_EXE) {
                found = true;
                break;
            }
            // SAFETY: wie oben
            ok = unsafe { Process32NextW(snap, &mut entry) } != 0;
        }
        // SAFETY: snap wurde oben geöffnet.
        unsafe { CloseHandle(snap) };
        Ok(found)
    }

    pub fn open_url(url: &str) -> Result<(), String> {
        let (op, file) = (wide("open"), wide(url));
        // SAFETY: Alle Zeichenketten sind nullterminiert und leben bis nach dem Aufruf.
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                op.as_ptr(),
                file.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize > 32 {
            Ok(())
        } else {
            Err(format!(
                "{url} ließ sich nicht öffnen. Ist Steam installiert? (Code {})",
                result as isize
            ))
        }
    }

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr().cast_mut(),
        }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: Bei Erfolg zeigt pbData auf cbData Bytes, die wir mit LocalFree freigeben.
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: Der Puffer stammt von CryptProtectData bzw. CryptUnprotectData.
        unsafe { LocalFree(out.pbData.cast()) };
        bytes
    }

    pub fn protect(data: &[u8]) -> Result<(Vec<u8>, &'static str), String> {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        // SAFETY: input zeigt auf data, das während des Aufrufs lebt. Die API liest nur.
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err("Der Token ließ sich nicht verschlüsseln".into());
        }
        Ok((take(out), DPAPI))
    }

    pub fn unprotect(data: &[u8], method: &str) -> Result<Vec<u8>, String> {
        if method != DPAPI {
            return Err(format!("Unbekanntes Verfahren {method}"));
        }
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        // SAFETY: wie bei protect
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(
                "Der gespeicherte Token ließ sich nicht entschlüsseln. Bitte neu anmelden".into(),
            );
        }
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    /// Ohne Windows gibt es kein DPAPI. Der Token wird dann unverschlüsselt gespeichert. Das ist
    /// nur für Entwicklung und Tests gedacht, der Launcher ist eine Windows-Anwendung.
    pub const PLAIN: &str = "none";

    pub fn documents_dir() -> Option<PathBuf> {
        // ETS2 für Linux legt seine Daten hier ab.
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
    }

    pub fn game_running() -> Result<bool, String> {
        let entries = std::fs::read_dir("/proc").map_err(|e| e.to_string())?;
        Ok(entries.flatten().any(|e| {
            std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "eurotrucks2")
        }))
    }

    pub fn open_url(url: &str) -> Result<(), String> {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("{url} ließ sich nicht öffnen: {e}"))
    }

    pub fn protect(data: &[u8]) -> Result<(Vec<u8>, &'static str), String> {
        Ok((data.to_vec(), PLAIN))
    }

    pub fn unprotect(data: &[u8], method: &str) -> Result<Vec<u8>, String> {
        if method == PLAIN {
            Ok(data.to_vec())
        } else {
            Err(format!(
                "Verfahren {method} gibt es nur unter Windows. Bitte neu anmelden"
            ))
        }
    }
}
