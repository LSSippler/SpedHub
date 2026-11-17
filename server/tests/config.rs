//! Start-Einstellungen: Datenbank und Adresse aus Kommandozeile und Umgebung.
use spedhub_server::config::{Command, Config, Env, local_data_dir, parse};
use std::path::PathBuf;

fn run(args: &[&str], env: &Env) -> Config {
    match parse(args, env).unwrap() {
        Command::Run(c) => c,
        other => panic!("{other:?}"),
    }
}

fn windows_env() -> Env {
    Env {
        local_app_data: Some("C:\\Users\\Alex\\AppData\\Local".into()),
        home: None,
        ..Env::default()
    }
}

#[test]
fn ohne_optionen_wie_bisher() {
    let c = run(&[], &windows_env());
    assert_eq!(c.db, PathBuf::from("spedhub.db"));
    assert_eq!(c.addr.to_string(), "127.0.0.1:8787");
    assert_eq!(c.create_dir, None);
}

#[test]
fn local_legt_die_datenbank_in_den_benutzerordner() {
    let base = PathBuf::from("C:\\Users\\Alex\\AppData\\Local");
    let c = run(&["--local"], &windows_env());
    let dir = base.join("SpedHub").join("server");
    assert_eq!(c.db, dir.join("spedhub.db"));
    assert_eq!(c.create_dir, Some(dir));
    assert_eq!(c.addr.to_string(), "127.0.0.1:8787");

    // Linux: ohne LOCALAPPDATA unter HOME
    let env = Env {
        home: Some("/home/alex".into()),
        ..Env::default()
    };
    let c = run(&["--local"], &env);
    assert_eq!(
        c.db,
        PathBuf::from("/home/alex/.local/share/SpedHub/server/spedhub.db")
    );
    assert_eq!(
        local_data_dir(&env),
        Some(PathBuf::from("/home/alex/.local/share/SpedHub/server"))
    );

    // Ohne beides geht --local nicht
    let e = parse(["--local"], &Env::default()).unwrap_err();
    assert!(e.contains("LOCALAPPDATA"), "{e}");
}

#[test]
fn umgebungsvariablen_gehen_vor() {
    let mut env = windows_env();
    env.spedhub_db = Some("D:\\daten\\firma.db".into());
    env.spedhub_addr = Some("0.0.0.0:9000".into());
    let c = run(&["--local"], &env);
    assert_eq!(c.db, PathBuf::from("D:\\daten\\firma.db"));
    assert_eq!(
        c.create_dir, None,
        "kein Ordner anlegen, der Pfad ist vorgegeben"
    );
    assert_eq!(c.addr.to_string(), "0.0.0.0:9000");

    env.spedhub_addr = Some("kaputt".into());
    let e = parse(["--local"], &env).unwrap_err();
    assert!(e.contains("SPEDHUB_ADDR"), "{e}");
}

#[test]
fn version_hilfe_und_unbekanntes() {
    assert_eq!(parse(["--version"], &Env::default()), Ok(Command::Version));
    assert_eq!(
        parse(["--local", "-V"], &Env::default()),
        Ok(Command::Version)
    );
    assert_eq!(parse(["--help"], &Env::default()), Ok(Command::Help));
    let e = parse(["--lokal"], &Env::default()).unwrap_err();
    assert!(e.starts_with("Unbekannte Option --lokal"), "{e}");
}
