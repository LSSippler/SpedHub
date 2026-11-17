# SpedHub: Hinweise für die Arbeit im Repo

SpedHub macht aus Euro Truck Simulator 2 eine Firma mit gemeinsamer Kasse und gemeinsamem Fuhrpark. Geliehene Lkw werden per Save-Editing in den Spielstand des Fahrers geschrieben. Lies vor größeren Änderungen `docs/architektur.md`, und vor jeder Arbeit am Spielstand `docs/save-format.md`.

## Befehle

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p spedhub-server
cargo run -p spedhub-save -- check testdata/quelle_mit_lkw.sii
python3 tools/transplant.py testdata/quelle_mit_lkw.sii testdata/ziel_schnellauftrag.sii /tmp/ergebnis.sii
cargo clippy -p spedhub-launcher -p spedhub-plugin --target x86_64-pc-windows-gnu --all-targets -- -D warnings
cargo build --release -p spedhub-server --target x86_64-unknown-linux-musl
```

Der Clippy-Lauf für `x86_64-pc-windows-gnu` prüft unter Linux den Windows-Code von Launcher und Plugin. Der musl-Build ist der statische Server fürs Release und braucht `musl-tools`. Das Setup baut nur unter Windows: `pwsh installer/build.ps1` (Inno Setup 6).

Vor jedem Commit müssen Tests und Clippy sauber durchlaufen.

## Aufbau

- `server/`: Backend (axum, rusqlite) und Dashboard (`server/web/index.html`, wird per `include_str!` eingebaut). Die Uhr ist über `api::router_with` stellbar, Tests warten nie auf echte Zeit. Endpunkte in `api.rs`, die Fachlogik in eigenen Modulen: `reconcile.rs` (Abgleichfenster, spät gemeldete Ereignisse), `presence.rs` (Herzschlag, Online, letzter bekannter Zustand), `damage.rs` (Schaden, Werkstatt, Haftung), `purchases.rs` (Anschaffungsbudget), `payroll.rs`, `levels.rs`, `config.rs` (Kommandozeile, `--local`), `version.rs` (Versionsprüfung)
- `crates/spedhub-save/`: Save-Bibliothek und Kommandozeile `spedhub-save`. Alle Feldnamen stehen in `src/layout.rs`
- `crates/spedhub-plugin/`: Telemetrie-Plugin (DLL) für ETS2, meldet Gameplay-Events und den Herzschlag an den Server. Logik ohne Spiel testbar, `src/ffi.rs` ist die einzige Stelle mit SDK-Bezug, `src/telemetry.rs` macht aus dem Snapshot der Kanäle den Herzschlag
- `crates/spedhub-launcher/`: Launcher für Windows (egui/eframe). Logik in testbaren Modulen, nur `src/main.rs` ist Oberfläche, Windows-Aufrufe stecken in `src/system.rs`. Am Spielstand arbeitet er nur über `spedhub-save`. Dazu `steam.rs` (ETS2 über Steam finden), `plugin_install.rs` (Plugin kopieren und entfernen), `update.rs` (GitHub-Releases, Prüfsumme), `version.rs`, `cli.rs` (Aufrufe für das Setup)
- `installer/`: Inno-Setup-Skript `spedhub.iss` und `build.ps1`
- `.github/workflows/`: `ci.yml` für jeden PR (Linux und Windows, baut Setup und musl-Server mit), `release.yml` baut bei einem Tag `vX.Y.Z` das Release. Ablauf in `README.md`, "So veröffentlichst du eine Version"
- `tools/transplant.py`: Python-Referenz für das Einsetzen eines Lkw
- `testdata/`: verkleinerte, anonymisierte Spielstände aus ETS2 1.62
- `docs/`: Architektur und Save-Format

Eine Versionsnummer für alle Teile: `[workspace.package] version` in der `Cargo.toml` im Wurzelordner.

Das Repo ist ein Cargo-Workspace. Neue Crates kommen als Mitglied in die `Cargo.toml` im Wurzelordner.

## Regeln

1. **Der Server ist die Wahrheit.** Firmen-Lkw und Geld im Spielstand sind Kopien, die bei jedem Abgleich aus dem Serverstand neu aufgebaut werden. Kein Ablauf darf sich darauf verlassen, dass der lokale Spielstand stimmt.
2. **Ins Spiel geschrieben wird nur über den Spielstand.** Kein Schreiben in den Arbeitsspeicher, keine Hooks, keine DLL-Injection. Erlaubt sind das Telemetry SDK und das Input SDK von SCS.
3. **Jeder Schreibvorgang am Spielstand ist abgesichert.** Backup vorher, danach Prüfung: alle Verweise lösen auf, keine Unit ist verwaist, die Datei lässt sich wieder einlesen. Bei einem Fehler wird nichts geschrieben.
4. **Die Kasse ist ein Journal.** Stände sind immer die Summe der Buchungen. Buchungen werden nie geändert oder gelöscht, Korrekturen sind neue Buchungen.
5. **Lizenzen.** Abhängigkeiten nur mit MIT, Apache-2.0 oder vergleichbar freier Lizenz. Aus `truck-tools` (GPL-3.0) und `NMC-SCS-LAUNCHER` (ohne Lizenz) wird kein Code übernommen, auch nicht umgeschrieben.
6. **Das Repo ist öffentlich.** Keine echten Spielstände, keine Zugangsdaten, keine persönlichen Daten. Neue Testdaten werden verkleinert und anonymisiert wie die vorhandenen.

## Stil

- Rust-Bezeichner sind englisch. Kommentare, Fehlermeldungen, Oberflächentexte und Doku sind deutsch.
- Oberflächentexte sind schlicht und sagen, was passiert: "Lkw anlegen", nicht "Absenden". Keine Großschreibung ganzer Wörter, keine Gedankenstriche.
- Das Dashboard bleibt eine einzelne HTML-Datei ohne Build-Schritt und ohne externe Abhängigkeiten. Texte von Nutzern werden nie als HTML eingefügt, sondern über den Helfer `h()` als Text.
- Neue Abhängigkeiten nur, wenn sie wirklich Arbeit sparen.
- Jede Änderung am Verhalten bekommt einen Test. Die Server-Tests in `server/tests/flow.rs` spielen ganze Abläufe über die HTTP-Schnittstelle durch, neue Tests folgen diesem Muster.

## Arbeitsweise

- Nicht direkt auf `main` arbeiten. Eigener Branch, am Ende ein Pull Request mit kurzer Beschreibung auf Deutsch: was geändert wurde, wie es getestet ist, was offen bleibt.
- Zielplattform für alles, was mit dem Spiel spricht, ist Windows. Der Server läuft auch unter Linux.
- Was sich nur mit laufendem Spiel prüfen lässt, wird im Pull Request ausdrücklich als ungeprüft genannt.

## Was noch nicht bestätigt ist

Im Spiel bestätigt (ETS2 1.62, Abschnitt "Im Spiel bestätigt" in `docs/save-format.md`): Ein mit `spedhub-save` eingesetzter Lkw lädt mit Zubehör, Kilometern, Verschleiß und Tank, und der nächste Autosave des Spiels ist intakt. Das Spiel vergibt dabei alle `_nameless`-Namen neu und kann das Kennzeichen ersetzen. Weder Unit-Name noch Kennzeichen sind also eine dauerhafte Kennung, Lkw werden über Modell und Teile wiedererkannt (`find_package_truck`).

Offen ist weiter: warum das Spiel Kennzeichen ersetzt, woher `Economy reset - company mismatch` kommt, ob neue Speicherplätze des Launchers im Spiel erscheinen, ob das Spiel einen beim Geldabgleich geschriebenen Kontostand übernimmt (auch mit Anschaffungsbudget), ob das aus `info_players_experience` berechnete Level zum Level im Spiel passt, ob das Spiel einen nach der Werkstatt mit Verschleiß 0 eingesetzten Lkw so annimmt, das Einsetzen neben eigene Lkw und das Entfernen. Code am Spielstand deshalb weiter so bauen, dass sich die Feldliste in `crates/spedhub-save/src/layout.rs` leicht anpassen lässt.

Ebenfalls ungeprüft: Lädt das Spiel die Plugin-DLL, liefern die Kanäle für den Herzschlag die erwarteten Einheiten, welches Format hat `license.plate`, ist die Job-Konfiguration bei `job.delivered` noch gefüllt? Das Setup ist nur auf dem Windows-Runner der CI gebaut. Setup, Plugin-Installation und Neustart über UAC sind nie auf einem echten Windows gelaufen.
