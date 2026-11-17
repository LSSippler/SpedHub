# SpedHub-Plugin für ETS2

Das Plugin meldet Gameplay-Events aus Euro Truck Simulator 2 an den SpedHub-Server. Der bucht sie in die Firmenkasse und auf die Lohnkonten und zählt die gefahrenen Kilometer für den Lohn. Dazu schickt es alle 10 Sekunden einen Herzschlag, damit die Firma sieht, wer gerade mit welchem Lkw fährt. Es nutzt nur das offizielle Telemetry SDK von SCS und schreibt nichts ins Spiel.

Wichtig: Ob ETS2 die DLL lädt, ob die Events im Spiel wirklich ankommen ob die Kanäle für den Herzschlag die erwarteten Werte und Einheiten liefern, welches Format `license.plate` genau hat und ob die Job-Konfiguration beim Abliefern noch gefüllt ist, ist noch nicht im Spiel geprüft. Getestet ist bisher alles ohne Spiel, siehe unten.

## Was gemeldet wird

| Im Spiel | Event im SDK | Buchung | Betrag |
|---|---|---|---|
| Auftrag abgeliefert | `job.delivered` | `job_delivered` | Erlös (`revenue`). Die Strecke (`distance.km`) geht gerundet als eigenes Feld `distance_km` mit, der Server schreibt sie dem Fahrer als Firmenkilometer gut. Strecke und Frachtschaden stehen außerdem in der Notiz. Fracht, Start- und Zielstadt gehen als `cargo`, `source_city` und `target_city` mit, aus der Job-Konfiguration (siehe Herzschlag) |
| Auftrag abgebrochen | `job.cancelled` | `job_cancelled` | Vertragsstrafe (`cancel.penalty`) |
| Strafe | `player.fined` | `fined` | `fine.amount`, der Grund steht in der Notiz |
| Maut | `player.tollgate.paid` | `tollgate` | `pay.amount` |
| Fähre | `player.use.ferry` | `ferry` | `pay.amount`, Start und Ziel in der Notiz |
| Zug | `player.use.train` | `train` | `pay.amount`, Start und Ziel in der Notiz |

Wer was zahlt, regelt der Server, siehe `server/README.md`, Abschnitt Buchungsregeln.

Zu jedem Event geht außerdem `occurred_at` mit: wann es im Spiel passiert ist, als Systemzeit dieses Rechners in Unix-Sekunden (UTC). Das Plugin hält den Zeitpunkt fest, sobald das Spiel das Event meldet, nicht erst beim Senden, und er bleibt im Postausgang erhalten. Damit ordnet der Server ein Event dem richtigen Geldabgleich zu, auch wenn es wegen eines Ausfalls erst Stunden später ankommt. Zeilen im Postausgang von älteren Fassungen haben das Feld nicht. Sie zählen beim Server im ersten Geldabgleich nach ihrem Empfang.

## Herzschlag

Alle 10 Sekunden, solange das Spiel läuft und nicht pausiert ist und ein Kennzeichen bekannt ist, geht ein Herzschlag an `POST /api/companies/{id}/heartbeat`. Der Server zeigt den Fahrer damit 30 Sekunden lang als online. Fährt er einen geliehenen Firmen-Lkw, merkt sich der Server den Zustand als letzten bekannten Zustand. Den nimmt er, wenn der Chef den Lkw nach einem Absturz ohne Kilometerstand zurücknimmt.

| Feld | Quelle im SDK |
|---|---|
| `plate` | Konfiguration `truck`, `license.plate`. Markup wie `<img src=...>` und ein Land hinter `\|` nimmt das Plugin heraus |
| `odometer_km` | Kanal `truck.odometer` |
| `wear` | Kanäle `truck.wear.engine`, `.transmission`, `.cabin`, `.chassis`, `.wheels`, je 0 bis 1 |
| `fuel_liters` | Kanal `truck.fuel.amount` |
| `fuel_relative` | `truck.fuel.amount` geteilt durch `fuel.capacity` aus der Konfiguration `truck` |
| `speed_kmh` | Kanal `truck.speed`, Meter je Sekunde mal 3,6, rückwärts positiv |
| `job` | Konfiguration `job`: `cargo`, `source.city`, `destination.city`. Eine leere Konfiguration heißt: kein Auftrag, dann geht `null` mit |
| `near_city` | gibt es im SDK nicht, bleibt leer |

Pausiert ist das Spiel laut SDK am Anfang und nach dem Ereignis `paused`, es läuft nach `started`. Alle Kanäle, Konfigurationen und die Ereignisse für die Pause sind optional angemeldet: Fehlt etwas in einer Spielversion, bleibt der Wert leer und die DLL lädt trotzdem.

Herzschläge kommen nie in den Postausgang. Ist der Server nicht erreichbar, ist der Herzschlag verloren, der nächste kommt 10 Sekunden später. Im Log steht nur der erste Fehler und wann sie wieder ankommen. Ein eigener Thread schickt sie, mit demselben Zeitlimit von 3 Sekunden, ein hängender Herzschlag hält also keine Buchung auf.

Fracht und Städte für `job_delivered`: Das Plugin merkt sich die letzte nicht leere Job-Konfiguration. Das SDK kann sie vor oder nach dem Event `job.delivered` leeren, deshalb gilt die letzte nicht leere. Sie wird mit dem abgelieferten Auftrag gebucht und danach vergessen, ebenso bei einem abgebrochenen Auftrag. Zeilen im Postausgang von älteren Fassungen haben die Felder nicht und lesen sich weiter.

## Installation

Voraussetzung ist ETS2 in der 64-Bit-Fassung ab Version 1.35 (Telemetry SDK mit Gameplay-Events).

Normalerweise erledigt das das Setup (siehe `README.md` im Wurzelordner): Es legt `spedhub_plugin.dll` neben den Launcher und ruft am Ende `spedhub-launcher.exe --install-plugin` mit Adminrechten auf. Der Launcher sucht ETS2 über Steam in allen Bibliotheken und kopiert die DLL nach `<Spielordner>\bin\win_x64\plugins`, eine ältere ersetzt er. Findet er das Spiel nicht, lässt er den Ordner wählen. Im Launcher steht danach "Plugin installiert", "Plugin veraltet" oder "Plugin fehlt", mit dem Knopf "Plugin installieren". Details in `crates/spedhub-launcher/README.md`, Abschnitt Plugin. Die Deinstallation von SpedHub entfernt das Plugin wieder.

Die Konfiguration `plugin.json` schreibt der Launcher, sobald du angemeldet bist und eine Firma gewählt hast.

Beim nächsten Start fragt ETS2 einmal nach, ob es Plugins laden darf, das mit OK bestätigen. In der Spielkonsole und in `Dokumente\Euro Truck Simulator 2\game.log.txt` steht dann `SpedHub: aktiv`.

### Von Hand, als Rückfall

1. Die DLL holen: aus `Programme\SpedHub` nach dem Setup, als Artefakt `spedhub-plugin-windows-x64` aus dem letzten Lauf der CI auf GitHub (Actions, Lauf öffnen, unten bei Artifacts), oder selbst bauen:

   ```powershell
   cd C:\dev\SpedHub
   cargo build --release -p spedhub-plugin
   ```

   Die DLL liegt dann unter `target\release\spedhub_plugin.dll`.

2. Den Ordner `plugins` im Spielordner anlegen, falls es ihn noch nicht gibt, und die DLL hineinkopieren. Bei Steam ist das meist:

   ```powershell
   $plugins = "C:\Program Files (x86)\Steam\steamapps\common\Euro Truck Simulator 2\bin\win_x64\plugins"
   New-Item -ItemType Directory -Force $plugins
   Copy-Item target\release\spedhub_plugin.dll $plugins
   ```

3. Die Konfiguration anlegen, falls der Launcher sie nicht schreibt: `%LOCALAPPDATA%\SpedHub\plugin.json`

   ```json
   {
     "server_url": "http://127.0.0.1:8787",
     "token": "<token aus dem Dashboard oder von /api/login>",
     "company_id": 1
   }
   ```

   Den Token gibt es zum Beispiel mit PowerShell:

   ```powershell
   $r = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:8787/api/login" -ContentType "application/json" -Body (@{ name = "Alex"; password = "geheim1234" } | ConvertTo-Json)
   New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\SpedHub"
   @{ server_url = "http://127.0.0.1:8787"; token = $r.token; company_id = 1 } | ConvertTo-Json | Set-Content "$env:LOCALAPPDATA\SpedHub\plugin.json"
   ```

4. Das Spiel starten und die Nachfrage nach Plugins mit OK bestätigen.

Zum Entfernen die DLL wieder aus dem Ordner `plugins` löschen.

## Dateien

Alles liegt in `%LOCALAPPDATA%\SpedHub`:

| Datei | Inhalt |
|---|---|
| `plugin.json` | Server, Token, Firma. Fehlt sie oder ist sie kaputt, bleibt das Plugin still und schreibt nur ins Log |
| `plugin.log` | Was das Plugin tut. Ab 256 KB wird es zu `plugin.log.1`, höchstens zwei Dateien |
| `plugin-outbox.jsonl` | Postausgang: Events, die noch nicht beim Server sind |
| `plugin-rejected.jsonl` | Events, die der Server dauerhaft abgelehnt hat, zur Fehlersuche |
| `plugin-state.json` | Kennung dieser Installation und Startzähler für eindeutige `event_id`s |

Der Token steht nur in `plugin.json`, nie im Log.

## Wie es arbeitet

- Das Spiel ruft das Plugin auf seinem Haupt-Thread auf. Dort wird das Event nur übersetzt, bekommt eine `event_id` und den Zeitpunkt `occurred_at` und wandert in eine Warteschlange. Kein Dateizugriff, kein Netz.
- Kanäle und Konfigurationen kopiert der Spiel-Thread in einen Snapshot hinter einem Mutex, der nur für das Kopieren gehalten wird. Der Herzschlag-Thread liest ihn alle 10 Sekunden.
- Ein eigener Thread schreibt jedes Event zuerst in den Postausgang und schickt es dann an `POST /api/companies/{id}/events`.
- Ist der Server nicht erreichbar, bleibt das Event im Postausgang. Der Thread versucht es nach 5 Sekunden wieder, dann mit wachsender Pause bis zu 5 Minuten, und beim nächsten Spielstart sowieso. Doppelt senden schadet nicht, der Server bucht jede `event_id` nur einmal.
- Antwortet der Server mit 400, ist das Event kaputt. Es wandert nach `plugin-rejected.jsonl` und wird nicht wieder gesendet. Bei allen anderen Fehlern, auch bei abgelaufener Anmeldung, bleibt es im Postausgang.
- Eine `event_id` sieht so aus: `<Kennung der Installation>-<Startzähler>-<laufende Nummer>`. Der Startzähler steigt bei jedem Start und wird gespeichert, bevor die erste ID vergeben wird. So gibt es auch nach einem Neustart keine Doppelten.
- Eine Anfrage dauert höchstens 3 Sekunden. Beim Beenden wartet das Plugin höchstens so lange auf eine laufende Anfrage, Sender- und Herzschlag-Thread bekommen das Signal gleichzeitig. Was nicht raus ist, bleibt im Postausgang.
- Keine Panic läuft ins Spiel: Das Framework fängt sie an den Einstiegen der DLL ab, und jeder Hook von SpedHub ist zusätzlich abgesichert.
- Proxy-Einstellungen aus Umgebungsvariablen werden nicht beachtet.
- Jede Anfrage trägt die Version des Plugins im Header `X-SpedHub-Version`, wie beim Launcher.

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/ffi.rs` | Dünne Schicht zum SDK: Anmeldung für Gameplay-Events, Kanäle und Konfigurationen, Werte kopieren, weiterreichen |
| `src/agent.rs` | Warteschlange, Sender-Thread und Herzschlag-Thread |
| `src/telemetry.rs` | Snapshot der laufenden Werte, daraus der Herzschlag |
| `src/event.rs` | Übersetzung der Events in Buchungen |
| `src/ids.rs` | Eindeutige `event_id`s über Neustarts |
| `src/outbox.rs` | Postausgang |
| `src/send.rs` | HTTP an den Server |
| `src/config.rs` | `plugin.json` und Ablageorte |
| `src/log.rs` | Log mit Rotation |
| `tests/logic.rs` | Übersetzung, IDs, Konfiguration, Log, Postausgang, Herzschlag aus dem Snapshot, Kennzeichen |
| `tests/server.rs` | Senden an einen echten spedhub-server, Ausfall, Neustart, doppeltes Senden, Herzschlag, Pause, Fracht und Städte, Version im Header |

Außer `ffi.rs` lässt sich alles ohne Spiel testen:

```powershell
cd C:\dev\SpedHub
cargo test -p spedhub-plugin
```

## SDK und Lizenzen

Die Anbindung an das SDK übernimmt `scs-sdk-plugin` 0.1.1 (MIT oder Apache-2.0, github.com/AptS-1547/scs-sdk-crates). Es erzeugt die Exporte `scs_telemetry_init` und `scs_telemetry_shutdown` und kapselt alle unsicheren Zugriffe. Das SCS Telemetry SDK selbst steht unter einer MIT-artigen Lizenz von SCS Software (Copyright 2013 und 2016). Die Crates enthalten diese Texte.

Gesendet wird mit `ureq` (MIT oder Apache-2.0) über rustls. Darunter liegen `ring` (Apache-2.0 und ISC) und `webpki-roots` (CDLA-Permissive-2.0, die Liste der Stammzertifikate).
