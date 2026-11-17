//! Versionen lesen, vergleichen und ob Launcher und Server zusammenpassen.
use spedhub_launcher::version::{
    Relation, VERSION, Version, compare, compatible, mismatch_warning,
};

#[test]
fn versionen_lesen() {
    let v = Version::parse("0.2.0").unwrap();
    assert_eq!((v.major, v.minor, v.patch, v.pre), (0, 2, 0, None));
    assert_eq!(Version::parse("v1.10.3").unwrap().to_string(), "1.10.3");
    assert_eq!(Version::parse(" V0.2.1 \n").unwrap().to_string(), "0.2.1");
    assert_eq!(
        Version::parse("0.3.0-rc1").unwrap().pre.as_deref(),
        Some("rc1")
    );
    assert_eq!(Version::parse("0.3.0+abc").unwrap().to_string(), "0.3.0");
    assert_eq!(Version::current().to_string(), VERSION);
}

#[test]
fn unsinn_ist_keine_version() {
    for bad in [
        "",
        "v",
        "0",
        "0.2",
        "0.2.",
        "0.2.x",
        "a.b.c",
        "0.2.0.1",
        "-0.2.0",
        "0.-2.0",
        "0.2.0-",
        "+0.2.0",
        "0 .2.0",
        "vv0.2.0",
        "0.2.0 alt",
        "99999999999999999999.0.0",
        "Release 3",
    ] {
        assert_eq!(Version::parse(bad), None, "{bad:?}");
    }
    assert_eq!(compare("Unsinn", "0.2.0"), None);
    assert_eq!(compare("0.2.0", ""), None);
    assert!(!compatible("Unsinn", "0.2.0"));
    assert!(!compatible("0.2.0", "0.2"));
}

#[test]
fn neuer_gleich_aelter() {
    assert_eq!(compare("v0.3.0", "0.2.0"), Some(Relation::Newer));
    assert_eq!(compare("0.2.1", "0.2.0"), Some(Relation::Newer));
    assert_eq!(compare("1.0.0", "0.99.99"), Some(Relation::Newer));
    assert_eq!(
        compare("0.10.0", "0.9.0"),
        Some(Relation::Newer),
        "Zahlen, nicht Text"
    );
    assert_eq!(compare("v0.2.0", "0.2.0"), Some(Relation::Same));
    assert_eq!(compare("0.2.0+build7", "0.2.0"), Some(Relation::Same));
    assert_eq!(compare("0.1.9", "0.2.0"), Some(Relation::Older));
    // Eine Vorabversion ist älter als die fertige
    assert_eq!(compare("0.3.0-rc1", "0.3.0"), Some(Relation::Older));
    assert_eq!(compare("0.3.0-rc1", "0.2.9"), Some(Relation::Newer));
    assert_eq!(compare("0.3.0-rc2", "0.3.0-rc1"), Some(Relation::Newer));
}

#[test]
fn launcher_und_server_passen_zusammen() {
    assert!(compatible("0.2.0", "0.2.0"));
    assert!(compatible("0.2.0", "v0.2.7"));
    assert!(compatible("0.2.3-rc1", "0.2.0"));
    assert!(!compatible("0.2.0", "0.3.0"));
    assert!(!compatible("0.2.0", "1.2.0"));

    assert_eq!(mismatch_warning("0.2.0", Some("0.2.5")), None);
    let w = mismatch_warning("0.2.0", Some("0.3.0")).unwrap();
    assert!(w.contains("Version 0.3.0") && w.contains("0.2.0"), "{w}");
    assert!(w.contains("passen nicht zusammen"), "{w}");
    let w = mismatch_warning("0.2.0", None).unwrap();
    assert!(w.contains("meldet keine Version"), "{w}");
    let w = mismatch_warning("0.2.0", Some("Unsinn")).unwrap();
    assert!(w.contains("Unsinn"), "{w}");
}
