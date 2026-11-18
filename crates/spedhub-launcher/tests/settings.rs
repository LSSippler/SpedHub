//! Einstellungen mit Token und die Konfiguration für das Plugin.
mod common;

use common::*;
use spedhub_launcher::profiles::{game_log, profile_name_of};
use spedhub_launcher::settings::{
    AppPaths, DEFAULT_SERVER, Settings, remove_plugin_config, write_plugin_config,
};
use std::fs;

#[test]
fn einstellungen_mit_token_speichern_und_laden() {
    let paths = AppPaths::new(temp_dir("einstellungen"));

    // Ohne Datei: Vorgaben
    let s = Settings::load(&paths);
    assert_eq!(s.server_url, DEFAULT_SERVER);
    assert_eq!(s.token(), None);

    let mut s = Settings::default();
    s.server_url = "http://spedhub.example:8787".into();
    s.user_name = "Alex".into();
    s.company_id = Some(3);
    s.profile_dir =
        Some("C:\\Users\\Alex\\Documents\\Euro Truck Simulator 2\\profiles\\416C6578".into());
    s.game_dir = Some("D:\\SteamLibrary\\steamapps\\common\\Euro Truck Simulator 2".into());
    let token = "0123456789abcdef0123456789abcdef";
    s.set_token(token).unwrap();
    s.save(&paths).unwrap();

    let loaded = Settings::load(&paths);
    assert_eq!(loaded, s);
    assert_eq!(loaded.token().as_deref(), Some(token));
    assert!(
        !format!("{loaded:?}").contains(token),
        "kein Token in Debug"
    );

    let text = fs::read_to_string(paths.settings()).unwrap();
    if cfg!(windows) {
        // Mit DPAPI verschlüsselt: weder im Klartext noch als Hex in der Datei
        assert_eq!(loaded.token_method(), Some("dpapi"));
        assert!(!text.contains(token));
        let hex: String = token.bytes().map(|b| format!("{b:02x}")).collect();
        assert!(!text.contains(&hex));
    } else {
        assert_eq!(loaded.token_method(), Some("none"));
    }

    let mut cleared = loaded.clone();
    cleared.clear_token();
    assert_eq!(cleared.token(), None);

    // Kaputte Datei: Vorgaben statt Absturz
    fs::write(paths.settings(), "{ kaputt").unwrap();
    assert_eq!(Settings::load(&paths).server_url, DEFAULT_SERVER);
}

#[test]
fn plugin_json_passt_zum_plugin() {
    let dir = temp_dir("plugin");
    let paths = AppPaths::new(&dir);
    let docs = documents("plugin-docs");
    let log = game_log(&docs.ets2);
    let name = profile_name_of(&docs.nordfracht).unwrap();
    assert_eq!(name, "Nordfracht");
    write_plugin_config(
        &paths,
        " http://127.0.0.1:8787/ ",
        "abc123",
        7,
        Some(&name),
        Some(&log),
    )
    .unwrap();

    // Genau so liest das Plugin die Datei
    let config = spedhub_plugin::Config::load(&paths.plugin_config()).unwrap();
    assert_eq!(config.server_url, "http://127.0.0.1:8787");
    assert_eq!(config.token, "abc123");
    assert_eq!(config.company_id, 7);
    assert_eq!(config.profile_name.as_deref(), Some("Nordfracht"));
    assert_eq!(config.game_log.as_deref(), Some(log.as_path()));
    let plugin_paths = spedhub_plugin::Paths::new(&dir);
    assert_eq!(plugin_paths.config(), paths.plugin_config(), "derselbe Ort");

    // Das Plugin erkennt mit dieser Datei das Firmenprofil, wenn das Spiel es wählt
    fs::write(&log, "00:00:12.345 : New profile selected: 'Nordfracht'\n").unwrap();
    let mut gate = spedhub_plugin::ProfileGate::new(
        config.profile_name.as_deref(),
        config.game_log.as_deref(),
    );
    assert_eq!(
        gate.check().0,
        spedhub_plugin::ProfileState::Company("Nordfracht".into())
    );

    // Anderes Firmenprofil (Name mit Umlaut und Leerzeichen): Datei wird ersetzt
    let other = profile_name_of(&docs.mueller).unwrap();
    write_plugin_config(
        &paths,
        "http://127.0.0.1:8787",
        "abc123",
        7,
        Some(&other),
        Some(&log),
    )
    .unwrap();
    let config = spedhub_plugin::Config::load(&paths.plugin_config()).unwrap();
    assert_eq!(config.profile_name.as_deref(), Some("Müller Logistik"));

    // Andere Firma: Datei wird ersetzt
    write_plugin_config(
        &paths,
        "http://127.0.0.1:8787",
        "abc123",
        9,
        Some(&name),
        Some(&log),
    )
    .unwrap();
    assert_eq!(
        spedhub_plugin::Config::load(&paths.plugin_config())
            .unwrap()
            .company_id,
        9
    );

    // Kein Firmenprofil gewählt: die Felder fehlen, das Plugin meldet dann nichts
    write_plugin_config(&paths, "http://127.0.0.1:8787", "abc123", 9, None, None).unwrap();
    let text = fs::read_to_string(paths.plugin_config()).unwrap();
    assert!(
        !text.contains("profile_name") && !text.contains("game_log"),
        "{text}"
    );
    let config = spedhub_plugin::Config::load(&paths.plugin_config()).unwrap();
    assert_eq!(config.profile_name, None);

    remove_plugin_config(&paths).unwrap();
    assert!(!paths.plugin_config().exists());
    remove_plugin_config(&paths).unwrap();
}

#[test]
fn paket_eines_geliehenen_lkw_ablegen() {
    let paths = AppPaths::new(temp_dir("leihe"));
    assert!(paths.load_lease(5).is_none());
    let (save, _) = spedhub_save::Save::read(testdata(QUELLE)).unwrap();
    let package = spedhub_save::export_truck(&save, None).unwrap();
    paths.store_lease(5, &package).unwrap();
    assert_eq!(paths.load_lease(5).unwrap(), package);
    assert!(paths.load_lease(6).is_none());
    paths.remove_lease(5);
    assert!(paths.load_lease(5).is_none());
    paths.remove_lease(5);
    // Kaputte Datei zählt als nicht vorhanden
    fs::write(paths.leases().join("7.json"), "{ kaputt").unwrap();
    assert!(paths.load_lease(7).is_none());
}

#[test]
fn spielordner_fehlt_in_alten_einstellungen() {
    let paths = AppPaths::new(temp_dir("einstellungen-alt"));
    fs::write(
        paths.settings(),
        r#"{ "server_url": "http://spedhub.example", "user_name": "Alex", "company_id": 2 }"#,
    )
    .unwrap();
    let s = Settings::load(&paths);
    assert_eq!(s.game_dir, None);
    assert_eq!(s.user_name, "Alex");
    // Ohne Spielordner steht er auch nicht in der Datei
    s.save(&paths).unwrap();
    assert!(
        !fs::read_to_string(paths.settings())
            .unwrap()
            .contains("game_dir")
    );
}
