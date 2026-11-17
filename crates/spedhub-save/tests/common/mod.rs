//! Gemeinsame Hilfen für die Tests.
#![allow(dead_code)]

use spedhub_save::{Save, sii::is_nameless};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

pub const QUELLE: &str = "quelle_mit_lkw.sii";
pub const ZIEL: &str = "ziel_schnellauftrag.sii";
pub const ERWARTET: &str = "erwartet_nach_einsetzen.sii";
pub const INFO: &str = "ziel_info.sii";
/// `erwartet_nach_einsetzen.sii`, nachdem das Spiel gespeichert hat: neue Unit-Namen, neues
/// Kennzeichen mit Markup, 425 km mehr.
pub const NACH_SPIEL: &str = "nach_spiel_neues_kennzeichen.sii";

pub fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(name)
}

pub fn text(name: &str) -> String {
    fs::read_to_string(testdata(name)).unwrap()
}

pub fn load(name: &str) -> Save {
    Save::parse(&text(name)).unwrap()
}

/// Ein frischer, leerer Ordner für einen Test.
pub fn temp_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "spedhub-save-test-{}-{n}-{label}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Text des Spielstands, in dem alle `_nameless`-Namen durch Nummern in der Reihenfolge
/// ersetzt sind, in der eine Breitensuche von der ersten Unit aus sie findet. Zwei
/// Spielstände, die sich nur in den vergebenen Namen unterscheiden, ergeben denselben Text.
pub fn canonical(save: &Save) -> String {
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut queue = VecDeque::new();
    if let Some(first) = save.units().first() {
        queue.push_back(first.name.clone());
    }
    let assign = |name: &str, ids: &mut HashMap<String, String>| {
        if is_nameless(name) && !ids.contains_key(name) {
            let id = format!("#{}", ids.len());
            ids.insert(name.to_string(), id);
        }
    };
    let mut visited = std::collections::HashSet::new();
    while let Some(n) = queue.pop_front() {
        if !visited.insert(n.clone()) {
            continue;
        }
        assign(&n, &mut ids);
        let unit = save.unit(&n).unwrap();
        for r in save.refs(unit) {
            assign(&r, &mut ids);
            queue.push_back(r);
        }
    }
    let map = |v: &str| ids.get(v).cloned().unwrap_or_else(|| v.to_string());
    let mut out = String::new();
    for u in save.units() {
        out.push_str(&format!("{} : {} {{\n", u.kind, map(&u.name)));
        for f in &u.fields {
            out.push_str(&format!(" {}: {}\n", f.key, map(&f.value)));
        }
        out.push_str("}\n");
    }
    out
}

/// Einfacher Zufallsgenerator, damit die Tests ohne weitere Abhängigkeit auskommen und
/// immer gleich laufen.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
