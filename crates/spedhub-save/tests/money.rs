//! Kontostand in game.sii und info.sii.
mod common;

use common::*;
use spedhub_save::{Error, Save, info_money, money, set_info_money, set_money, write_saves};
use std::path::Path;

#[test]
fn kontostand_lesen_und_in_beiden_dateien_setzen() {
    let mut game = load(ZIEL);
    let mut info = load(INFO);
    assert_eq!(money(&game).unwrap(), 2000);
    assert_eq!(info_money(&info).unwrap(), 2000);
    assert_eq!(money(&load(QUELLE)).unwrap(), 38101);

    set_money(&mut game, 12345).unwrap();
    set_info_money(&mut info, 12345).unwrap();
    assert_eq!(
        game.dump(),
        text(ZIEL).replace(" money_account: 2000\n", " money_account: 12345\n")
    );
    // info.sii hat CRLF und einen Zeilenumbruch am Ende wie beim Spiel. Das bleibt so.
    assert!(text(INFO).contains("\r\n"));
    assert_eq!(
        info.dump(),
        text(INFO).replace(
            "info_money_account: 2000\r\n",
            "info_money_account: 12345\r\n"
        )
    );
    assert!(info.dump().ends_with("}\r\n\r\n}\r\n"));

    let dir = temp_dir("geld");
    let (g, i) = (dir.join("game.sii"), dir.join("info.sii"));
    let items: Vec<(&Path, &Save)> = vec![(&g, &game), (&i, &info)];
    write_saves(&items).unwrap();
    assert_eq!(money(&Save::read(&g).unwrap().0).unwrap(), 12345);
    assert_eq!(info_money(&Save::read(&i).unwrap().0).unwrap(), 12345);

    // Schulden sind erlaubt, das Spiel kennt den Dispo
    set_money(&mut game, -500).unwrap();
    assert_eq!(money(&game).unwrap(), -500);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fehlerfaelle_beim_kontostand() {
    // Falsche Datei: info.sii hat keine Bank, game.sii keinen save_container
    assert!(matches!(money(&load(INFO)), Err(Error::Structure(_))));
    assert!(matches!(info_money(&load(ZIEL)), Err(Error::Structure(_))));
    assert!(matches!(
        set_money(&mut load(INFO), 1),
        Err(Error::Structure(_))
    ));

    // Kein ganzzahliger Wert
    let mut game = load(ZIEL);
    game.set("_nameless.20e.2752.4a68", "money_account", "&3f800000")
        .unwrap();
    assert!(matches!(money(&game), Err(Error::InvalidValue { .. })));
    let before = game.clone();
    assert!(set_money(&mut game, 5).is_err());
    assert_eq!(game, before);
}

#[test]
fn name_und_zeit_des_speicherplatzes() {
    let mut info = load(INFO);
    assert_eq!(spedhub_save::info_name(&info).unwrap(), "");
    spedhub_save::set_info_name(&mut info, "SpedHub: SPED 001").unwrap();
    spedhub_save::set_info_file_time(&mut info, 1_800_000_000).unwrap();
    assert_eq!(spedhub_save::info_name(&info).unwrap(), "SpedHub: SPED 001");
    assert_eq!(
        info.dump(),
        text(INFO)
            .replace(" name: \"\"\r\n", " name: \"SpedHub: SPED 001\"\r\n")
            .replace(" file_time: 1794131316\r\n", " file_time: 1800000000\r\n")
    );
    for bad in ["", "mit \"Anführung\"", "Müller", &"x".repeat(65)] {
        assert!(
            matches!(
                spedhub_save::set_info_name(&mut info, bad),
                Err(Error::InvalidInput(_))
            ),
            "{bad:?}"
        );
    }
    // game.sii hat keinen save_container
    assert!(spedhub_save::set_info_name(&mut load(ZIEL), "x").is_err());
}
