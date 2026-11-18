//! Abläufe, die Server und Spielstand zusammenbringen.
//!
//! - [`lease_package`]: Bauplan einer laufenden Leihe, aus `leases\` oder vom Server, mit dem
//!   Kennzeichen vom Server
//! - [`return_truck`]: Rückgabe mit dem Kennzeichen aus dem Spielstand, falls es frei ist
//! - [`sync_and_write`]: Geldabgleich, auf Wunsch zusammen mit dem Einsetzen eines Lkw
//! - [`profile_to_report`]: muss das Firmenprofil (wieder) beim Server gemeldet werden?
//! - [`ProfileReport`]: läuft die Meldung schon, oder muss sie nach einem Fehler noch einmal?

use crate::client::{Checkin, Client, Company, SyncResult};
use crate::error::{Error, Result};
use crate::savegame::{
    Deployed, package_from_checkout, prepare_slot, set_company_plate, unix_seconds, write_slot,
};
use crate::settings::AppPaths;
use crate::system::GameCheck;
use serde_json::Value;
use spedhub_save::TruckPackage;
use std::{path::Path, time::SystemTime};

/// Woher der Bauplan kam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageSource {
    /// Aus `leases\<id>.json` auf diesem Rechner
    Local,
    /// Vom Server geholt und lokal abgelegt
    Server,
}

/// Der Bauplan eines geliehenen Lkw. Liegt er nicht in `leases\`, etwa auf einem anderen
/// Rechner, holt der Launcher ihn vom Server und legt ihn dort ab. Die lokale Ablage ist nur
/// ein Zwischenspeicher.
///
/// `plate` ist das Kennzeichen des Lkw auf dem Server, wie es im Fuhrpark steht. Es kommt auch
/// in einen Bauplan aus der Ablage, die von einem älteren Launcher noch das Kennzeichen aus dem
/// Spielstand des Chefs tragen kann.
pub fn lease_package(
    client: &Client,
    paths: &AppPaths,
    truck_id: i64,
    plate: &str,
) -> Result<(TruckPackage, PackageSource)> {
    if let Some(mut p) = paths.load_lease(truck_id) {
        set_company_plate(&mut p, plate)?;
        return Ok((p, PackageSource::Local));
    }
    let lease = client.lease(truck_id)?;
    let package = package_from_checkout(&lease.truck)?;
    // Scheitert das Ablegen, geht es trotzdem weiter: Beim nächsten Mal fragt er wieder.
    let _ = paths.store_lease(truck_id, &package);
    Ok((package, PackageSource::Server))
}

/// Ergebnis einer Rückgabe.
#[derive(Debug, Clone, PartialEq)]
pub struct Returned {
    pub checkin: Checkin,
    /// Das Kennzeichen aus dem Spielstand, das der Server nicht übernommen hat, weil ein
    /// anderer Firmen-Lkw es trägt. Der Lkw behält dann sein Kennzeichen vom Server.
    pub plate_taken: Option<String>,
}

/// Lkw zurückgeben. Mit `new_plate` meldet der Launcher das Kennzeichen, das das Spiel dem
/// Lkw gegeben hat. Trägt das schon ein anderer Firmen-Lkw (409), etwa weil das Spiel das
/// Wunschkennzeichen eines Profils vergeben hat, geht die Rückgabe ohne neues Kennzeichen
/// durch: Beim nächsten Einsetzen schreibt der Launcher ohnehin das Kennzeichen vom Server.
pub fn return_truck(
    client: &Client,
    truck_id: i64,
    odometer_km: i64,
    state: Option<&Value>,
    new_plate: Option<&str>,
) -> Result<Returned> {
    match client.checkin(truck_id, odometer_km, state, new_plate) {
        Ok(checkin) => Ok(Returned {
            checkin,
            plate_taken: None,
        }),
        Err(Error::Server { status: 409, .. }) if new_plate.is_some() => {
            let checkin = client.checkin(truck_id, odometer_km, state, None)?;
            Ok(Returned {
                checkin,
                plate_taken: new_plate.map(str::to_string),
            })
        }
        Err(e) => Err(e),
    }
}

/// Ergebnis eines Abgleichs mit neuem Speicherplatz.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncOutcome {
    pub deployed: Deployed,
    pub sync: SyncResult,
    /// Hat der Server die Bestätigung bekommen, dass der Betrag im Spielstand steht? Wenn
    /// nicht, rechnet er beim nächsten Abgleich vom vorgefundenen Stand aus und bucht die
    /// Differenz. Die Oberfläche sagt das dann dazu.
    pub confirmed: bool,
}

/// Geldabgleich mit neuem Speicherplatz, mit `package` zusammen mit dem Einsetzen des Lkw.
///
/// 1. Spiel prüfen, neuesten Speicherplatz lesen, Kontostand und Erfahrung auslesen, den Lkw
///    einsetzen (alles im Speicher). Geht hier etwas schief, hat der Server nichts erfahren.
/// 2. Abgleich beim Server, mit der Änderungszeit des gelesenen Spielstands (`saved_at`):
///    bucht Betriebskosten oder legt einen ungeklärten Betrag an und sagt, welcher Kontostand
///    in den Spielstand kommt.
/// 3. Diesen Betrag in `game.sii` und `info.sii` setzen und den neuen Speicherplatz schreiben,
///    abgesichert wie beim Einsetzen. Läuft das Spiel inzwischen, wird nichts geschrieben.
/// 4. Dem Server melden, dass der Betrag im Spielstand steht, mit `now` als `written_at`
///    (dieselbe Zeit, die als `file_time` in `info.sii` steht).
pub fn sync_and_write(
    client: &Client,
    company_id: i64,
    profile_dir: &Path,
    package: Option<&TruckPackage>,
    backup_root: &Path,
    game: &dyn GameCheck,
    now: SystemTime,
) -> Result<SyncOutcome> {
    let mut prepared = prepare_slot(profile_dir, package, game)?;
    let sync = client.sync(
        company_id,
        prepared.ingame_money,
        prepared.xp,
        Some(prepared.saved_at),
    )?;
    prepared.set_money(sync.write_money)?;
    let deployed = match write_slot(prepared, profile_dir, backup_root, game, now) {
        Ok(d) => d,
        Err(Error::GameRunning) => return Err(Error::GameRunning),
        Err(e) => {
            return Err(Error::Other(format!(
                "{e}. Der Abgleich ist beim Server gebucht, der Spielstand blieb aber unverändert. Der nächste Abgleich rechnet vom vorgefundenen Kontostand weiter"
            )));
        }
    };
    let written_at = unix_seconds(now) as i64;
    let confirmed = client
        .sync_written(company_id, sync.sync_id, Some(written_at))
        .is_ok();
    Ok(SyncOutcome {
        deployed,
        sync,
        confirmed,
    })
}

/// Das Firmenprofil, das der Launcher dem Server melden soll: der Name des gewählten Profils,
/// wenn der Server für `user_id` ein anderes oder keins kennt. `None`, wenn nichts zu tun ist
/// oder kein Profil gewählt ist. Ein gemeldetes Profil löscht der Launcher nie, ohne Wahl
/// bleibt es beim Server, wie es ist.
pub fn profile_to_report(company: &Company, user_id: i64, chosen: Option<&str>) -> Option<String> {
    let chosen = chosen.map(str::trim).filter(|c| !c.is_empty())?;
    let reported = company
        .members
        .iter()
        .find(|m| m.user_id == user_id)
        .and_then(|m| m.profile_name.as_deref());
    (reported != Some(chosen)).then(|| chosen.to_string())
}

/// Welche Meldung des Firmenprofils gerade läuft oder geklappt hat, je Firma und Name.
/// Verhindert, dass dieselbe Meldung nach jedem Laden wieder rausgeht, etwa bei einem älteren
/// Server, der das Profil nicht zurückgibt. Eine fehlgeschlagene Meldung vergisst sie, damit
/// das nächste Laden sie wiederholt: Bis der Server das Profil kennt, lehnt er Ereignisse aus
/// dem neuen Profil ab.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProfileReport {
    sent: Option<(i64, String)>,
}

impl ProfileReport {
    /// Soll diese Meldung jetzt rausgehen? Dann merkt sie sich sie als laufend.
    pub fn start(&mut self, company_id: i64, name: &str) -> bool {
        let attempt = Some((company_id, name.to_string()));
        if self.sent == attempt {
            return false;
        }
        self.sent = attempt;
        true
    }

    /// Ergebnis einer Meldung. Bei einem Fehler wird sie vergessen, sofern inzwischen keine
    /// andere läuft.
    pub fn finished(&mut self, company_id: i64, name: &str, ok: bool) {
        if !ok
            && self
                .sent
                .as_ref()
                .is_some_and(|(c, n)| *c == company_id && n == name)
        {
            self.sent = None;
        }
    }
}

/// Kurze Beschreibung des Abgleichs für die Oberfläche.
pub fn describe(sync: &SyncResult) -> String {
    let euro = |v: i64| format!("{v} €");
    let mut parts = Vec::new();
    if sync.first {
        parts.push(format!(
            "Erster Abgleich, nichts gebucht. Im Spiel standen {}",
            euro(sync.ingame_money)
        ));
    } else {
        for b in &sync.booked {
            if b.kind == "betriebskosten" {
                parts.push(format!("Betriebskosten {} gebucht", euro(-b.amount)));
            }
        }
        if let Some(u) = &sync.unexplained {
            parts.push(format!(
                "{} mehr im Spiel als erklärt, nicht gutgeschrieben. Der Chef sieht das",
                euro(u.amount)
            ));
        }
        if parts.is_empty() {
            parts.push("Kontostand passt, nichts gebucht".into());
        }
    }
    let budget = if sync.budget > 0 {
        format!(" plus Anschaffungsbudget {}", euro(sync.budget))
    } else {
        String::new()
    };
    parts.push(format!(
        "Neuer Kontostand im Spiel: {} (Lohnkonto {} plus Vorschuss {}{budget})",
        euro(sync.write_money),
        euro(sync.wallet),
        euro(sync.advance)
    ));
    parts.join(". ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profil_meldung_nach_fehler_wiederholen() {
        let mut r = ProfileReport::default();
        assert!(r.start(1, "B"));
        // Läuft schon: nicht noch einmal
        assert!(!r.start(1, "B"));
        // Server nicht erreichbar: beim nächsten Laden wieder
        r.finished(1, "B", false);
        assert!(r.start(1, "B"));
        // Geklappt: bleibt gemerkt
        r.finished(1, "B", true);
        assert!(!r.start(1, "B"));
        // Ein anderes Profil oder eine andere Firma geht raus
        assert!(r.start(1, "C"));
        assert!(r.start(2, "C"));
        // Ein später gemeldeter Fehler einer älteren Meldung vergisst die neue nicht
        r.finished(1, "C", false);
        assert!(!r.start(2, "C"));
    }
}
