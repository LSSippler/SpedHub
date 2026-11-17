# SpedHub

SpedHub macht aus Euro Truck Simulator 2 eine echte Firma. Mehrere Fahrer fahren zusammen im Convoy, die Einnahmen gehen in eine gemeinsame Kasse, und wer keinen eigenen Lkw hat, leiht sich einen aus dem Fuhrpark der Firma. Der geliehene Lkw landet wirklich im eigenen Spielstand, mit Kilometerstand, Schaden und Tuning.

Das Projekt steht am Anfang. Was schon läuft und was noch fehlt:

| Baustein | Stand |
|---|---|
| Server mit Accounts, Kasse und Fuhrpark | läuft, siehe `server/` |
| Web-Dashboard | läuft, wird vom Server ausgeliefert |
| Werkzeuge für den Spielstand | erste Fassung, siehe `tools/` |
| Save-Bibliothek in Rust | geplant |
| Launcher | geplant |
| Spiel-Plugin für Telemetrie | geplant |

Wichtig: Dass das Spiel einen von SpedHub bearbeiteten Spielstand fehlerfrei lädt, ist noch nicht im Spiel bestätigt. Bis dahin sind alle Aussagen zum Spielstand-Format vorläufig.

## Loslegen

Voraussetzung ist ein aktuelles Rust.

```powershell
git clone https://github.com/LSSippler/SpedHub.git C:\dev\SpedHub
cd C:\dev\SpedHub
cargo test --workspace
cargo run --release -p spedhub-server
```

Danach liegt das Dashboard unter http://127.0.0.1:8787.

## Aufbau

| Ordner | Inhalt |
|---|---|
| `server/` | Backend und Dashboard, Details in `server/README.md` |
| `tools/` | Hilfsprogramme für den Spielstand, Details in `tools/README.md` |
| `testdata/` | Kleine Test-Spielstände mit echtem Aufbau |
| `docs/` | Architektur und Notizen zum Spielstand-Format |

## Hinweis

SpedHub ist ein privates Fan-Projekt und steht in keiner Verbindung zu SCS Software. Es bearbeitet Spielstände. Das wird von SCS geduldet, aber nicht unterstützt. Benutzung auf eigene Gefahr, und immer mit Backup des Profils.
