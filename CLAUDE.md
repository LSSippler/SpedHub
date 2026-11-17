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
```

Vor jedem Commit müssen Tests und Clippy sauber durchlaufen.

## Aufbau

- `server/`: Backend (axum, rusqlite) und Dashboard (`server/web/index.html`, wird per `include_str!` eingebaut)
- `crates/spedhub-save/`: Save-Bibliothek und Kommandozeile `spedhub-save`. Alle Feldnamen stehen in `src/layout.rs`
- `crates/spedhub-plugin/`: Telemetrie-Plugin (DLL) für ETS2, meldet Gameplay-Events an den Server. Logik ohne Spiel testbar, `src/ffi.rs` ist die einzige Stelle mit SDK-Bezug
- `tools/transplant.py`: Python-Referenz für das Einsetzen eines Lkw
- `testdata/`: verkleinerte, anonymisierte Spielstände aus ETS2 1.62
- `docs/`: Architektur und Save-Format

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

Der Test, ob ETS2 1.62 einen mit `tools/transplant.py` gebauten Spielstand fehlerfrei lädt, steht aus. Die Feldliste in `docs/save-format.md` ist aus echten Spielständen abgelesen, aber noch nicht im Spiel bestätigt. Code am Spielstand deshalb so bauen, dass sich die Feldliste leicht anpassen lässt.
