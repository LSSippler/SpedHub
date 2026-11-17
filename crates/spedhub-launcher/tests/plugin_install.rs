//! Plugin installieren, ersetzen, entfernen und Stand prüfen, in Testordnern. Dazu die
//! Kommandozeile `--install-plugin` und `--uninstall-plugin` mit nachgebautem Steam.
mod common;

use common::*;
use spedhub_launcher::Error;
use spedhub_launcher::cli::{
    self, Command, InstallEnv, InstallOutcome, install_parameters, parse_args, quote_arg,
};
use spedhub_launcher::plugin_install::{
    GameSource, PLUGIN_DLL, PluginStatus, Removal, install, installed_dll, locate_game,
    plugins_dir, set_aside, sha256_file, status, uninstall,
};
use spedhub_launcher::settings::{AppPaths, Settings};
use spedhub_launcher::steam::RegistryValues;
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
};

/// Löschen beim Neustart, das nur mitschreibt.
#[derive(Default)]
struct Reboot {
    marked: RefCell<Vec<PathBuf>>,
}

impl Reboot {
    fn mark(&self, path: &Path) -> Result<(), String> {
        self.marked.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
}

fn no_reboot(_: &Path) -> Result<(), String> {
    panic!("nichts zum Löschen beim Neustart vormerken")
}

/// Dateinamen im Ordner plugins, sortiert.
fn plugin_files(game: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(plugins_dir(game))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Eine DLL neben dem Launcher mit bestimmtem Inhalt.
fn bundled(dir: &Path, content: &[u8]) -> PathBuf {
    let path = dir.join("Programme").join("SpedHub").join(PLUGIN_DLL);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    path
}

/// Steam mit ETS2 in einer zweiten Bibliothek. Liefert Registry und Spielordner.
fn steam_with_game(root: &Path) -> (RegistryValues, PathBuf) {
    let steam = root.join("Steam");
    let lib = root.join("SteamLibrary");
    fs::create_dir_all(lib.join("steamapps")).unwrap();
    library_folders(&steam, &[(&steam, &[]), (&lib, &["227300"])]);
    app_manifest(&lib, None);
    let game = game_folder(
        &lib.join("steamapps")
            .join("common")
            .join("Euro Truck Simulator 2"),
    );
    (registry_hkcu(&steam), game)
}

#[test]
fn installieren_ersetzen_und_entfernen() {
    let root = temp_dir("plugin");
    let game = game_folder(&root.join("ETS2"));
    let dll = bundled(&root, b"SpedHub-Plugin 0.2.0");

    // Ohne Spiel, vor der Installation
    assert_eq!(status(None, Some(&dll)), PluginStatus::GameNotFound);
    assert_eq!(status(Some(&game), Some(&dll)), PluginStatus::Missing);
    assert!(!plugins_dir(&game).exists());

    // Installieren legt plugins an
    let r = install(&dll, &game, &FakeGame::closed()).unwrap();
    assert_eq!(r.target, installed_dll(&game));
    assert!(!r.replaced && !r.unchanged);
    assert_eq!(fs::read(&r.target).unwrap(), b"SpedHub-Plugin 0.2.0");
    assert_eq!(status(Some(&game), Some(&dll)), PluginStatus::Installed);
    let leftovers: Vec<_> = fs::read_dir(plugins_dir(&game))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![PLUGIN_DLL], "keine temporäre Datei bleibt");

    // Noch einmal: nichts zu tun
    let r = install(&dll, &game, &FakeGame::running()).unwrap();
    assert!(
        r.unchanged,
        "gleiche DLL, auch bei laufendem Spiel kein Fehler"
    );

    // Eine neue Fassung neben dem Launcher: im Spiel veraltet, wird ersetzt
    fs::write(&dll, b"SpedHub-Plugin 0.3.0, etwas laenger").unwrap();
    assert_eq!(status(Some(&game), Some(&dll)), PluginStatus::Outdated);
    let r = install(&dll, &game, &FakeGame::closed()).unwrap();
    assert!(r.replaced && !r.unchanged);
    assert_eq!(
        fs::read(installed_dll(&game)).unwrap(),
        b"SpedHub-Plugin 0.3.0, etwas laenger"
    );
    assert_eq!(
        sha256_file(&installed_dll(&game)).unwrap(),
        sha256_file(&dll).unwrap()
    );
    assert_eq!(status(Some(&game), Some(&dll)), PluginStatus::Installed);

    // Ohne mitgelieferte DLL lässt sich nichts vergleichen
    assert_eq!(status(Some(&game), None), PluginStatus::NoBundledDll);
    assert_eq!(
        status(Some(&game), Some(&root.join("fehlt.dll"))),
        PluginStatus::NoBundledDll
    );

    // Entfernen, zweimal
    assert_eq!(uninstall(&game, &FakeGame::closed()), Ok(Removal::Deleted));
    assert!(!installed_dll(&game).exists());
    assert_eq!(status(Some(&game), Some(&dll)), PluginStatus::Missing);
    assert_eq!(uninstall(&game, &FakeGame::closed()), Ok(Removal::NotThere));
}

#[test]
fn laufendes_spiel_und_falsche_ordner() {
    let root = temp_dir("plugin-gesperrt");
    let game = game_folder(&root.join("ETS2"));
    let old = bundled(&root, b"alt");
    install(&old, &game, &FakeGame::closed()).unwrap();
    fs::write(&old, b"neu").unwrap();

    // Das Spiel läuft: nichts ersetzen, nichts entfernen, klare Meldung
    let e = install(&old, &game, &FakeGame::running()).unwrap_err();
    assert!(e.to_string().contains("Spiel beenden"), "{e}");
    assert_eq!(fs::read(installed_dll(&game)).unwrap(), b"alt");
    let e = uninstall(&game, &FakeGame::running()).unwrap_err();
    assert!(matches!(e, Error::PluginInUse(_)), "{e:?}");
    assert!(e.to_string().contains("Spiel beenden"), "{e}");
    assert!(installed_dll(&game).exists());

    // Lässt sich nicht prüfen, ob es läuft: trotzdem versuchen
    let unknown = FakeGame::sequence(vec![Err("keine Prozessliste".into())]);
    install(&old, &game, &unknown).unwrap();
    assert_eq!(fs::read(installed_dll(&game)).unwrap(), b"neu");

    // Kein Spielordner, keine DLL
    let e = install(&old, &root, &FakeGame::closed()).unwrap_err();
    assert!(e.to_string().contains("bin\\win_x64"), "{e}");
    let e = install(&root.join("fehlt.dll"), &game, &FakeGame::closed()).unwrap_err();
    assert!(e.to_string().contains("Plugin-DLL fehlt"), "{e}");
}

#[test]
fn spielordner_finden_reihenfolge() {
    let root = temp_dir("plugin-suche");
    let (registry, steam_game) = steam_with_game(&root);
    let own = game_folder(&root.join("Eigener Ordner"));

    // Steam
    assert_eq!(
        locate_game(None, None, &registry),
        Ok((steam_game.clone(), GameSource::Steam))
    );
    // Gemerkt geht vor Steam, ein nicht mehr passender gemerkter wird übergangen
    assert_eq!(
        locate_game(None, Some(&own), &registry),
        Ok((own.clone(), GameSource::Settings))
    );
    assert_eq!(
        locate_game(None, Some(&root.join("weg")), &registry),
        Ok((steam_game, GameSource::Steam))
    );
    // Übergeben geht vor allem, ein falscher übergebener ist ein Fehler
    assert_eq!(
        locate_game(Some(&own.join("bin")), None, &registry),
        Ok((own, GameSource::Argument))
    );
    assert!(locate_game(Some(&root), None, &registry).is_err());
    let e = locate_game(None, None, &RegistryValues::default()).unwrap_err();
    assert!(e.contains("Steam"), "{e}");
}

#[test]
fn kommandozeile_lesen() {
    assert_eq!(parse_args(Vec::<String>::new()), Ok(Command::Gui));
    assert_eq!(
        parse_args(["--install-plugin"]),
        Ok(Command::InstallPlugin {
            game_dir: None,
            report: None,
            quiet: false
        })
    );
    assert_eq!(
        parse_args([
            "--install-plugin",
            "--game-dir",
            "D:\\Spiele\\ETS2",
            "--quiet"
        ]),
        Ok(Command::InstallPlugin {
            game_dir: Some(PathBuf::from("D:\\Spiele\\ETS2")),
            report: None,
            quiet: true
        })
    );
    assert_eq!(
        parse_args([
            "--install-plugin",
            "--report-game-dir",
            "C:\\Temp\\is-1\\spielordner.txt",
        ]),
        Ok(Command::InstallPlugin {
            game_dir: None,
            report: Some(PathBuf::from("C:\\Temp\\is-1\\spielordner.txt")),
            quiet: false
        })
    );
    assert_eq!(
        parse_args(["--remember-game-dir", "D:\\Spiele\\ETS2", "--quiet"]),
        Ok(Command::RememberGameDir {
            game_dir: PathBuf::from("D:\\Spiele\\ETS2"),
            quiet: true
        })
    );
    assert_eq!(
        parse_args(["--uninstall-plugin"]),
        Ok(Command::UninstallPlugin { quiet: false })
    );
    assert_eq!(parse_args(["--version"]), Ok(Command::Version));
    for bad in [
        vec!["--install-plugin", "--uninstall-plugin"],
        vec!["--install-plugin", "--game-dir"],
        vec!["--game-dir", "C:\\x"],
        vec!["--report-game-dir", "C:\\x.txt"],
        vec!["--uninstall-plugin", "--report-game-dir", "C:\\x.txt"],
        vec!["--install-plugin", "--report-game-dir"],
        vec!["--remember-game-dir"],
        vec!["--remember-game-dir", "C:\\x", "--install-plugin"],
        vec!["--remember-game-dir", "C:\\x", "--uninstall-plugin"],
        vec!["--irgendwas"],
    ] {
        assert!(parse_args(bad.clone()).is_err(), "{bad:?}");
    }
}

#[test]
fn argumente_fuer_den_neustart_mit_adminrechten() {
    assert_eq!(quote_arg("C:\\ETS2"), "C:\\ETS2");
    assert_eq!(
        quote_arg("C:\\Program Files (x86)\\Steam"),
        "\"C:\\Program Files (x86)\\Steam\""
    );
    // Backslash am Ende würde sonst das Anführungszeichen schützen
    assert_eq!(quote_arg("D:\\Mein Spiel\\"), "\"D:\\Mein Spiel\\\\\"");
    assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
    assert_eq!(quote_arg("a\\\"b"), "\"a\\\\\\\"b\"");
    assert_eq!(quote_arg(""), "\"\"");
    assert_eq!(install_parameters(None), "--install-plugin");
    assert_eq!(
        install_parameters(Some(Path::new("D:\\Mein Spiel"))),
        "--install-plugin --game-dir \"D:\\Mein Spiel\""
    );
}

#[test]
fn install_plugin_ueber_steam() {
    let root = temp_dir("cli-steam");
    let (registry, game) = steam_with_game(&root);
    let paths = AppPaths::new(root.join("appdata"));
    let dll = bundled(&root, b"plugin");
    let env = InstallEnv {
        paths: Some(&paths),
        registry,
        bundled: Some(dll.clone()),
        game: &FakeGame::closed(),
        delete_later: &no_reboot,
    };
    let mut asked = 0;
    let out = cli::install_plugin(None, &env, &mut |_| {
        asked += 1;
        None
    })
    .unwrap();
    match out {
        InstallOutcome::Installed {
            installed,
            game_dir,
            source,
            not_remembered,
        } => {
            assert_eq!(game_dir, game);
            assert_eq!(source, GameSource::Steam);
            assert_eq!(installed.target, installed_dll(&game));
            assert_eq!(not_remembered, None);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(asked, 0, "gefunden, also kein Ordnerdialog");
    assert_eq!(
        Settings::load(&paths).game_dir,
        None,
        "über Steam gefunden wird nicht gemerkt"
    );

    // Ohne DLL neben dem Launcher
    let env = InstallEnv {
        bundled: None,
        ..env
    };
    let e = cli::install_plugin(None, &env, &mut |_| None).unwrap_err();
    assert!(e.to_string().contains("spedhub_plugin.dll"), "{e}");
}

#[test]
fn install_plugin_ohne_steam_ordner_waehlen_und_merken() {
    let root = temp_dir("cli-waehlen");
    let paths = AppPaths::new(root.join("appdata"));
    // Vorhandene Einstellungen bleiben erhalten
    let mut s = Settings::default();
    s.server_url = "http://spedhub.example".into();
    s.user_name = "Alex".into();
    s.save(&paths).unwrap();

    let game = game_folder(&root.join("Spiele").join("ETS2"));
    let dll = bundled(&root, b"plugin");
    let env = InstallEnv {
        paths: Some(&paths),
        registry: RegistryValues::default(),
        bundled: Some(dll.clone()),
        game: &FakeGame::closed(),
        delete_later: &no_reboot,
    };

    // Erst ein falscher Ordner, dann der richtige (bin\win_x64 darin gewählt)
    let mut prompts = Vec::new();
    let mut answers = vec![root.clone(), game.join("bin").join("win_x64")].into_iter();
    let out = cli::install_plugin(None, &env, &mut |reason| {
        prompts.push(reason.to_string());
        answers.next()
    })
    .unwrap();
    assert!(matches!(
        out,
        InstallOutcome::Installed {
            source: GameSource::Argument,
            ..
        }
    ));
    assert_eq!(prompts.len(), 2);
    assert!(
        prompts[0].contains("Steam ist auf diesem Rechner nicht eingetragen"),
        "{}",
        prompts[0]
    );
    assert!(prompts[1].contains("bin\\win_x64"), "{}", prompts[1]);
    assert!(installed_dll(&game).is_file());

    let loaded = Settings::load(&paths);
    assert_eq!(loaded.game_dir, Some(game.clone()));
    assert_eq!(loaded.user_name, "Alex");
    assert_eq!(loaded.server_url, "http://spedhub.example");

    // Beim nächsten Mal kennt er den Ordner, ohne zu fragen
    fs::write(&dll, b"plugin neu").unwrap();
    let out = cli::install_plugin(None, &env, &mut |_| panic!("nicht fragen")).unwrap();
    assert!(matches!(
        out,
        InstallOutcome::Installed {
            source: GameSource::Settings,
            ..
        }
    ));
    assert_eq!(fs::read(installed_dll(&game)).unwrap(), b"plugin neu");

    // Mit --game-dir
    let other = game_folder(&root.join("Anderes ETS2"));
    cli::install_plugin(Some(&other), &env, &mut |_| None).unwrap();
    assert!(installed_dll(&other).is_file());
    assert_eq!(Settings::load(&paths).game_dir, Some(other.clone()));

    // Entfernen aus dem gemerkten Ordner
    let removed = cli::uninstall_plugin(&env, &mut |_| panic!("nicht fragen")).unwrap();
    assert_eq!(removed, vec![(other.clone(), Removal::Deleted)]);
    assert_eq!(cli::removal_note(&removed), None);
    assert!(!installed_dll(&other).exists());
    assert_eq!(cli::uninstall_plugin(&env, &mut |_| false), Ok(vec![]));
}

#[test]
fn install_plugin_abgebrochen() {
    let root = temp_dir("cli-abbruch");
    let paths = AppPaths::new(root.join("appdata"));
    let env = InstallEnv {
        paths: Some(&paths),
        registry: RegistryValues::default(),
        bundled: Some(bundled(&root, b"plugin")),
        game: &FakeGame::closed(),
        delete_later: &no_reboot,
    };
    match cli::install_plugin(None, &env, &mut |_| None).unwrap() {
        InstallOutcome::Cancelled { reason } => {
            assert!(reason.contains("nicht gefunden"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(Settings::load(&paths).game_dir, None);

    // Spiel nicht gefunden beim Entfernen: nichts zu tun
    assert_eq!(cli::uninstall_plugin(&env, &mut |_| false), Ok(vec![]));
}

#[test]
fn uninstall_plugin_laufendes_spiel_wiederholen() {
    let root = temp_dir("cli-entfernen");
    let (registry, game) = steam_with_game(&root);
    let dll = bundled(&root, b"plugin");
    install(&dll, &game, &FakeGame::closed()).unwrap();
    // Erst läuft das Spiel, nach dem Wiederholen ist es beendet
    let game_check = FakeGame::sequence(vec![Ok(true), Ok(false)]);
    let env = InstallEnv {
        paths: None,
        registry,
        bundled: Some(dll),
        game: &game_check,
        delete_later: &no_reboot,
    };
    let mut prompts = Vec::new();
    let removed = cli::uninstall_plugin(&env, &mut |reason| {
        prompts.push(reason.to_string());
        true
    })
    .unwrap();
    assert_eq!(removed, vec![(game.clone(), Removal::Deleted)]);
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("Spiel beenden"), "{}", prompts[0]);
    assert!(prompts[0].contains("Wiederholen"), "{}", prompts[0]);
    assert!(prompts[0].contains("Abbrechen"), "{}", prompts[0]);
    assert!(!installed_dll(&game).exists());
    assert_eq!(plugin_files(&game), Vec::<String>::new());
}

/// Die Deinstallation läuft bei laufendem Spiel weiter und löscht danach den Launcher. Es
/// darf keine DLL zurückbleiben, die das Spiel beim nächsten Start lädt, auch nicht still.
#[test]
fn uninstall_plugin_laufendes_spiel_zur_seite_legen() {
    let root = temp_dir("cli-entfernen-still");
    let (registry, game) = steam_with_game(&root);
    let dll = bundled(&root, b"plugin");
    install(&dll, &game, &FakeGame::closed()).unwrap();
    let running = FakeGame::running();
    let reboot = Reboot::default();
    let mark = |p: &Path| reboot.mark(p);
    let env = InstallEnv {
        paths: None,
        registry,
        bundled: Some(dll.clone()),
        game: &running,
        delete_later: &mark,
    };

    // Still: nicht fragen (retry liefert false), sondern zur Seite legen
    let mut asked = 0;
    let removed = cli::uninstall_plugin(&env, &mut |_| {
        asked += 1;
        false
    })
    .unwrap();
    assert_eq!(asked, 1);
    let aside = plugins_dir(&game).join(format!("{PLUGIN_DLL}.1.entfernt"));
    assert_eq!(
        removed,
        vec![(
            game.clone(),
            Removal::SetAside {
                path: aside.clone(),
                delete_on_reboot: Ok(())
            }
        )]
    );
    assert!(
        !installed_dll(&game).exists(),
        "das Spiel findet keine DLL mehr"
    );
    assert_eq!(fs::read(&aside).unwrap(), b"plugin");
    assert_eq!(*reboot.marked.borrow(), vec![aside.clone()]);
    assert!(
        plugin_files(&game).iter().all(|n| !n.ends_with(".dll")),
        "{:?}",
        plugin_files(&game)
    );
    assert_eq!(status(Some(&game), Some(&dll)), PluginStatus::Missing);
    let note = cli::removal_note(&removed).unwrap();
    assert!(note.contains("Neustart von Windows"), "{note}");

    // Eine neue Installation räumt die zur Seite gelegte weg
    install(&dll, &game, &FakeGame::closed()).unwrap();
    assert!(!aside.exists());
    // Hält das Spiel eine alte noch fest, bleibt sie liegen, die nächste bekommt einen freien
    // Namen
    fs::write(&aside, b"noch geladen").unwrap();
    let r = set_aside(&game, &|_| Err("keine Adminrechte".into())).unwrap();
    let second = plugins_dir(&game).join(format!("{PLUGIN_DLL}.2.entfernt"));
    assert_eq!(
        r,
        Removal::SetAside {
            path: second.clone(),
            delete_on_reboot: Err("keine Adminrechte".into())
        }
    );
    let note = cli::removal_note(&[(game.clone(), r)]).unwrap();
    assert!(note.contains("von Hand löschen"), "{note}");
    assert!(note.contains("keine Adminrechte"), "{note}");

    // Die nächste Deinstallation bei beendetem Spiel räumt die Reste weg
    assert_eq!(uninstall(&game, &FakeGame::closed()), Ok(Removal::NotThere));
    assert_eq!(plugin_files(&game), Vec::<String>::new());

    // Auch eine Installation räumt sie weg
    fs::write(&second, b"alt").unwrap();
    install(&dll, &game, &FakeGame::closed()).unwrap();
    assert_eq!(plugin_files(&game), vec![PLUGIN_DLL.to_string()]);

    // Nichts zum Zur-Seite-Legen
    uninstall(&game, &FakeGame::closed()).unwrap();
    assert_eq!(set_aside(&game, &no_reboot), Ok(Removal::NotThere));
}

/// Die Oberfläche wählt den Ordner selbst und gibt ihn mit --game-dir weiter. Der Launcher
/// mit Adminrechten fragt dann nicht noch einmal, auch wenn der Ordner nicht passt.
#[test]
fn install_plugin_uebergebener_ordner_ohne_nachfrage() {
    let root = temp_dir("cli-uebergeben");
    let paths = AppPaths::new(root.join("appdata"));
    let env = InstallEnv {
        paths: Some(&paths),
        registry: RegistryValues::default(),
        bundled: Some(bundled(&root, b"plugin")),
        game: &FakeGame::closed(),
        delete_later: &no_reboot,
    };
    let e = cli::install_plugin(Some(&root.join("kein Spiel")), &env, &mut |_| {
        panic!("nicht fragen")
    })
    .unwrap_err();
    assert!(e.to_string().contains("kein Spiel"), "{e}");
    assert_eq!(Settings::load(&paths).game_dir, None);
}

/// Das Setup läuft mit Adminrechten, unter Umständen mit einem anderen Konto. Ein dort von Hand
/// gewählter Ordner geht über eine Datei zurück und wird als ursprünglicher Benutzer gemerkt.
#[test]
fn spielordner_fuer_das_setup_melden_und_merken() {
    let root = temp_dir("cli-melden");
    let game = game_folder(&root.join("Spiele").join("Euro Truck Simulator 2"));
    let report = root.join("spielordner.txt");
    cli::write_game_dir_report(&report, &game).unwrap();
    let bytes = fs::read(&report).unwrap();
    assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]), "UTF-8 mit BOM");
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(text, format!("\u{feff}{}\r\n", game.display()));

    // Der Benutzer, der das Setup gestartet hat, merkt ihn, vorhandene Einstellungen bleiben
    let user = AppPaths::new(root.join("benutzer"));
    let mut s = Settings::default();
    s.user_name = "Alex".into();
    s.save(&user).unwrap();
    // Inno liest die Zeile mit BOM, falls es das nicht entfernt
    let line = text.trim_end().to_string();
    let dir = cli::remember_game_dir(Some(&user), Path::new(&line)).unwrap();
    assert_eq!(dir, game);
    let loaded = Settings::load(&user);
    assert_eq!(loaded.game_dir, Some(game.clone()));
    assert_eq!(loaded.user_name, "Alex");
    // Ein Unterordner des Spiels zählt als Spielordner
    let dir = cli::remember_game_dir(Some(&user), &game.join("bin").join("win_x64")).unwrap();
    assert_eq!(dir, game);

    // Falscher Ordner oder keine Einstellungen: Fehler, nichts gemerkt
    let other = AppPaths::new(root.join("anderer"));
    assert!(cli::remember_game_dir(Some(&other), &root).is_err());
    assert_eq!(Settings::load(&other).game_dir, None);
    assert!(cli::remember_game_dir(None, &game).is_err());
}
