//! Abläufe, die Server und Spielstand zusammenbringen.
//!
//! - [`lease_package`]: Bauplan einer laufenden Leihe, aus `leases\` oder vom Server
//! - [`sync_and_write`]: Geldabgleich, auf Wunsch zusammen mit dem Einsetzen eines Lkw
//! - [`profile_to_report`]: muss das Firmenprofil (wieder) beim Server gemeldet werden?

use crate::client::{Client, Company, SyncResult};
use crate::error::{Error, Result};
use crate::savegame::{Deployed, package_from_checkout, prepare_slot, unix_seconds, write_slot};
use crate::settings::AppPaths;
use crate::system::GameCheck;
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
pub fn lease_package(
    client: &Client,
    paths: &AppPaths,
    truck_id: i64,
) -> Result<(TruckPackage, PackageSource)> {
    if let Some(p) = paths.load_lease(truck_id) {
        return Ok((p, PackageSource::Local));
    }
    let lease = client.lease(truck_id)?;
    let package = package_from_checkout(&lease.truck)?;
    // Scheitert das Ablegen, geht es trotzdem weiter: Beim nächsten Mal fragt er wieder.
    let _ = paths.store_lease(truck_id, &package);
    Ok((package, PackageSource::Server))
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
