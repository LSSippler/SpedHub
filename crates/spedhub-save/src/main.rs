//! Kommandozeile für die Save-Bibliothek. Aufruf ohne Argumente zeigt die Hilfe.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use spedhub_save::{
    Error, Factor, InsertOptions, ModOptions, Save, ScaleOptions, TruckPackage, build_mod, check,
    economy, export_truck, info_money, insert_truck, money, plate_text, remove_truck,
    scale_economy, set_info_money, set_money, write_mod, write_save, write_saves,
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
  economy-mod <economy_data.sii> [-o <ziel.scs>] [--factor 0.01] [--also <schlüssel>]...
              [--version <x>] [--compatible 1.62.*] [--icon <bild.jpg>] [--force]
      Wirtschafts-Mod bauen: die Werte für den Erlös aus economy_data.sii mal Faktor,
      als spedhub_economy.scs. Ein vorhandenes Ziel wird nur mit --force ersetzt.
      --also skaliert weitere Schlüssel mit, mehrfach oder mit Komma getrennt.
      Anleitung in docs/wirtschafts-mod.md.
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
        Err(Failure::Guide(m, code)) => {
            eprintln!("{m}");
            ExitCode::from(code)
        }
    }
}

enum Failure {
    Usage(String),
    Lib(Error),
    /// Meldung mit Anleitung statt der Hilfe, und der Exit-Code
    Guide(String, u8),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Lib(e)
    }
}

type Run = Result<(), Failure>;

/// Positionsargumente und Optionen. Optionen mit Wert stehen in `VALUE_FLAGS`. Eine Option
/// darf mehrfach vorkommen, [`Args::value`] nimmt dann die letzte, [`Args::all`] alle.
struct Args {
    positional: Vec<String>,
    values: HashMap<String, Vec<String>>,
    switches: Vec<String>,
}

const VALUE_FLAGS: &[&str] = &[
    "--plate",
    "--replacement",
    "--info",
    "--set",
    "--output",
    "--factor",
    "--also",
    "--version",
    "--compatible",
    "--icon",
];
const SWITCHES: &[&str] = &["--keep-names", "--force"];
/// Kurze Optionen und ihre langen Namen
const ALIASES: &[(&str, &str)] = &[("-o", "--output")];

impl Args {
    fn parse(raw: &[String]) -> Result<Args, Failure> {
        let mut args = Args {
            positional: Vec::new(),
            values: HashMap::new(),
            switches: Vec::new(),
        };
        let mut it = raw.iter();
        while let Some(a) = it.next() {
            let a = &ALIASES
                .iter()
                .find(|(short, _)| short == a)
                .map_or_else(|| a.clone(), |(_, long)| long.to_string());
            if VALUE_FLAGS.contains(&a.as_str()) {
                let v = it
                    .next()
                    .ok_or_else(|| Failure::Usage(format!("Nach {a} fehlt ein Wert")))?;
                args.values.entry(a.clone()).or_default().push(v.clone());
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
        self.values
            .get(flag)
            .and_then(|v| v.last())
            .map(String::as_str)
    }

    fn all(&self, flag: &str) -> Vec<&str> {
        self.values
            .get(flag)
            .map(|v| v.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    fn switch(&self, flag: &str) -> bool {
        self.switches.iter().any(|s| s == flag)
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
        "economy-mod" => economy_mod(&args),
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
        keep_names: args.switch("--keep-names"),
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

fn economy_mod(args: &Args) -> Run {
    args.allow(
        "economy-mod",
        &[
            "--output",
            "--factor",
            "--also",
            "--version",
            "--compatible",
            "--icon",
            "--force",
        ],
    )?;
    let input = match args.positional.as_slice() {
        [] => {
            return Err(Failure::Guide(
                format!(
                    "economy-mod braucht die Datei economy_data.sii aus dem Spiel. Sie gehört SCS und liegt deshalb nicht bei SpedHub.\n\n{}",
                    economy::GUIDE
                ),
                2,
            ));
        }
        [one] => one.as_str(),
        _ => {
            return Err(Failure::Usage(
                "economy-mod braucht genau eine Angabe: <economy_data.sii>".into(),
            ));
        }
    };
    if !Path::new(input).is_file() {
        return Err(Failure::Guide(
            format!("Die Datei {input} gibt es nicht.\n\n{}", economy::GUIDE),
            1,
        ));
    }
    let factor = Factor::parse(args.value("--factor").unwrap_or(economy::DEFAULT_FACTOR))?;
    let also: Vec<String> = args
        .all("--also")
        .iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(String::from)
        .collect();
    let mut options = ModOptions::default();
    if let Some(v) = args.value("--version") {
        options.version = v.into();
    }
    if let Some(c) = args.value("--compatible") {
        options.compatible = c.into();
    }
    if let Some(icon) = args.value("--icon") {
        options.icon = Some(economy::read_icon(icon)?);
    }
    let output = args.value("--output").unwrap_or(economy::MOD_FILE_NAME);

    let data = fs::read(input).map_err(|e| Error::Io {
        path: input.into(),
        message: e.to_string(),
    })?;
    let edit = scale_economy(&data, &ScaleOptions { factor, also })?;
    println!(
        "Gelesen: {input}. Faktor {}, also {} Prozent des Erlöses",
        edit.factor.text(),
        edit.factor.percent()
    );
    println!("Geändert:");
    for c in &edit.changes {
        let component = c
            .component
            .map(|n| format!(" (Komponente {n})"))
            .unwrap_or_default();
        let note = if c.became_fraction {
            "  (war eine ganze Zahl, bitte prüfen, ob das Spiel hier eine Kommazahl annimmt)"
        } else {
            ""
        };
        println!(
            "  Zeile {}: {}{component}: {} -> {}{note}",
            c.line, c.key, c.old, c.new
        );
    }
    for key in &edit.missing {
        println!("Warnung: {key} kommt in der Datei nicht vor");
    }
    if !edit.candidates.is_empty() {
        println!("Nicht verändert, bitte prüfen (mit --also <schlüssel> mitskalieren):");
        for c in &edit.candidates {
            println!("  Zeile {}: {}: {}", c.line, c.key, c.value);
        }
    }
    for i in &edit.includes {
        println!(
            "Hinweis: Zeile {} bindet eine Datei ein ({}). Was dort steht, ist nicht geprüft und nicht verändert",
            i.line, i.text
        );
    }
    for s in &edit.skipped {
        println!(
            "Hinweis: Zeile {} nicht verstanden und unverändert: {}",
            s.line, s.text
        );
    }
    let zip = build_mod(&edit, &options)?;
    let written = write_mod(output, &zip, args.switch("--force"))?;
    println!(
        "Geschrieben: {} ({} Bytes, mit {}, {}{} und {})",
        written.display(),
        zip.len(),
        economy::MANIFEST_PATH,
        economy::DESCRIPTION_PATH,
        if options.icon.is_some() {
            format!(", {}", economy::ICON_PATH)
        } else {
            String::new()
        },
        economy::SII_PATH,
    );
    println!(
        "Weiter: Datei nach Dokumente\\Euro Truck Simulator 2\\mod kopieren, im Mod-Manager aktivieren und im Dashboard im Büro den Erlösfaktor auf {} stellen. Anleitung in docs/wirtschafts-mod.md",
        edit.factor.revenue_factor()
    );
    Ok(())
}
