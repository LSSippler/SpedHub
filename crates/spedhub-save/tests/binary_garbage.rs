//! Kaputte Binärdaten. `decrypt_truck` gerät bei manchen davon in Panik, die Bibliothek muss
//! daraus einen normalen Fehler machen. Eigene Testdatei, weil der Test die Panic-Meldungen
//! ausblendet und das nur geht, solange kein anderer Test im selben Prozess läuft.
mod common;

use common::Lcg;
use spedhub_save::{Error, Save};
use std::sync::atomic::{AtomicUsize, Ordering};

static PANICS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn kaputte_binaerdaten_werden_zu_fehlern() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {
        PANICS.fetch_add(1, Ordering::SeqCst);
    }));

    let mut rng = Lcg(1);
    let mut decode_errors = 0;
    for i in 0..3000u32 {
        // Gültige Kennung und Version, dahinter Zufall. Daran scheitert decrypt_truck 1.3.6
        // in einigen Fällen mit einer Panic.
        let mut b = if i % 4 == 0 {
            b"ScsC".to_vec()
        } else {
            b"BSII".to_vec()
        };
        b.extend((1 + i % 3).to_le_bytes());
        b.extend((0..rng.below(300)).map(|_| rng.next() as u8));
        if let Err(Error::Decode(_)) = Save::from_bytes(&b) {
            decode_errors += 1;
        }
    }

    std::panic::set_hook(default_hook);
    // Nur zur Info: wie oft decrypt_truck in Panik geraten ist und abgefangen wurde.
    println!("abgefangene Panics: {}", PANICS.load(Ordering::SeqCst));
    assert!(
        decode_errors > 2900,
        "fast alles ist ein Lesefehler: {decode_errors}"
    );
}
