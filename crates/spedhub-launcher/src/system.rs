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

/// Was die Registry über Steam weiß. Außerhalb von Windows leer.
pub fn steam_registry() -> crate::steam::RegistryValues {
    imp::steam_registry()
}

/// Art einer Meldung im Fenster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Warning,
    Error,
}

/// Zeigt eine Meldung in einem eigenen Fenster (MessageBox) und wartet auf OK. Für die
/// Kommandozeile, die keine Oberfläche hat. Außerhalb von Windows auf stderr.
pub fn message_box(title: &str, text: &str, kind: MessageKind) {
    imp::message_box(title, text, kind)
}

/// Lässt einen Ordner wählen. `None` bei Abbrechen. Außerhalb von Windows immer `None`.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    imp::pick_folder(title)
}

/// Startet `exe` mit Adminrechten (UAC, Verb `runas`) und wartet auf das Ende. Liefert den
/// Exit-Code. Lehnt der Benutzer die Nachfrage ab, gibt es einen Fehler. Außerhalb von Windows
/// startet es `exe` ohne Weiteres, zum Ausprobieren.
pub fn run_elevated_and_wait(exe: &std::path::Path, parameters: &str) -> Result<i32, String> {
    imp::run_elevated_and_wait(exe, parameters)
}

/// Öffnet eine Datei wie ein Doppelklick, für das geladene Setup.
pub fn open_path(path: &std::path::Path) -> Result<(), String> {
    imp::open_path(path)
}

#[cfg(windows)]
mod imp {
    use std::{
        ffi::OsString, os::windows::ffi::OsStrExt, os::windows::ffi::OsStringExt, path::PathBuf,
        ptr,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::Foundation::{ERROR_CANCELLED, GetLastError};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, INFINITE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, FOLDERID_Documents,
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHBrowseForFolderW, SHELLEXECUTEINFOW,
        SHGetKnownFolderPath, SHGetPathFromIDListW, ShellExecuteExW, ShellExecuteW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MessageBoxW,
        SW_SHOWNORMAL,
    };

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

    /// Liest einen Text-Wert aus der Registry, `None`, wenn es ihn nicht gibt.
    fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut size: u32 = 0;
        // SAFETY: Erster Aufruf nur für die Größe, ohne Puffer.
        let rc = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut size,
            )
        };
        if rc != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: buf hat bytes Bytes Platz, die API schreibt höchstens so viel.
        let rc = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if rc != 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let text = String::from_utf16_lossy(buf.get(..len).unwrap_or_default());
        let text = text.trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    pub fn steam_registry() -> crate::steam::RegistryValues {
        crate::steam::RegistryValues {
            current_user_steam_path: reg_string(
                HKEY_CURRENT_USER,
                "Software\\Valve\\Steam",
                "SteamPath",
            ),
            machine_wow64_install_path: reg_string(
                HKEY_LOCAL_MACHINE,
                "SOFTWARE\\WOW6432Node\\Valve\\Steam",
                "InstallPath",
            ),
            machine_install_path: reg_string(
                HKEY_LOCAL_MACHINE,
                "SOFTWARE\\Valve\\Steam",
                "InstallPath",
            ),
        }
    }

    pub fn message_box(title: &str, text: &str, kind: super::MessageKind) {
        let icon = match kind {
            super::MessageKind::Info => MB_ICONINFORMATION,
            super::MessageKind::Warning => MB_ICONWARNING,
            super::MessageKind::Error => MB_ICONERROR,
        };
        let (title, text) = (wide(title), wide(text));
        // SAFETY: Beide Zeichenketten sind nullterminiert und leben bis nach dem Aufruf.
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | icon | MB_SETFOREGROUND,
            )
        };
    }

    /// Ordnerdialog der Shell (SHBrowseForFolderW). windows-sys bringt keine
    /// COM-Schnittstellen mit, IFileOpenDialog bräuchte eine selbst gebaute vtable.
    pub fn pick_folder(title: &str) -> Option<PathBuf> {
        // SAFETY: COM für diesen Thread, der Dialog braucht es. Wird unten wieder beendet.
        let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        let title = wide(title);
        let mut display = vec![0u16; 260];
        let info = BROWSEINFOW {
            hwndOwner: ptr::null_mut(),
            pidlRoot: ptr::null_mut(),
            pszDisplayName: display.as_mut_ptr(),
            lpszTitle: title.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            lpfn: None,
            lParam: 0,
            iImage: 0,
        };
        // SAFETY: info zeigt auf gültige Puffer, die bis nach dem Aufruf leben.
        let pidl = unsafe { SHBrowseForFolderW(&info) };
        let result = if pidl.is_null() {
            None
        } else {
            // MAX_PATH reicht laut Dokumentation von SHGetPathFromIDListW
            let mut path = vec![0u16; 260];
            // SAFETY: pidl kommt vom Dialog, path hat MAX_PATH Zeichen Platz.
            let ok = unsafe { SHGetPathFromIDListW(pidl, path.as_mut_ptr()) } != 0;
            // SAFETY: Den pidl gibt der Aufrufer mit CoTaskMemFree frei.
            unsafe { CoTaskMemFree(pidl.cast()) };
            ok.then(|| {
                let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
                PathBuf::from(OsString::from_wide(path.get(..len).unwrap_or_default()))
            })
        };
        if hr >= 0 {
            // SAFETY: passt zum erfolgreichen CoInitializeEx oben.
            unsafe { CoUninitialize() };
        }
        result
    }

    pub fn run_elevated_and_wait(exe: &std::path::Path, parameters: &str) -> Result<i32, String> {
        let verb = wide("runas");
        let file: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
        let params = wide(parameters);
        // SAFETY: Ein reines Datenstruct, Nullen sind ein gültiger Anfang.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        // SAFETY: Alle Zeiger in info leben bis nach dem Aufruf.
        let ok = unsafe { ShellExecuteExW(&mut info) } != 0;
        if !ok {
            // SAFETY: direkt nach dem fehlgeschlagenen Aufruf
            let err = unsafe { GetLastError() };
            return Err(if err == ERROR_CANCELLED {
                "Die Nachfrage nach Adminrechten wurde abgelehnt, das Plugin ist nicht installiert"
                    .into()
            } else {
                format!("Der Launcher ließ sich nicht mit Adminrechten starten (Fehler {err})")
            });
        }
        if info.hProcess.is_null() {
            return Err("Der Launcher mit Adminrechten lässt sich nicht verfolgen".into());
        }
        let mut code: u32 = 1;
        // SAFETY: hProcess gehört uns (SEE_MASK_NOCLOSEPROCESS) und wird unten geschlossen.
        unsafe {
            WaitForSingleObject(info.hProcess, INFINITE);
            GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
        }
        Ok(code as i32)
    }

    pub fn open_path(path: &std::path::Path) -> Result<(), String> {
        let op = wide("open");
        let file: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
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
                "{} ließ sich nicht öffnen (Code {})",
                path.display(),
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

    pub fn steam_registry() -> crate::steam::RegistryValues {
        crate::steam::RegistryValues::default()
    }

    pub fn message_box(title: &str, text: &str, _kind: super::MessageKind) {
        eprintln!("{title}: {text}");
    }

    pub fn pick_folder(_title: &str) -> Option<PathBuf> {
        None
    }

    pub fn run_elevated_and_wait(exe: &std::path::Path, parameters: &str) -> Result<i32, String> {
        let status = std::process::Command::new(exe)
            .args(parameters.split_whitespace())
            .status()
            .map_err(|e| format!("{} ließ sich nicht starten: {e}", exe.display()))?;
        Ok(status.code().unwrap_or(1))
    }

    pub fn open_path(path: &std::path::Path) -> Result<(), String> {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("{} ließ sich nicht öffnen: {e}", path.display()))
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
