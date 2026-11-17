//! Kommandozeile für die Save-Bibliothek. Aufruf ohne Argumente zeigt die Hilfe.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use spedhub_save::{
    Error, InsertOptions, Save, TruckPackage, check, export_truck, info_money, insert_truck, money,
    plate_text, remove_truck, set_info_money, set_money, write_save, write_saves,
};
use std::{collections::HashMap, fs, path::Path, process::ExitCode};

const HELP: &str = "spedhub-save: Spielstände von ETS2 lesen und bearbeiten

Gelesen wird jedes Format (ScsC, BSII, SiiN), geschrieben wird immer Klartext.
Vor jedem Schreiben läuft die Integritätsprüfung, eine vorhandene Datei wird vorher
als .bak daneben gesichert.

Befehle:
  decode <eingabe> <ausgabe>
      Spielstand in Klartext umwandeln.
  check <datei>
      Integrität prüfen und Probleme auflisten.
  export-truck <spielstand> <paket.json> [--plate <kennzeichen>]
      Lkw als Paket speichern. Ohne --plate der Lkw, den der Spieler gerade fährt.
  insert-truck <spielstand> <paket.json> <ausgabe> [--keep-names]
      Lkw aus einem Paket einsetzen und zum aktiven Lkw machen.
      --keep-names übernimmt die Unit-Namen aus der Quelle statt neue zu vergeben.
  remove-truck <spielstand> <kennzeichen> <ausgabe> [--replacement <kennzeichen>]
      Eigenen Lkw entfernen. Beim aktiven Lkw ist --replacement Pflicht.
  money <game.sii> [--info <info.sii>] [--set <betrag>]
      Kontostand anzeigen. Mit --set in game.sii und info.sii setzen.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(m)) => {
            eprintln!("{m}\n\n{HELP}");
            ExitCode::from(2)
        }
        Err(Failure::Lib(e)) => {
            eprintln!("Fehler: {e}");
            ExitCode::from(1)
        }
    }
}

enum Failure {
    Usage(String),
    Lib(Error),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Lib(e)
    }
}

type Run = Result<(), Failure>;

/// Positionsargumente und Optionen. Optionen mit Wert stehen in `VALUE_FLAGS`.
struct Args {
    positional: Vec<String>,
    values: HashMap<String, String>,
    switches: Vec<String>,
}

const VALUE_FLAGS: &[&str] = &["--plate", "--replacement", "--info", "--set"];
const SWITCHES: &[&str] = &["--keep-names"];

impl Args {
    fn parse(raw: &[String]) -> Result<Args, Failure> {
        let mut args = Args {
            positional: Vec::new(),
            values: HashMap::new(),
            switches: Vec::new(),
        };
        let mut it = raw.iter();
        while let Some(a) = it.next() {
            if VALUE_FLAGS.contains(&a.as_str()) {
                let v = it
                    .next()
                    .ok_or_else(|| Failure::Usage(format!("Nach {a} fehlt ein Wert")))?;
                args.values.insert(a.clone(), v.clone());
            } else if SWITCHES.contains(&a.as_str()) {
                args.switches.push(a.clone());
            } else if a.starts_with("--") {
                return Err(Failure::Usage(format!("Unbekannte Option {a}")));
            } else {
                args.positional.push(a.clone());
            }
        }
        Ok(args)
    }

    fn expect(&self, command: &str, names: &[&str]) -> Result<Vec<&str>, Failure> {
        if self.positional.len() != names.len() {
            return Err(Failure::Usage(format!(
                "{command} braucht {} Angaben: {}",
                names.len(),
                names.join(" ")
            )));
        }
        Ok(self.positional.iter().map(String::as_str).collect())
    }

    fn allow(&self, command: &str, allowed: &[&str]) -> Run {
        for k in self.values.keys().chain(&self.switches) {
            if !allowed.contains(&k.as_str()) {
                return Err(Failure::Usage(format!(
                    "{command} kennt die Option {k} nicht"
                )));
            }
        }
        Ok(())
    }

    fn value(&self, flag: &str) -> Option<&str> {
        self.values.get(flag).map(String::as_str)
    }
}

fn run(raw: &[String]) -> Run {
    let Some((command, rest)) = raw.split_first() else {
        println!("{HELP}");
        return Ok(());
    };
    let args = Args::parse(rest)?;
    match command.as_str() {
        "decode" => decode(&args),
        "check" => check_cmd(&args),
        "export-truck" => export(&args),
        "insert-truck" => insert(&args),
        "remove-truck" => remove(&args),
        "money" => money_cmd(&args),
        "help" | "--help" | "-h" => {
            println!("{HELP}");
            Ok(())
        }
        other => Err(Failure::Usage(format!("Unbekannter Befehl {other}"))),
    }
}

fn read(path: &str) -> Result<Save, Failure> {
    Ok(Save::read(path)?.0)
}

fn report_written(w: &spedhub_save::Written) {
    match &w.backup {
        Some(b) => println!(
            "Geschrieben: {} (Backup: {})",
            w.path.display(),
            b.display()
        ),
        None => println!("Geschrieben: {}", w.path.display()),
    }
}

fn decode(args: &Args) -> Run {
    args.allow("decode", &[])?;
    let p = args.expect("decode", &["<eingabe>", "<ausgabe>"])?;
    let (input, output) = (
        p.first().copied().unwrap_or_default(),
        p.get(1).copied().unwrap_or_default(),
    );
    let (save, format) = Save::read(input)?;
    println!(
        "Gelesen: {input}, {}, {} Units",
        format.label(),
        save.units().len()
    );
    report_written(&write_save(output, &save)?);
    Ok(())
}

fn check_cmd(args: &Args) -> Run {
    args.allow("check", &[])?;
    let p = args.expect("check", &["<datei>"])?;
    let path = p.first().copied().unwrap_or_default();
    let (save, format) = Save::read(path)?;
    let report = check(&save);
    println!("{path}: {}, {} Units", format.label(), report.units);
    if report.is_ok() {
        println!("Keine Probleme gefunden");
        Ok(())
    } else {
        for problem in &report.problems {
            println!("  {problem}");
        }
        Err(Failure::Lib(Error::Integrity(report.problems)))
    }
}

fn export(args: &Args) -> Run {
    args.allow("export-truck", &["--plate"])?;
    let p = args.expect("export-truck", &["<spielstand>", "<paket.json>"])?;
    let (input, output) = (
        p.first().copied().unwrap_or_default(),
        p.get(1).copied().unwrap_or_default(),
    );
    let save = read(input)?;
    let package = export_truck(&save, args.value("--plate"))?;
    let json = package.to_json()?;
    fs::write(output, json).map_err(|e| Error::Io {
        path: output.into(),
        message: e.to_string(),
    })?;
    println!(
        "Lkw {} ({}) mit {} Units nach {output} exportiert",
        plate_text(&package.license_plate),
        package.model,
        package.units.len()
    );
    Ok(())
}

fn insert(args: &Args) -> Run {
    args.allow("insert-truck", &["--keep-names"])?;
    let p = args.expect(
        "insert-truck",
        &["<spielstand>", "<paket.json>", "<ausgabe>"],
    )?;
    let (input, package_path, output) = (
        p.first().copied().unwrap_or_default(),
        p.get(1).copied().unwrap_or_default(),
        p.get(2).copied().unwrap_or_default(),
    );
    let mut save = read(input)?;
    let text = fs::read_to_string(package_path).map_err(|e| Error::Io {
        path: package_path.into(),
        message: e.to_string(),
    })?;
    let package = TruckPackage::from_json(&text)?;
    let options = InsertOptions {
        keep_names: args.switches.iter().any(|s| s == "--keep-names"),
    };
    let report = insert_truck(&mut save, &package, &options)?;
    if report.removed_quick_job {
        println!(
            "Schnellauftrag mit Firmen-Lkw entfernt ({} Units)",
            report.removed_units
        );
    }
    if let Some(m) = &report.moved_truck {
        println!("Bisheriger Lkw {m} steht jetzt auf einem freien Platz der Heimatgarage");
    }
    println!(
        "Lkw {} eingesetzt als {}",
        plate_text(&package.license_plate),
        report.vehicle
    );
    report_written(&write_save(output, &save)?);
    Ok(())
}

fn remove(args: &Args) -> Run {
    args.allow("remove-truck", &["--replacement"])?;
    let p = args.expect(
        "remove-truck",
        &["<spielstand>", "<kennzeichen>", "<ausgabe>"],
    )?;
    let (input, plate, output) = (
        p.first().copied().unwrap_or_default(),
        p.get(1).copied().unwrap_or_default(),
        p.get(2).copied().unwrap_or_default(),
    );
    let mut save = read(input)?;
    let report = remove_truck(&mut save, plate, args.value("--replacement"))?;
    println!("Lkw {plate} entfernt ({} Units)", report.removed_units);
    if let Some(r) = &report.replacement {
        println!("Aktiver Lkw ist jetzt {r}");
    }
    report_written(&write_save(output, &save)?);
    Ok(())
}

fn money_cmd(args: &Args) -> Run {
    args.allow("money", &["--info", "--set"])?;
    let p = args.expect("money", &["<game.sii>"])?;
    let game_path = p.first().copied().unwrap_or_default();
    let mut game = read(game_path)?;
    let mut info = match args.value("--info") {
        Some(path) => Some((path, read(path)?)),
        None => None,
    };
    println!("Kontostand in {game_path}: {}", money(&game)?);
    if let Some((path, save)) = &info {
        println!("Kontostand in {path}: {}", info_money(save)?);
    }
    let Some(raw) = args.value("--set") else {
        return Ok(());
    };
    let amount: i64 = raw
        .parse()
        .map_err(|_| Failure::Usage(format!("{raw} ist kein ganzer Betrag in Euro")))?;
    set_money(&mut game, amount)?;
    if let Some((_, save)) = &mut info {
        set_info_money(save, amount)?;
    }
    let mut items: Vec<(&Path, &Save)> = vec![(Path::new(game_path), &game)];
    if let Some((path, save)) = &info {
        items.push((Path::new(*path), save));
    }
    for w in write_saves(&items)? {
        report_written(&w);
    }
    println!("Kontostand auf {amount} gesetzt");
    Ok(())
}
