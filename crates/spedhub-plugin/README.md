# SpedHub-Plugin für ETS2

Das Plugin meldet Gameplay-Events aus Euro Truck Simulator 2 an den SpedHub-Server. Der bucht sie in die Firmenkasse und auf die Lohnkonten und zählt die gefahrenen Kilometer für den Lohn. Dazu schickt es alle 10 Sekunden einen Herzschlag, damit die Firma sieht, wer gerade mit welchem Lkw fährt. Es nutzt nur das offizielle Telemetry SDK von SCS und schreibt nichts ins Spiel.

Gemeldet wird nur, solange im Spiel das Firmenprofil aktiv ist, siehe [Nur das Firmenprofil](#nur-das-firmenprofil).

Im Spiel bestätigt (ETS2 1.62): Das Spiel lädt die DLL (Log "SpedHub: aktiv"), Herzschläge kommen beim Server an, `job.delivered` kommt mit Fracht, Start- und Zielstadt, Strecke und Zeitpunkt an. Noch nicht im Spiel geprüft ist, ob die Kanäle für den Herzschlag die erwarteten Einheiten liefern, welches Format `license.plate` genau hat und alles zur Erkennung des Profils (siehe dort).

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

Ebenso geht `profile` mit: der Name des ETS2-Profils, das beim Annehmen aktiv war. Das ist immer das Firmenprofil, aus anderen Profilen nimmt das Plugin nichts an. Zeilen im Postausgang von älteren Fassungen haben das Feld nicht und lesen sich weiter. Hat der Launcher dem Server ein Firmenprofil gemeldet, lehnt der Server sie ab, weil sich nicht sagen lässt, aus welchem Profil sie kamen.

## Herzschlag

Alle 10 Sekunden, solange das Spiel läuft und nicht pausiert ist, ein Kennzeichen bekannt ist und das Firmenprofil aktiv ist, geht ein Herzschlag an `POST /api/companies/{id}/heartbeat`. Der Server zeigt den Fahrer damit 30 Sekunden lang als online. Fährt er einen geliehenen Firmen-Lkw, merkt sich der Server den Zustand als letzten bekannten Zustand. Den nimmt er, wenn der Chef den Lkw nach einem Absturz ohne Kilometerstand zurücknimmt.

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
| `profile` | Name des aktiven Profils aus `game.log.txt`, immer das Firmenprofil |

Pausiert ist das Spiel laut SDK am Anfang und nach dem Ereignis `paused`, es läuft nach `started`. Alle Kanäle, Konfigurationen und die Ereignisse für die Pause sind optional angemeldet: Fehlt etwas in einer Spielversion, bleibt der Wert leer und die DLL lädt trotzdem.

Herzschläge kommen nie in den Postausgang. Ist der Server nicht erreichbar, ist der Herzschlag verloren, der nächste kommt 10 Sekunden später. Im Log steht nur der erste Fehler und wann sie wieder ankommen. Ein eigener Thread schickt sie, mit demselben Zeitlimit von 3 Sekunden, ein hängender Herzschlag hält also keine Buchung auf.

Fracht und Städte für `job_delivered`: Das Plugin merkt sich die letzte nicht leere Job-Konfiguration. Das SDK kann sie vor oder nach dem Event `job.delivered` leeren, deshalb gilt die letzte nicht leere. Sie wird mit dem abgelieferten Auftrag gebucht und danach vergessen, ebenso bei einem abgebrochenen Auftrag. Zeilen im Postausgang von älteren Fassungen haben die Felder nicht und lesen sich weiter.

## Nur das Firmenprofil

Ein Fahrer kann in ETS2 mehrere Profile haben. Das Plugin läuft in jedem, gebucht werden soll aber nur, was im Firmenprofil passiert. Ohne diese Prüfung kamen beim Test im Spiel 68 Ereignisse aus einem privaten Profil in die Firmenkasse und aufs Lohnkonto, und der Geldabgleich hat daraus falsche Betriebskosten gebucht.

Das Telemetry SDK nennt das Profil nicht. Das Spiel schreibt aber beim Wählen eines Profils eine Zeile in `Dokumente\Euro Truck Simulator 2\game.log.txt`:

```text
00:00:12.345 : New profile selected: 'Nordfracht'
```

So arbeitet das Plugin damit (`src/profile.rs`):

- **Welches Profil:** Die letzte Zeile mit `New profile selected:` zählt, auch nach einem Profilwechsel im laufenden Spiel. Was davor steht, ist egal. Der Name steht in einfachen Anführungszeichen, er darf Leerzeichen, Umlaute und selbst ein `'` enthalten (es gilt das letzte `'` der Zeile). Die Datei wird als UTF-8 gelesen, kaputte Bytes werden ersetzt.
- **Vergleich:** Der Name aus dem Log wird nach dem Trimmen genau mit `profile_name` aus `plugin.json` verglichen, auch in Groß- und Kleinschreibung. Das Spiel kennt `Nordfracht` und `nordfracht` als zwei Profile (der Ordnername sind die Bytes des Namens als Hex), und der Launcher schreibt den Namen aus dem Ordnernamen. Schreibt das Spiel den Namen im Log anders als im Ordnernamen, passt nichts mehr, dann geht nichts raus. Das ist im Spiel noch nicht geprüft.
- **Wann:** Beim Annehmen jedes Ereignisses und vor jedem Herzschlag, dazu bei jedem Takt des Herzschlag-Threads auch in der Pause. Gelesen werden nur die neuen Bytes seit dem letzten Blick, das kostet fast nichts. Eine Zeile ohne Zeilenende zählt schon, wenn der Name vollständig ist.
- **Neuer Spielstart:** Das Spiel schreibt die Datei bei jedem Start neu. Ist sie kürzer als die gemerkte Stelle, haben sich ihre ersten 64 Bytes geändert oder ihre Erstellungszeit, liest das Plugin wieder von vorn. Die Änderungszeit taugt dafür nicht, sie ändert sich mit jeder Zeile.
- **Nur Dateizugriff:** Das Plugin liest die Datei, es schreibt nichts und greift nicht in das Spiel ein.

| Zustand | Ereignisse | Herzschlag | Log |
|---|---|---|---|
| Firmenprofil aktiv | angenommen, mit `profile` | geht raus, mit `profile` | "Aktives Profil 'Nordfracht' ist das Firmenprofil" |
| anderes Profil aktiv | nicht angenommen (`Submitted::OtherProfile`), nicht im Postausgang | nicht gesendet | "Aktives Profil 'Privat' ist nicht das Firmenprofil 'Nordfracht'", dazu je Ereignis "Ereignis nicht angenommen: fined 300 Euro, aus Profil 'Privat', ..." |
| Profil unbekannt: `game.log.txt` fehlt, `game_log` fehlt in `plugin.json` oder das Spiel hat noch kein Profil gewählt | nicht angenommen (`Submitted::UnknownProfile`) | nicht gesendet | "Aktives Profil unbekannt", je Ereignis "Ereignis nicht angenommen: ..., aktives Profil unbekannt" |
| `plugin.json` ohne `profile_name` (älterer Launcher) | nicht angenommen (`Submitted::NoCompanyProfile`) | nicht gesendet | "plugin.json nennt kein Firmenprofil (profile_name). Der Launcher muss die Konfiguration neu schreiben ...". Auch der Postausgang wartet, bis der Launcher die Datei neu geschrieben hat |

Ein Zustandswechsel steht einmal im Log, nicht bei jedem Takt. Nicht angenommene Ereignisse stehen nur im Log, mit Art, Betrag und Profilname, nie mit Token. Sie kommen auch später nicht nach: Der Postausgang enthält nur Ereignisse aus dem Firmenprofil.

Fehlt `game_log` in `plugin.json`, sucht das Plugin `game.log.txt` nicht selbst. Der Dokumente-Ordner kann umgeleitet sein (OneDrive), sicher finden ließe er sich nur über die Windows-API (`SHGetKnownFolderPath`) und damit eine neue Abhängigkeit. Ein geratener Pfad könnte ein altes Log treffen und Ereignisse aus dem falschen Profil durchlassen. Der Launcher schreibt den Pfad immer mit.

Der Server prüft ein zweites Mal: Hat der Launcher das Firmenprofil gemeldet, bucht er nur Ereignisse, deren `profile` genau dieses ist. Andere und solche ohne `profile` (etwa aus einem Postausgang von einem älteren Plugin) lehnt er mit 422 ab und zeigt sie dem Chef. Für das Plugin heißt 422 wie 400: endgültig abgelehnt, das Ereignis kommt nach `plugin-rejected.jsonl` und wird nicht wieder gesendet, die Ereignisse dahinter gehen weiter raus. Einen Herzschlag aus einem anderen Profil verwirft der Server mit `recorded: false` und einem Hinweis, der einmal im Log steht ("Herzschlag nicht angekommen: Herzschlag verworfen. ...").

## Installation

Voraussetzung ist ETS2 in der 64-Bit-Fassung ab Version 1.35 (Telemetry SDK mit Gameplay-Events).

Normalerweise erledigt das das Setup (siehe `README.md` im Wurzelordner): Es legt `spedhub_plugin.dll` neben den Launcher und ruft am Ende `spedhub-launcher.exe --install-plugin` mit Adminrechten auf. Der Launcher sucht ETS2 über Steam in allen Bibliotheken und kopiert die DLL nach `<Spielordner>\bin\win_x64\plugins`, eine ältere ersetzt er. Findet er das Spiel nicht, lässt er den Ordner wählen. Im Launcher steht danach "Plugin installiert", "Plugin veraltet" oder "Plugin fehlt", mit dem Knopf "Plugin installieren". Details in `crates/spedhub-launcher/README.md`, Abschnitt Plugin. Die Deinstallation von SpedHub entfernt das Plugin wieder.

Die Konfiguration `plugin.json` schreibt der Launcher, sobald du angemeldet bist und eine Firma gewählt hast, und neu bei jedem Wechsel von Firma oder Firmenprofil. Das Plugin liest sie nur beim Start des Spiels.

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
     "company_id": 1,
     "profile_name": "Nordfracht",
     "game_log": "C:\\Users\\Alex\\Documents\\Euro Truck Simulator 2\\game.log.txt"
   }
   ```

   | Feld | Bedeutung |
   |---|---|
   | `server_url` | Adresse des Servers, `http://` oder `https://` |
   | `token` | Anmeldung, steht nie im Log |
   | `company_id` | Firma, an die gemeldet wird |
   | `profile_name` | Name des Firmenprofils, so wie das Spiel ihn anzeigt. Fehlt er, sendet das Plugin nichts |
   | `game_log` | Vollständiger Pfad zu `game.log.txt`. Fehlt er, ist das Profil unbekannt und es geht nichts raus |

   Den Token gibt es zum Beispiel mit PowerShell:

   ```powershell
   $r = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:8787/api/login" -ContentType "application/json" -Body (@{ name = "Alex"; password = "geheim1234" } | ConvertTo-Json)
   New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\SpedHub"
   @{ server_url = "http://127.0.0.1:8787"; token = $r.token; company_id = 1; profile_name = "Nordfracht"; game_log = "$([Environment]::GetFolderPath('MyDocuments'))\Euro Truck Simulator 2\game.log.txt" } | ConvertTo-Json | Set-Content "$env:LOCALAPPDATA\SpedHub\plugin.json"
   ```

4. Das Spiel starten und die Nachfrage nach Plugins mit OK bestätigen.

Zum Entfernen die DLL wieder aus dem Ordner `plugins` löschen.

## Dateien

Alles liegt in `%LOCALAPPDATA%\SpedHub`:

| Datei | Inhalt |
|---|---|
| `plugin.json` | Server, Token, Firma, Firmenprofil und Pfad zu `game.log.txt`. Fehlt sie oder ist sie kaputt, bleibt das Plugin still und schreibt nur ins Log. Ohne Firmenprofil sendet es nichts |
| `plugin.log` | Was das Plugin tut. Ab 256 KB wird es zu `plugin.log.1`, höchstens zwei Dateien |
| `plugin-outbox.jsonl` | Postausgang: Events, die noch nicht beim Server sind |
| `plugin-rejected.jsonl` | Events, die der Server dauerhaft abgelehnt hat, zur Fehlersuche |
| `plugin-state.json` | Kennung dieser Installation und Startzähler für eindeutige `event_id`s |

Der Token steht nur in `plugin.json`, nie im Log.

## Wie es arbeitet

- Das Spiel ruft das Plugin auf seinem Haupt-Thread auf. Dort wird das Event übersetzt, das aktive Profil geprüft (nur die neuen Bytes von `game.log.txt`), und aus dem Firmenprofil bekommt es eine `event_id`, den Zeitpunkt `occurred_at` und den Profilnamen `profile` und wandert in eine Warteschlange. Kein Netz, geschrieben wird nichts.
- Kanäle und Konfigurationen kopiert der Spiel-Thread in einen Snapshot hinter einem Mutex, der nur für das Kopieren gehalten wird. Der Herzschlag-Thread liest ihn alle 10 Sekunden.
- Ein eigener Thread schreibt jedes Event zuerst in den Postausgang und schickt es dann an `POST /api/companies/{id}/events`.
- Ist der Server nicht erreichbar, bleibt das Event im Postausgang. Der Thread versucht es nach 5 Sekunden wieder, dann mit wachsender Pause bis zu 5 Minuten, und beim nächsten Spielstart sowieso. Doppelt senden schadet nicht, der Server bucht jede `event_id` nur einmal.
- Antwortet der Server mit 400 oder 422, ist das Event kaputt oder kommt nicht aus dem gemeldeten Firmenprofil. Es wandert nach `plugin-rejected.jsonl` und wird nicht wieder gesendet, die Events dahinter gehen weiter raus. Bei allen anderen Fehlern, auch bei abgelaufener Anmeldung, bleibt es im Postausgang.
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
| `src/profile.rs` | Aktives Profil aus `game.log.txt`, Stück für Stück gelesen, Vergleich mit dem Firmenprofil |
| `src/send.rs` | HTTP an den Server |
| `src/config.rs` | `plugin.json` und Ablageorte |
| `src/log.rs` | Log mit Rotation |
| `tests/logic.rs` | Übersetzung, IDs, Konfiguration, Log, Postausgang, Herzschlag aus dem Snapshot, Kennzeichen, Profilname aus dem Log, Lesen in Stücken und nach neuem Spielstart, Ereignisse aus anderem oder unbekanntem Profil, alte `plugin.json` |
| `tests/server.rs` | Senden an einen echten spedhub-server, Ausfall, Neustart, doppeltes Senden, Herzschlag, Pause, Fracht und Städte, Version im Header, nur aus dem Firmenprofil, Profilwechsel im laufenden Spiel, vom Server abgelehnte und verworfene Meldungen |

Außer `ffi.rs` lässt sich alles ohne Spiel testen:

```powershell
cd C:\dev\SpedHub
cargo test -p spedhub-plugin
```

## SDK und Lizenzen

Die Anbindung an das SDK übernimmt `scs-sdk-plugin` 0.1.1 (MIT oder Apache-2.0, github.com/AptS-1547/scs-sdk-crates). Es erzeugt die Exporte `scs_telemetry_init` und `scs_telemetry_shutdown` und kapselt alle unsicheren Zugriffe. Das SCS Telemetry SDK selbst steht unter einer MIT-artigen Lizenz von SCS Software (Copyright 2013 und 2016). Die Crates enthalten diese Texte.

Gesendet wird mit `ureq` (MIT oder Apache-2.0) über rustls. Darunter liegen `ring` (Apache-2.0 und ISC) und `webpki-roots` (CDLA-Permissive-2.0, die Liste der Stammzertifikate).
