# SpedHub

[![CI](https://github.com/LSSippler/SpedHub/actions/workflows/ci.yml/badge.svg)](https://github.com/LSSippler/SpedHub/actions/workflows/ci.yml)

SpedHub macht aus Euro Truck Simulator 2 eine echte Firma. Mehrere Fahrer fahren zusammen im Convoy, die Einnahmen gehen in eine gemeinsame Kasse, und wer keinen eigenen Lkw hat, leiht sich einen aus dem Fuhrpark der Firma. Der geliehene Lkw landet wirklich im eigenen Spielstand, mit Kilometerstand, Schaden und Tuning.

Das Projekt steht am Anfang. Was schon läuft und was noch fehlt:

| Baustein | Stand |
|---|---|
| Server mit Accounts, Kasse und Fuhrpark | läuft, siehe `server/` |
| Web-Dashboard | läuft, wird vom Server ausgeliefert |
| Save-Bibliothek mit Kommandozeile | erste Fassung, siehe `crates/spedhub-save` und `tools/README.md` |
| Launcher | erste Fassung, siehe `crates/spedhub-launcher` |
| Setup und Updates | erste Fassung, siehe Installation und `installer/` |
| Spiel-Plugin für Telemetrie | erste Fassung, meldet Gameplay-Events, siehe `crates/spedhub-plugin` |

Wichtig: Dass das Spiel einen von SpedHub bearbeiteten Spielstand fehlerfrei lädt, ist noch nicht im Spiel bestätigt. Bis dahin sind alle Aussagen zum Spielstand-Format vorläufig.

## Installation

Für Fahrer gibt es ein Setup für Windows (64 Bit):

1. Auf GitHub unter [Releases](https://github.com/LSSippler/SpedHub/releases) beim neuesten Release `SpedHub-Setup-<Version>.exe` laden. Wer sichergehen will, vergleicht die SHA-256 mit `SHA256SUMS.txt` aus demselben Release: `Get-FileHash .\SpedHub-Setup-0.2.0.exe`.
2. Setup starten und die Nachfrage nach Adminrechten bestätigen. Es installiert den Launcher nach `Programme\SpedHub`, legt eine Verknüpfung im Startmenü an und, wenn nicht abgewählt, auf dem Desktop.
3. Am Ende kopiert das Setup das Plugin in den Spielordner von Euro Truck Simulator 2. Den Spielordner sucht es über Steam, in allen Bibliotheken. Findet es ihn nicht, sagt es das und lässt den Ordner wählen.
4. Wahlweise, standardmäßig abgewählt: "Server für lokale Tests". Dann gibt es im Startmenü "SpedHub-Server (lokaler Test)", der einen Server auf http://127.0.0.1:8787 mit Datenbank in `%LOCALAPPDATA%\SpedHub\server` startet.

Updates meldet der Launcher selbst: Gibt es ein neueres Release, zeigt er oben "Version X verfügbar". "Herunterladen und installieren" lädt das Setup, prüft die Prüfsumme und startet es. Ohne Klick passiert nichts.

Deinstallieren geht über die Windows-Einstellungen (Apps) oder "SpedHub deinstallieren" im Startmenü. Das entfernt Programm und Plugin. Die Frage "Anmeldung, Einstellungen und Backups behalten?" steht auf Ja. Mit Nein wird `%LOCALAPPDATA%\SpedHub` gelöscht, mit allen Backups.

Details zum Launcher in `crates/spedhub-launcher/README.md`, zum Plugin in `crates/spedhub-plugin/README.md`, zum Server für den eigenen VPS in `server/README.md`.

## Loslegen als Entwickler

Voraussetzung ist ein aktuelles Rust.

```powershell
git clone https://github.com/LSSippler/SpedHub.git C:\dev\SpedHub
cd C:\dev\SpedHub
cargo test --workspace
cargo run --release -p spedhub-server
```

Danach liegt das Dashboard unter http://127.0.0.1:8787.

## Version

Alle Teile haben eine gemeinsame Versionsnummer, `[workspace.package] version` in der `Cargo.toml` im Wurzelordner: Server, Save-Bibliothek, Plugin, Launcher und Setup. Der Launcher zeigt sie unten im Fenster. Server und Launcher passen zusammen, wenn Haupt- und Nebenversion gleich sind, also `0.2.0` mit `0.2.5`, aber nicht mit `0.3.0`. Beim Anmelden vergleichen beide, der Launcher zeigt sonst einen gelben Hinweis und der Server schreibt eine Warnung ins Log.

## So veröffentlichst du eine Version

Ein Release baut GitHub Actions aus einem Tag. Von Hand gebaut wird nichts.

1. **Version erhöhen.** In der `Cargo.toml` im Wurzelordner `version` unter `[workspace.package]` setzen, etwa von `0.2.0` auf `0.2.1`. Eine neue Nebenversion (`0.3.0`) heißt: Launcher und Server passen nicht mehr zu älteren, alle müssen aktualisieren. Danach `cargo build` laufen lassen, damit `Cargo.lock` die neue Version kennt, und wie immer prüfen:

   ```powershell
   cd C:\dev\SpedHub
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

2. **Änderungen beschreiben.** Die Release-Notes entstehen aus den Commit-Titeln seit dem letzten Tag (beim ersten Tag aus allen). Commit-Titel deshalb so schreiben, dass Fahrer sie verstehen. Die Versionsänderung als eigenen Commit, etwa "Version 0.2.1", per Pull Request nach `main`.

3. **Tag setzen und pushen**, auf dem Stand von `main`, der veröffentlicht werden soll:

   ```powershell
   git switch main
   git pull
   git tag -a v0.2.1 -m "SpedHub 0.2.1"
   git push origin v0.2.1
   ```

   Der Tag muss genau `v` plus die Version aus der `Cargo.toml` sein, sonst bricht der Workflow mit einer Meldung ab.

4. **Was der Workflow tut** (`.github/workflows/release.yml`):
   - Windows: prüft Tag gegen Version, führt alle Tests aus, baut Launcher, Plugin und Server im Release-Modus und daraus mit `installer/build.ps1` und Inno Setup `SpedHub-Setup-0.2.1.exe`.
   - Linux: baut `spedhub-server` für x86_64, statisch mit musl gelinkt, und prüft, dass er keine Bibliotheken braucht.
   - Release: legt eine gemeinsame `SHA256SUMS.txt` an, schreibt die Release-Notes aus `git log` und veröffentlicht das Release "SpedHub 0.2.1" mit Setup, `spedhub-server-linux-x86_64` und `SHA256SUMS.txt`.

5. **Release prüfen.** Unter Actions muss der Lauf "Release" grün sein, unter Releases steht das neue Release mit drei Dateien. Das Setup auf einem Windows-Rechner laden, Prüfsumme vergleichen (`Get-FileHash .\SpedHub-Setup-0.2.1.exe` gegen die Zeile in `SHA256SUMS.txt`), installieren, Launcher starten: unten steht die neue Version. Ein älterer Launcher zeigt beim Start "Version 0.2.1 verfügbar".

6. **Server auf dem VPS aktualisieren**, wenn sich am Server etwas geändert hat oder die Nebenversion neu ist:

   ```bash
   cd /opt/spedhub
   curl -fLO https://github.com/LSSippler/SpedHub/releases/download/v0.2.1/spedhub-server-linux-x86_64
   curl -fLO https://github.com/LSSippler/SpedHub/releases/download/v0.2.1/SHA256SUMS.txt
   sha256sum -c --ignore-missing SHA256SUMS.txt
   chmod +x spedhub-server-linux-x86_64
   ./spedhub-server-linux-x86_64 --version
   ```

   Dann den laufenden Server beenden, die Datei an seinen Platz legen und neu starten. Vorher die Datenbank sichern. Details in `server/README.md`, Abschnitt "Auf einem VPS".

Ging etwas schief: Release auf GitHub löschen, den Tag löschen (`git push --delete origin v0.2.1` und `git tag -d v0.2.1`), Fehler beheben und mit derselben oder einer neuen Version wieder ab Schritt 3. Ein schon veröffentlichtes Release, das Launcher eventuell geladen haben, besser nicht ersetzen, sondern eine neue Version machen.

Die normale CI baut bei jedem Pull Request ebenfalls das Setup und den Linux-Server und lädt beide als Artefakte hoch (`spedhub-setup-windows-x64`, `spedhub-server-linux-x86_64`).

## Aufbau

| Ordner | Inhalt |
|---|---|
| `server/` | Backend und Dashboard, Details in `server/README.md` |
| `crates/spedhub-save/` | Save-Bibliothek: Spielstand lesen, prüfen, schreiben, Lkw exportieren und einsetzen |
| `crates/spedhub-plugin/` | Plugin für ETS2: meldet Aufträge, Strafen, Maut, Fähre und Zug an den Server |
| `crates/spedhub-launcher/` | Launcher für Windows: anmelden, Fuhrpark, Lkw in den Spielstand schreiben, Spiel starten |
| `installer/` | Setup mit Inno Setup (`spedhub.iss`) und das Bauskript `build.ps1` |
| `tools/` | Bedienung von `spedhub-save` und die Python-Referenz, Details in `tools/README.md` |
| `testdata/` | Kleine Test-Spielstände mit echtem Aufbau |
| `docs/` | Architektur und Notizen zum Spielstand-Format |

## Hinweis

SpedHub ist ein privates Fan-Projekt und steht in keiner Verbindung zu SCS Software. Es bearbeitet Spielstände. Das wird von SCS geduldet, aber nicht unterstützt. Benutzung auf eigene Gefahr, und immer mit Backup des Profils.
