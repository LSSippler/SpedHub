# SpedHub-Plugin für ETS2

Das Plugin meldet Gameplay-Events aus Euro Truck Simulator 2 an den SpedHub-Server. Der bucht sie in die Firmenkasse und auf die Lohnkonten. Es nutzt nur das offizielle Telemetry SDK von SCS und schreibt nichts ins Spiel.

Wichtig: Ob ETS2 die DLL lädt und ob die Events im Spiel wirklich ankommen, ist noch nicht im Spiel geprüft. Getestet ist bisher alles ohne Spiel, siehe unten.

## Was gemeldet wird

| Im Spiel | Event im SDK | Buchung | Betrag |
|---|---|---|---|
| Auftrag abgeliefert | `job.delivered` | `job_delivered` | Erlös (`revenue`). Strecke und Frachtschaden stehen in der Notiz |
| Auftrag abgebrochen | `job.cancelled` | `job_cancelled` | Vertragsstrafe (`cancel.penalty`) |
| Strafe | `player.fined` | `fined` | `fine.amount`, der Grund steht in der Notiz |
| Maut | `player.tollgate.paid` | `tollgate` | `pay.amount` |
| Fähre | `player.use.ferry` | `ferry` | `pay.amount`, Start und Ziel in der Notiz |
| Zug | `player.use.train` | `train` | `pay.amount`, Start und Ziel in der Notiz |

Wer was zahlt, regelt der Server, siehe `server/README.md`, Abschnitt Buchungsregeln.

## Installation

Voraussetzung ist ETS2 in der 64-Bit-Fassung ab Version 1.35 (Telemetry SDK mit Gameplay-Events).

1. Die DLL holen: entweder als Artefakt `spedhub-plugin-windows-x64` aus dem letzten Lauf der CI auf GitHub (Actions, Lauf öffnen, unten bei Artifacts), oder selbst bauen:

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

3. Die Konfiguration anlegen. Später schreibt sie der Launcher, bis dahin von Hand: `%LOCALAPPDATA%\SpedHub\plugin.json`

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

4. Das Spiel starten. Beim Start fragt ETS2 einmal nach, ob es Plugins laden darf, das mit OK bestätigen. In der Spielkonsole und in `Dokumente\Euro Truck Simulator 2\game.log.txt` steht dann `SpedHub: aktiv`.

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

- Das Spiel ruft das Plugin auf seinem Haupt-Thread auf. Dort wird das Event nur übersetzt, bekommt eine `event_id` und wandert in eine Warteschlange. Kein Dateizugriff, kein Netz.
- Ein eigener Thread schreibt jedes Event zuerst in den Postausgang und schickt es dann an `POST /api/companies/{id}/events`.
- Ist der Server nicht erreichbar, bleibt das Event im Postausgang. Der Thread versucht es nach 5 Sekunden wieder, dann mit wachsender Pause bis zu 5 Minuten, und beim nächsten Spielstart sowieso. Doppelt senden schadet nicht, der Server bucht jede `event_id` nur einmal.
- Antwortet der Server mit 400, ist das Event kaputt. Es wandert nach `plugin-rejected.jsonl` und wird nicht wieder gesendet. Bei allen anderen Fehlern, auch bei abgelaufener Anmeldung, bleibt es im Postausgang.
- Eine `event_id` sieht so aus: `<Kennung der Installation>-<Startzähler>-<laufende Nummer>`. Der Startzähler steigt bei jedem Start und wird gespeichert, bevor die erste ID vergeben wird. So gibt es auch nach einem Neustart keine Doppelten.
- Eine Anfrage dauert höchstens 3 Sekunden. Beim Beenden wartet das Plugin höchstens so lange auf eine laufende Anfrage. Was nicht raus ist, bleibt im Postausgang.
- Keine Panic läuft ins Spiel: Das Framework fängt sie an den Einstiegen der DLL ab, und jeder Hook von SpedHub ist zusätzlich abgesichert.
- Proxy-Einstellungen aus Umgebungsvariablen werden nicht beachtet.

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/ffi.rs` | Dünne Schicht zum SDK: Anmeldung für Gameplay-Events, Werte kopieren, weiterreichen |
| `src/agent.rs` | Warteschlange und Sender-Thread |
| `src/event.rs` | Übersetzung der Events in Buchungen |
| `src/ids.rs` | Eindeutige `event_id`s über Neustarts |
| `src/outbox.rs` | Postausgang |
| `src/send.rs` | HTTP an den Server |
| `src/config.rs` | `plugin.json` und Ablageorte |
| `src/log.rs` | Log mit Rotation |
| `tests/logic.rs` | Übersetzung, IDs, Konfiguration, Log, Postausgang |
| `tests/server.rs` | Senden an einen echten spedhub-server, Ausfall, Neustart, doppeltes Senden |

Außer `ffi.rs` lässt sich alles ohne Spiel testen:

```powershell
cd C:\dev\SpedHub
cargo test -p spedhub-plugin
```

## SDK und Lizenzen

Die Anbindung an das SDK übernimmt `scs-sdk-plugin` 0.1.1 (MIT oder Apache-2.0, github.com/AptS-1547/scs-sdk-crates). Es erzeugt die Exporte `scs_telemetry_init` und `scs_telemetry_shutdown` und kapselt alle unsicheren Zugriffe. Das SCS Telemetry SDK selbst steht unter einer MIT-artigen Lizenz von SCS Software (Copyright 2013 und 2016). Die Crates enthalten diese Texte.

Gesendet wird mit `ureq` (MIT oder Apache-2.0) über rustls. Darunter liegen `ring` (Apache-2.0 und ISC) und `webpki-roots` (CDLA-Permissive-2.0, die Liste der Stammzertifikate).
