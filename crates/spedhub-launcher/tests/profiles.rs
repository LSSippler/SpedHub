//! Profilsuche, Hex-Namen, Speicherplätze.
mod common;

use common::*;
use spedhub_launcher::profiles::{
    decode_hex_name, encode_hex_name, ets2_dir_of, find_profiles, game_log, list_slots,
    newest_slot, next_free_slot_id, profile_name_of,
};

#[test]
fn hex_namen() {
    assert_eq!(decode_hex_name("416C6578").as_deref(), Some("Alex"));
    assert_eq!(
        decode_hex_name("416c6578").as_deref(),
        Some("Alex"),
        "auch klein"
    );
    assert_eq!(encode_hex_name("Alex"), "416C6578");
    // Umlaute sind UTF-8
    assert_eq!(encode_hex_name("Müller"), "4DC3BC6C6C6572");
    assert_eq!(decode_hex_name("4DC3BC6C6C6572").as_deref(), Some("Müller"));
    for bad in ["", "416", "41ZZ", "Kein Hex!", "FF"] {
        assert_eq!(decode_hex_name(bad), None, "{bad:?}");
    }
    for name in ["Nordfracht", "Ä Ö Ü ß", "x"] {
        assert_eq!(
            decode_hex_name(&encode_hex_name(name)).as_deref(),
            Some(name)
        );
    }
}

#[test]
fn profile_in_beiden_ordnern_finden() {
    let docs = documents("profile");
    let profiles = find_profiles(&docs.ets2);
    let summary: Vec<(&str, bool)> = profiles
        .iter()
        .map(|p| (p.name.as_str(), p.steam_cloud))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("Alex Steam", true),
            ("Kein Hex!", false),
            ("Müller Logistik", false),
            ("Nordfracht", false),
        ]
    );
    let nord = profiles.iter().find(|p| p.name == "Nordfracht").unwrap();
    assert_eq!(nord.folder, encode_hex_name("Nordfracht"));
    assert_eq!(nord.dir, docs.nordfracht);
    let steam = profiles.iter().find(|p| p.steam_cloud).unwrap();
    assert_eq!(steam.dir, docs.steam);

    // Kein Ordner, keine Profile, kein Fehler
    assert!(find_profiles(&docs.ets2.join("gibt-es-nicht")).is_empty());
}

#[test]
fn speicherplaetze_und_der_neueste() {
    let docs = documents("slots");
    let slots = list_slots(&docs.nordfracht);
    let ids: Vec<&str> = slots.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["autosave", "1"],
        "neuester zuerst, quicksave ohne game.sii fehlt"
    );
    assert_eq!(slots[0].label(), "Autosave (autosave)");
    assert_eq!(slots[1].name, "Mein Scania");
    assert_eq!(slots[1].label(), "Mein Scania");
    assert_eq!(newest_slot(&docs.nordfracht).unwrap().id, "autosave");

    // Ein älterer Speicherplatz wird neu gespeichert: jetzt ist er der neueste
    add_slot(&docs.nordfracht, "1", QUELLE, None, 0);
    assert_eq!(newest_slot(&docs.nordfracht).unwrap().id, "1");

    // Profil ohne Spielstand: klare Meldung
    assert!(list_slots(&docs.mueller).is_empty());
    let e = newest_slot(&docs.mueller).unwrap_err().to_string();
    assert!(e.contains("noch keinen Spielstand"), "{e}");

    // Nächster freier Platz: 1 ist belegt, quicksave zählt nicht
    assert_eq!(next_free_slot_id(&docs.nordfracht), "2");
    assert_eq!(next_free_slot_id(&docs.mueller), "1");
}

#[test]
fn name_und_log_zum_firmenprofil() {
    let docs = documents("profilname");
    // Derselbe Name wie in der Liste der Profile
    for p in find_profiles(&docs.ets2) {
        assert_eq!(profile_name_of(&p.dir).as_deref(), Some(p.name.as_str()));
    }
    assert_eq!(
        profile_name_of(&docs.mueller).as_deref(),
        Some("Müller Logistik")
    );
    assert_eq!(profile_name_of(&docs.steam).as_deref(), Some("Alex Steam"));
    // Das Log liegt im Dokumente-Ordner des Spiels, neben profiles und steam_profiles
    assert_eq!(ets2_dir_of(&docs.nordfracht), Some(docs.ets2.as_path()));
    assert_eq!(ets2_dir_of(&docs.steam), Some(docs.ets2.as_path()));
    assert_eq!(game_log(&docs.ets2), docs.ets2.join("game.log.txt"));
}
