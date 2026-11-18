# Werkzeuge für den Spielstand

Die Arbeit am Spielstand macht die Save-Bibliothek in `crates/spedhub-save`. Dazu gehört das Kommandozeilenprogramm `spedhub-save`. Hier im Ordner liegt nur noch `transplant.py` als Referenz.

## spedhub-save

```powershell
cd C:\dev\SpedHub
cargo build --release -p spedhub-save
.\target\release\spedhub-save.exe
```

Ohne Argumente zeigt das Programm die Hilfe. Gelesen wird jedes Format (`ScsC`, `BSII`, `SiiN`), geschrieben wird immer Klartext. Vor jedem Schreiben läuft die Integritätsprüfung aus `docs/save-format.md`. Schlägt sie fehl, wird nichts geschrieben. Eine vorhandene Datei wird vorher als `<name>.spedhub-<zeit>.bak` daneben gesichert. Das gilt auch, wenn Ein- und Ausgabe dieselbe Datei sind.

| Befehl | Was passiert |
|---|---|
| `decode <eingabe> <ausgabe>` | Spielstand in Klartext umwandeln |
| `check <datei>` | Integrität prüfen, Probleme auflisten. Rückgabewert 1 bei Problemen |
| `export-truck <spielstand> <paket.json> [--plate <kennzeichen>]` | Lkw als Paket speichern. Ohne `--plate` der Lkw, den der Spieler gerade fährt |
| `insert-truck <spielstand> <paket.json> <ausgabe> [--keep-names]` | Lkw aus einem Paket einsetzen und zum aktiven Lkw machen |
| `remove-truck <spielstand> <kennzeichen> <ausgabe> [--replacement <kennzeichen>]` | Eigenen Lkw mit Zubehör und Statistik entfernen. Beim aktiven Lkw ist ein Ersatz Pflicht |
| `money <game.sii> [--info <info.sii>] [--set <betrag>]` | Kontostand anzeigen oder in beiden Dateien setzen |
| `economy-mod <economy_data.sii> [-o <ziel.scs>] [--factor 0.01] [--also <schlüssel>]... [--version <x>] [--compatible 1.62.*] [--icon <bild.jpg>] [--force]` | Wirtschafts-Mod `spedhub_economy.scs` bauen: die Werte für den Erlös mal Faktor, alles andere Byte für Byte gleich. Ohne Datei zeigt der Befehl, wie man sie aus `def.scs` holt |

Kennzeichen werden ohne Markup und ohne Land angegeben, Groß- und Kleinschreibung ist egal: `SPED 001`, `dd ri 772`.

Beispiel, ein Lkw von einem Profil ins andere:

```powershell
spedhub-save export-truck quelle\game.sii lkw.json
spedhub-save insert-truck ziel\game.sii lkw.json ziel\game.sii
```

### Das Lkw-Paket

JSON mit Modell, Kennzeichen, Zustand, allen `data_path`-Einträgen und dem Bauplan. Der Bauplan sind die `vehicle`-Unit und ihr Zubehör, statt Unit-Namen stehen Platzhalter `@0`, `@1` und so weiter. Beim Einsetzen bekommen die Units neue, eindeutige Namen. Kennzeichen und Zustand im Paket gelten vor den Werten im Bauplan, der Server kann sie also pflegen, ohne den Bauplan anzufassen.

`--keep-names` übernimmt stattdessen die Namen aus der Quelle (`source_name` im Paket). Das ist für den Vergleich mit `transplant.py` gedacht.

### Was beim Einsetzen geht

- Ziel ohne eigenen Lkw im Schnellauftrag: Auftrag, Firmen-Lkw und Anhänger kommen weg, der neue Lkw wird der eigene.
- Ziel mit eigenem Lkw ohne Auftrag: Der bisherige Lkw zieht in einen freien Stellplatz der Heimatgarage, der neue übernimmt den Platz des Spielers. Ohne freien Stellplatz bricht das Einsetzen mit einer Meldung ab.

Alles andere, zum Beispiel ein laufender Auftrag mit eigenem Lkw, wird abgelehnt.

### Wirtschafts-Mod

`economy-mod` arbeitet nicht am Spielstand, sondern an `def/economy_data.sii` aus dem Archiv `def.scs` des Spiels. Die Datei gehört SCS, du entpackst sie selbst mit dem SCS Extractor. Der Befehl multipliziert alle Werte, die den Auftragserlös bestimmen, mit dem Faktor (Standard 0.01), listet jede Änderung alt -> neu, meldet weitere Schlüssel, die nach Geld klingen, und baut daraus die Mod. Ein vorhandenes Ziel ersetzt er nur mit `--force`.

```powershell
spedhub-save economy-mod C:\SpedHub\entpackt\def\economy_data.sii -o C:\SpedHub\spedhub_economy.scs
```

`spedhub-save.exe` installiert das Setup neben den Launcher. Die ganze Anleitung mit Extractor, Mod-Manager, Convoy-Server und Erlösfaktor im Dashboard steht in `docs/wirtschafts-mod.md`. Im Spiel ist die Mod noch nicht geprüft.

## transplant.py

Die erste Fassung des Einsetzens, in Python. Die Rust-Bibliothek macht dasselbe und mehr. Das Skript bleibt als Referenz: Die CI vergleicht sein Ergebnis mit `testdata/erwartet_nach_einsetzen.sii`, und `spedhub-save insert-truck --keep-names` muss genau dieselbe Datei erzeugen.

```powershell
python tools\transplant.py quelle.sii ziel.sii ergebnis.sii
```

Im Spiel geladen wurde ein Spielstand, den `spedhub-save` gebaut hat, siehe `docs/save-format.md`, Abschnitt "Im Spiel bestätigt". Das Ergebnis des Skripts selbst ist dort nicht geprüft.
