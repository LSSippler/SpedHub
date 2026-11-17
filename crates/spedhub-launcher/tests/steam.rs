//! ETS2 über Steam finden: Registry-Werte, libraryfolders.vdf, App-Manifest. Alles mit
//! nachgebauten Werten und Ordnern, ohne Windows und ohne Steam.
mod common;

use common::*;
use spedhub_launcher::steam::{
    FindError, Library, RegistryValues, Vdf, find_ets2, game_dir_in, libraries, parse_install_dir,
    parse_library_folders, parse_vdf, validate_game_dir,
};
use std::{fs, path::PathBuf};

const NEW_FORMAT: &str = r#"
"libraryfolders"
{
	"contentstatsid"		"-4372837788361231553"
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"contentid"		"6098458396845716523"
		"totalsize"		"0"
		"apps"
		{
			"228980"		"1258371463"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"label"		"Spiele \"groß\""
		"apps"
		{
			"227300"		"28437492734"
			"270880"		"12"
		}
	}
}
"#;

#[test]
fn vdf_neues_format_mit_escapes() {
    let libs = parse_library_folders(NEW_FORMAT).unwrap();
    assert_eq!(
        libs,
        vec![
            Library {
                path: PathBuf::from("C:\\Program Files (x86)\\Steam"),
                apps: vec!["228980".into()],
            },
            Library {
                path: PathBuf::from("D:\\SteamLibrary"),
                apps: vec!["227300".into(), "270880".into()],
            },
        ]
    );
    let root = parse_vdf(NEW_FORMAT).unwrap();
    let label = root
        .get("libraryfolders")
        .and_then(|f| f.get("1"))
        .and_then(|l| l.get("label"))
        .and_then(Vdf::text);
    assert_eq!(label, Some("Spiele \"groß\""));
}

#[test]
fn vdf_altes_format() {
    let text = "\"LibraryFolders\"\n{\n\t\"TimeNextStatsReport\"\t\t\"1590000000\"\n\t\"ContentStatsID\"\t\t\"-1\"\n\t\"1\"\t\t\"D:\\\\SteamLibrary\"\n\t\"2\"\t\t\"E:\\\\Spiele\\\\Steam\"\n}\n";
    let libs = parse_library_folders(text).unwrap();
    let paths: Vec<PathBuf> = libs.iter().map(|l| l.path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            PathBuf::from("D:\\SteamLibrary"),
            PathBuf::from("E:\\Spiele\\Steam")
        ]
    );
    assert!(libs.iter().all(|l| l.apps.is_empty()));
}

#[test]
fn vdf_kommentare_bedingungen_und_ohne_anfuehrungszeichen() {
    let text = "// Kommentar\nlibraryfolders\n{\n  \"1\" \"D:\\\\Lib\" [$WIN32]\n  // noch einer\n  2 E:/Lib\n}\n";
    let libs = parse_library_folders(text).unwrap();
    assert_eq!(libs.len(), 2);
    assert_eq!(libs[1].path, PathBuf::from("E:/Lib"));
}

#[test]
fn kaputte_dateien_geben_fehler_statt_panik() {
    let cases = [
        (
            "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\"C:\\\\Steam\"\n",
            "nicht geschlossen",
        ),
        (
            "\"libraryfolders\"\n{\n\t\"path\"\t\t\"C:\\\\Steam\n",
            "Anführungszeichen",
        ),
        ("\"libraryfolders\"\n{\n}\n}\n", "Klammer zu viel"),
        ("\"libraryfolders\"\n{\n\t\"0\"\n}\n", "fehlt der Wert"),
        ("{ \"0\" \"x\" }", "Klammer ohne Schlüssel"),
        ("\"libraryfolders\"", "fehlt der Wert"),
        ("\"etwas\" \"anderes\"", "libraryfolders"),
        ("", "libraryfolders"),
        ("\"libraryfolders\" \"kein Block\"", "libraryfolders"),
    ];
    for (text, expected) in cases {
        let e = parse_library_folders(text).unwrap_err();
        assert!(e.contains(expected), "{text:?}: {e}");
    }
    // Ein Backslash ganz am Ende
    assert!(parse_vdf("\"a\" \"b\\").is_err());
    // Binärmüll
    let junk: String = (0u8..=255).map(char::from).collect();
    let _ = parse_vdf(&junk);
}

#[test]
fn installdir_aus_dem_app_manifest() {
    let text =
        "\"AppState\"\n{\n\t\"appid\"\t\t\"227300\"\n\t\"installdir\"\t\t\"ETS2 Steam\"\n}\n";
    assert_eq!(parse_install_dir(text).as_deref(), Some("ETS2 Steam"));
    assert_eq!(
        parse_install_dir("\"AppState\" { \"appid\" \"227300\" }"),
        None
    );
    assert_eq!(parse_install_dir("kaputt {"), None);
    // Kein Pfad nach oben oder woandershin
    assert_eq!(
        parse_install_dir("\"AppState\" { \"installdir\" \"..\\\\..\\\\Windows\" }"),
        None
    );
    assert_eq!(
        parse_install_dir("\"AppState\" { \"installdir\" \"C:/x\" }"),
        None
    );

    let root = temp_dir("appmanifest");
    app_manifest(&root, Some("ETS2 Steam"));
    assert_eq!(
        game_dir_in(&root),
        root.join("steamapps").join("common").join("ETS2 Steam")
    );
    let other = temp_dir("appmanifest-ohne");
    app_manifest(&other, None);
    assert_eq!(
        game_dir_in(&other),
        other
            .join("steamapps")
            .join("common")
            .join("Euro Truck Simulator 2")
    );
}

#[test]
fn registry_werte() {
    // Keiner
    assert!(RegistryValues::default().steam_dirs().is_empty());
    assert_eq!(
        find_ets2(&RegistryValues::default()),
        Err(FindError::NoSteam)
    );

    // Nur HKLM
    let only_hklm = RegistryValues {
        machine_wow64_install_path: Some("C:\\Program Files (x86)\\Steam".into()),
        ..Default::default()
    };
    assert_eq!(
        only_hklm.steam_dirs(),
        vec![PathBuf::from("C:\\Program Files (x86)\\Steam")]
    );

    // Nur HKCU, wie Steam ihn schreibt: mit / und klein
    let only_hkcu = RegistryValues {
        current_user_steam_path: Some("c:/program files (x86)/steam".into()),
        ..Default::default()
    };
    assert_eq!(
        only_hkcu.steam_dirs(),
        vec![PathBuf::from("c:/program files (x86)/steam")]
    );

    // Alle drei, derselbe Ordner zählt nur einmal, HKCU zuerst
    let all = RegistryValues {
        current_user_steam_path: Some("c:/program files (x86)/steam".into()),
        machine_wow64_install_path: Some("C:\\Program Files (x86)\\Steam\\".into()),
        machine_install_path: Some("  ".into()),
    };
    assert_eq!(
        all.steam_dirs(),
        vec![PathBuf::from("c:/program files (x86)/steam")]
    );

    // Eingetragen, aber nicht vorhanden
    let gone = RegistryValues {
        machine_install_path: Some(
            temp_dir("steam-weg")
                .join("gibt es nicht")
                .to_string_lossy()
                .into(),
        ),
        ..Default::default()
    };
    assert_eq!(find_ets2(&gone), Err(FindError::NoSteam));
}

#[test]
fn spiel_in_der_zweiten_bibliothek() {
    let root = temp_dir("zwei-bibliotheken");
    let steam = root.join("Steam");
    let second = root.join("SteamLibrary");
    let third = root.join("Weitere");
    for d in [&steam, &second, &third] {
        fs::create_dir_all(d.join("steamapps")).unwrap();
    }
    library_folders(
        &steam,
        &[
            (&steam, &["228980"]),
            (&second, &["270880", "227300"]),
            (&third, &[]),
        ],
    );
    app_manifest(&second, Some("Euro Truck Simulator 2"));
    let game = game_folder(
        &second
            .join("steamapps")
            .join("common")
            .join("Euro Truck Simulator 2"),
    );

    let (libs, problem) = libraries(&steam);
    assert_eq!(problem, None);
    assert_eq!(libs.len(), 3, "Steam-Ordner nur einmal: {libs:?}");
    assert_eq!(find_ets2(&registry_hkcu(&steam)), Ok(game.clone()));

    // Auch über HKLM gefunden
    let hklm = RegistryValues {
        machine_install_path: Some(steam.to_string_lossy().into()),
        ..Default::default()
    };
    assert_eq!(find_ets2(&hklm), Ok(game));
}

#[test]
fn altes_format_ohne_apps_sucht_das_manifest() {
    let root = temp_dir("altes-format");
    let steam = root.join("Steam");
    let lib = root.join("Spiele");
    fs::create_dir_all(steam.join("steamapps")).unwrap();
    fs::write(
        steam.join("steamapps").join("libraryfolders.vdf"),
        format!(
            "\"LibraryFolders\"\n{{\n\t\"1\"\t\t\"{}\"\n}}\n",
            vdf_path(&lib)
        ),
    )
    .unwrap();
    app_manifest(&lib, Some("ETS2"));
    let game = game_folder(&lib.join("steamapps").join("common").join("ETS2"));
    assert_eq!(find_ets2(&registry_hkcu(&steam)), Ok(game));
}

#[test]
fn spiel_im_steam_ordner_ohne_libraryfolders() {
    let root = temp_dir("nur-steam");
    let steam = root.join("Steam");
    app_manifest(&steam, None);
    let game = game_folder(
        &steam
            .join("steamapps")
            .join("common")
            .join("Euro Truck Simulator 2"),
    );
    assert_eq!(find_ets2(&registry_hkcu(&steam)), Ok(game));
}

#[test]
fn spiel_fehlt() {
    let root = temp_dir("spiel-fehlt");
    let steam = root.join("Steam");
    let second = root.join("SteamLibrary");
    fs::create_dir_all(second.join("steamapps")).unwrap();
    library_folders(&steam, &[(&steam, &["228980"]), (&second, &["270880"])]);
    match find_ets2(&registry_hkcu(&steam)) {
        Err(FindError::NotInstalled { libraries, broken }) => {
            assert_eq!(libraries, vec![steam.clone(), second.clone()]);
            assert!(broken.is_empty());
        }
        other => panic!("{other:?}"),
    }
    let text = find_ets2(&registry_hkcu(&steam)).unwrap_err().to_string();
    assert!(text.contains("in keiner Steam-Bibliothek"), "{text}");

    // Laut libraryfolders.vdf da, aber der Ordner fehlt: dann nicht gefunden
    library_folders(&steam, &[(&steam, &["228980"]), (&second, &["227300"])]);
    assert!(matches!(
        find_ets2(&registry_hkcu(&steam)),
        Err(FindError::NotInstalled { .. })
    ));

    // Ordner da, aber ohne bin\win_x64
    let game = second
        .join("steamapps")
        .join("common")
        .join("Euro Truck Simulator 2");
    fs::create_dir_all(&game).unwrap();
    assert_eq!(
        find_ets2(&registry_hkcu(&steam)),
        Err(FindError::NoBinaries(game.clone()))
    );
    assert!(
        FindError::NoBinaries(game)
            .to_string()
            .contains("bin\\win_x64")
    );
}

#[test]
fn kaputte_libraryfolders_datei() {
    let root = temp_dir("kaputt");
    let steam = root.join("Steam");
    fs::create_dir_all(steam.join("steamapps")).unwrap();
    fs::write(
        steam.join("steamapps").join("libraryfolders.vdf"),
        "\"libraryfolders\"\n{\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Steam",
    )
    .unwrap();
    let (libs, problem) = libraries(&steam);
    assert_eq!(libs.len(), 1, "der Steam-Ordner selbst bleibt");
    assert!(problem.unwrap().contains("ist kaputt"));
    match find_ets2(&registry_hkcu(&steam)) {
        Err(e @ FindError::NotInstalled { .. }) => {
            assert!(
                e.to_string().contains("libraryfolders.vdf ist kaputt"),
                "{e}"
            );
        }
        other => panic!("{other:?}"),
    }

    // Liegt das Spiel im Steam-Ordner selbst, wird es trotzdem gefunden
    app_manifest(&steam, None);
    let game = game_folder(
        &steam
            .join("steamapps")
            .join("common")
            .join("Euro Truck Simulator 2"),
    );
    assert_eq!(find_ets2(&registry_hkcu(&steam)), Ok(game));
}

#[test]
fn ordner_von_hand_pruefen() {
    let root = temp_dir("von-hand");
    let game = game_folder(&root.join("ETS2"));
    assert_eq!(validate_game_dir(&game), Ok(game.clone()));
    // Auch bin oder bin\win_x64 gewählt
    assert_eq!(validate_game_dir(&game.join("bin")), Ok(game.clone()));
    assert_eq!(
        validate_game_dir(&game.join("bin").join("win_x64")),
        Ok(game.clone())
    );
    let e = validate_game_dir(&root).unwrap_err();
    assert!(e.contains("bin\\win_x64"), "{e}");
    assert!(validate_game_dir(&root.join("gibt es nicht")).is_err());
}
