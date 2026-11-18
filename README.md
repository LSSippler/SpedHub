# SpedHub

[![CI](https://github.com/LSSippler/SpedHub/actions/workflows/ci.yml/badge.svg)](https://github.com/LSSippler/SpedHub/actions/workflows/ci.yml)

SpedHub macht aus Euro Truck Simulator 2 eine echte Firma. Mehrere Fahrer fahren zusammen im Convoy des Spiels, die Einnahmen gehen in eine gemeinsame Kasse, und wer keinen eigenen Lkw hat, leiht sich einen aus dem Fuhrpark der Firma. Der geliehene Lkw landet wirklich im eigenen Spielstand, mit Kilometerstand, Verschleiß und Tuning.

## Was SpedHub kann

- **Kasse und Lohn.** Erlöse aus Aufträgen gehen ganz in die Firmenkasse, auf Wunsch mit einem Erlösfaktor für die Wirtschafts-Mod. Einmal am Tag bekommt jeder Fahrer seinen Lohn: Kilometer mal Cent je km seiner Lohnstufe, dazu eine Tagespauschale. Die Lohnstufe hängt am Level im Spiel und an den Firmenkilometern. Reicht die Kasse nicht, bleibt der Rest als offene Lohnforderung stehen. Die Kasse ist ein Journal, Korrekturen sind neue Buchungen.
- **Geldabgleich.** Im Spiel hat jeder Fahrer sein Lohnkonto plus einen Betriebsvorschuss der Firma. Der Launcher gleicht den Kontostand im Spielstand mit dem Server ab: Was fehlt, etwa fürs Tanken, bucht die Firma als Betriebskosten, was nicht erklärbar mehr ist, sieht der Chef. Ereignisse, die erst nach dem Abgleich ankommen, ordnet der Server über ihren Zeitpunkt im Spiel dem richtigen Abgleich zu und korrigiert ihn mit einer neuen Buchung.
- **Fuhrpark und Leihe.** Der Chef übernimmt Lkw aus seinem Spielstand in den Fuhrpark. Fahrer leihen sie im Launcher aus, der sie in einen neuen Speicherplatz einsetzt, und geben sie zurück. Der Lkw wird über Modell und Teile wiedererkannt, auch wenn das Spiel das Kennzeichen ersetzt hat.
- **Herzschlag und Online-Status.** Das Plugin meldet Aufträge, Strafen, Maut, Fähre und Zug und schickt alle 10 Sekunden einen Herzschlag. Das Dashboard zeigt, wer online ist, mit welchem Lkw und welchem Auftrag, der Launcher, welcher geliehene Lkw gerade fährt. Nach einem Absturz nimmt der Chef einen Lkw mit dem letzten bekannten Zustand zurück.
- **Werkstatt und Haftung.** Der Server rechnet aus dem Verschleiß einen Schaden in Prozent. Über der Schadensgrenze steht ein Lkw nach der Rückgabe in der Werkstatt, bis der Chef ihn aus der Kasse reparieren lässt. Mit einem Haftungsanteil zahlt der Fahrer einen Teil des neuen Schadens aus seinem Lohnkonto.
- **Anschaffungen.** Der Chef gibt ein Anschaffungsbudget frei, das beim Geldabgleich zusätzlich in seinen Spielstand kommt. Den Kauf im Spiel bucht der Abgleich genau einmal, danach lässt er sich als Lkw-Kauf mit dem Lkw verknüpfen. Lkw lassen sich verkaufen oder ausmustern.
- **Wirtschafts-Mod.** `spedhub-save economy-mod` baut aus der `economy_data.sii` deines Spiels die Mod `spedhub_economy.scs`, mit der Aufträge im Spiel nur 1 Prozent zahlen. Der Server rechnet mit dem Erlösfaktor 100 zurück, so hat der Fahrer im Spiel nur sein Lohnkonto und den Vorschuss. Im Convoy ist die Mod Pflicht. Die Originaldatei gehört SCS und liegt nicht im Repo, Anleitung in `docs/wirtschafts-mod.md`. Im Spiel noch nicht geprüft.
- **Setup und Updates.** Ein Setup für Windows installiert Launcher und Plugin und findet das Spiel über Steam. Der Launcher meldet neue Versionen und installiert sie auf Knopfdruck nach geprüfter Prüfsumme. Für den eigenen Server gibt es ein statisches Linux-Programm.

| Baustein | Stand |
|---|---|
| Server mit Accounts, Kasse, Lohn, Fuhrpark, Geldabgleich, Online-Status, Werkstatt und Anschaffungen | läuft, siehe `server/` |
| Web-Dashboard | läuft, wird vom Server ausgeliefert |
| Save-Bibliothek mit Kommandozeile, auch für die Wirtschafts-Mod | erste Fassung, siehe `crates/spedhub-save`, `tools/README.md` und `docs/wirtschafts-mod.md` |
| Launcher | erste Fassung, siehe `crates/spedhub-launcher` |
| Spiel-Plugin für Telemetrie | erste Fassung, meldet Gameplay-Events und Herzschlag, siehe `crates/spedhub-plugin` |
| Setup und Updates | erste Fassung, siehe Installation und `installer/` |

Noch nicht gebaut sind das Aufräumen des Firmenprofils beim Start (fremde Firmen-Lkw entfernen), der Lkw-Wechsel im laufenden Spiel, ein Overlay im Spiel und die Anbindung an einen Convoy-Server.

Im Spiel bestätigt ist bisher (ETS2 1.62): Das Spiel lädt einen Spielstand, in den `spedhub-save` einen Lkw eingesetzt hat, der neue Speicherplatz des Launchers erscheint, das Spiel übernimmt den Kontostand aus dem Geldabgleich, und das Plugin lädt und meldet Herzschläge und abgelieferte Aufträge. Anderes ist nur ohne Spiel getestet, etwa die Einheiten im Herzschlag, die Wirtschafts-Mod, das Erkennen des Firmenprofils über `game.log.txt` und ob Setup und Plugin-Installation auf einem echten Windows laufen. Die Liste steht in `docs/architektur.md`, Abschnitt "Offene Tests im Spiel und auf Windows".

## Installation

Für Fahrer gibt es ein Setup für Windows (64 Bit):

1. Auf GitHub unter [Releases](https://github.com/LSSippler/SpedHub/releases) beim neuesten Release `SpedHub-Setup-<Version>.exe` laden. Wer sichergehen will, vergleicht die SHA-256 mit `SHA256SUMS.txt` aus demselben Release: `Get-FileHash .\SpedHub-Setup-0.2.0.exe`.
2. Setup starten und die Nachfrage nach Adminrechten bestätigen. Es installiert den Launcher (und daneben `spedhub-save.exe`, unter anderem für die Wirtschafts-Mod) nach `Programme\SpedHub`, legt eine Verknüpfung im Startmenü an und, wenn nicht abgewählt, auf dem Desktop.
3. Am Ende kopiert das Setup das Plugin in den Spielordner von Euro Truck Simulator 2. Den Spielordner sucht es über Steam, in allen Bibliotheken. Findet es ihn nicht, sagt es das und lässt den Ordner wählen.
4. Wahlweise, standardmäßig abgewählt: "Server für lokale Tests". Dann gibt es im Startmenü "SpedHub-Server (lokaler Test)", der einen Server auf http://127.0.0.1:8787 mit Datenbank in `%LOCALAPPDATA%\SpedHub\server` startet.
5. Launcher starten, die Adresse des Firmenservers eintragen, anmelden und die Firma wählen. Eine Firma gründen oder ihr beitreten geht im Dashboard des Servers. Sobald eine Firma gewählt ist, schreibt der Launcher die Anmeldung für das Plugin. Beim nächsten Spielstart fragt ETS2 einmal, ob es Plugins laden darf, das mit OK bestätigen.

Den Server für die Firma betreibt der Chef, am besten auf einem VPS. Dafür gibt es im Release `spedhub-server-linux-x86_64`, siehe `server/README.md`, Abschnitt "Auf einem VPS".

Updates meldet der Launcher selbst: Gibt es ein neueres Release, zeigt er oben "Version X verfügbar". "Herunterladen und installieren" lädt das Setup, prüft die Prüfsumme und startet es. Ohne Klick passiert nichts.

Deinstallieren geht über die Windows-Einstellungen (Apps) oder "SpedHub deinstallieren" im Startmenü. Das entfernt Programm und Plugin. Läuft dabei das Spiel, fragt die Deinstallation, ob sie es nach dem Beenden noch einmal versuchen soll. Mit Abbrechen, und bei einer stillen Deinstallation immer, wird das Plugin umbenannt, sodass das Spiel es nicht mehr lädt, und beim nächsten Neustart von Windows gelöscht. Die Frage "Anmeldung, Einstellungen und Backups behalten?" steht auf Ja. Mit Nein wird `%LOCALAPPDATA%\SpedHub` gelöscht, mit allen Backups.

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

Das Setup baut nur unter Windows `pwsh installer\build.ps1`, mit Inno Setup 6 (fehlt es, installiert das Skript es über Chocolatey). Ergebnis sind `dist\SpedHub-Setup-<Version>.exe` und `dist\SHA256SUMS.txt`. Den statischen Server für Linux baut unter Linux `cargo build --release -p spedhub-server --target x86_64-unknown-linux-musl`, vorher einmal `rustup target add x86_64-unknown-linux-musl` und `sudo apt-get install -y musl-tools`.

## Version

Alle Teile haben eine gemeinsame Versionsnummer, `[workspace.package] version` in der `Cargo.toml` im Wurzelordner: Server, Save-Bibliothek, Plugin, Launcher und Setup. Der Launcher zeigt sie unten im Fenster. Server und Launcher passen zusammen, wenn Haupt- und Nebenversion gleich sind, also `0.2.0` mit `0.2.5`, aber nicht mit `0.3.0`. Beim Anmelden vergleichen beide, der Launcher zeigt sonst einen gelben Hinweis und der Server schreibt eine Warnung ins Log.

## So veröffentlichst du eine Version

Ein Release baut GitHub Actions aus einem Tag. Von Hand gebaut wird nichts.

1. **Version erhöhen.** In der `Cargo.toml` im Wurzelordner `version` unter `[workspace.package]` setzen, etwa von `0.2.0` auf `0.2.1`. Eine neue Nebenversion (`0.3.0`) heißt: Launcher und Server passen nicht mehr zu älteren, alle müssen aktualisieren. Danach `cargo build` laufen lassen, damit `Cargo.lock` die neue Version kennt, und `Cargo.lock` mit committen: Der Workflow baut mit `--locked` und bricht sonst ab. Wie immer prüfen:

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
   - Windows: prüft Tag gegen Version, führt alle Tests aus, baut Launcher, Plugin, `spedhub-save` und Server im Release-Modus und daraus mit `installer/build.ps1` und Inno Setup `SpedHub-Setup-0.2.1.exe`.
   - Linux: baut `spedhub-server` für x86_64, statisch mit musl gelinkt, und prüft, dass er keine Bibliotheken braucht.
   - Release: legt eine gemeinsame `SHA256SUMS.txt` an, schreibt die Release-Notes aus `git log` und veröffentlicht das Release "SpedHub 0.2.1" mit Setup, `spedhub-server-linux-x86_64` und `SHA256SUMS.txt`.

5. **Release prüfen.** Unter Actions muss der Lauf "Release" grün sein, unter Releases steht das neue Release mit drei Dateien. Das Setup auf einem Windows-Rechner laden, Prüfsumme vergleichen (`Get-FileHash .\SpedHub-Setup-0.2.1.exe` gegen die Zeile in `SHA256SUMS.txt`), installieren, Launcher starten: unten steht die neue Version. Ein älterer Launcher zeigt beim Start "Version 0.2.1 verfügbar".

6. **Server auf dem VPS aktualisieren**, wenn sich am Server etwas geändert hat oder die Nebenversion neu ist:

   Erst in einen eigenen Ordner laden und prüfen, nicht nach `/opt/spedhub`: Dort liegt die Datei, die der Dienst gerade ausführt, und der Ordner gehört dem Benutzer `spedhub`.

   ```bash
   mkdir -p /tmp/spedhub-update && cd /tmp/spedhub-update
   curl -fLO https://github.com/LSSippler/SpedHub/releases/download/v0.2.1/spedhub-server-linux-x86_64
   curl -fLO https://github.com/LSSippler/SpedHub/releases/download/v0.2.1/SHA256SUMS.txt
   sha256sum -c --ignore-missing SHA256SUMS.txt
   chmod +x spedhub-server-linux-x86_64
   ./spedhub-server-linux-x86_64 --version
   ```

   Dann in dieser Reihenfolge: Dienst stoppen, Datenbank sichern, die neue Datei an ihren Platz legen, Dienst starten. `spedhub.db*` nimmt eine vorhandene `spedhub.db-wal` mit, sie gehört zur Datenbank.

   ```bash
   sudo systemctl stop spedhub
   sudo mkdir -p /opt/spedhub/backup/vor-0.2.1
   sudo sh -c 'cp -p /opt/spedhub/spedhub.db* /opt/spedhub/backup/vor-0.2.1/'
   sudo install -m 755 -o spedhub spedhub-server-linux-x86_64 /opt/spedhub/
   sudo systemctl start spedhub
   systemctl status spedhub
   ```

   Eine vorhandene Datenbank erweitert der Server beim Start selbst. Zurück zur alten Fassung geht es genauso mit der alten Datei und der gesicherten Datenbank. Details in `server/README.md`, Abschnitt "Auf einem VPS".

Ging etwas schief: Release auf GitHub löschen, den Tag löschen (`git push --delete origin v0.2.1` und `git tag -d v0.2.1`), Fehler beheben und mit derselben oder einer neuen Version wieder ab Schritt 3. Ein schon veröffentlichtes Release, das Launcher eventuell geladen haben, besser nicht ersetzen, sondern eine neue Version machen.

Die normale CI (`.github/workflows/ci.yml`) baut bei jedem Pull Request ebenfalls das Setup und den Linux-Server und lädt sie als Artefakte hoch, dazu Plugin, Launcher und `spedhub-save.exe` einzeln (`spedhub-setup-windows-x64`, `spedhub-server-linux-x86_64`, `spedhub-plugin-windows-x64`, `spedhub-launcher-windows-x64`, `spedhub-save-windows-x64`).

## Aufbau

| Ordner | Inhalt |
|---|---|
| `server/` | Backend und Dashboard: Kasse, Lohn, Fuhrpark, Geldabgleich, Online-Status, Werkstatt, Anschaffungen. Details in `server/README.md` |
| `crates/spedhub-save/` | Save-Bibliothek und Kommandozeile `spedhub-save`: Spielstand lesen, prüfen, schreiben, Lkw exportieren, wiedererkennen und einsetzen, Kontostand setzen, Wirtschafts-Mod bauen |
| `crates/spedhub-plugin/` | Plugin für ETS2: meldet Aufträge, Strafen, Maut, Fähre und Zug und schickt den Herzschlag. Details in `crates/spedhub-plugin/README.md` |
| `crates/spedhub-launcher/` | Launcher für Windows: anmelden, Fuhrpark, Lkw in den Spielstand schreiben, Geldabgleich, Spiel starten, Plugin installieren, Updates. Details in `crates/spedhub-launcher/README.md` |
| `installer/` | Setup mit Inno Setup (`spedhub.iss`) und das Bauskript `build.ps1`, nur unter Windows |
| `.github/workflows/` | `ci.yml` prüft jeden Pull Request unter Linux und Windows und baut Setup und Linux-Server mit, `release.yml` baut aus einem Tag das Release |
| `tools/` | Bedienung von `spedhub-save` und die Python-Referenz, Details in `tools/README.md` |
| `testdata/` | Kleine, anonymisierte Test-Spielstände aus ETS2 1.62 |
| `docs/` | Architektur (`architektur.md`), Spielstand-Format (`save-format.md`) und Wirtschafts-Mod (`wirtschafts-mod.md`) |

## Hinweis

SpedHub ist ein privates Fan-Projekt und steht in keiner Verbindung zu SCS Software. Es bearbeitet Spielstände. Das wird von SCS geduldet, aber nicht unterstützt. Benutzung auf eigene Gefahr, und immer mit Backup des Profils.
