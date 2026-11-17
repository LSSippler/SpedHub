# SpedHub-Server

Das Backend von SpedHub: Accounts, Firmenkasse und Fuhrpark. Die Version steht in der `Cargo.toml` im Wurzelordner und gilt für alle Teile von SpedHub.

Der Server ist die Wahrheit. Kontostand und geliehene Lkw im Spielstand sind nur Kopien, die der Launcher aus dem Serverstand neu schreibt. Mehr dazu in `docs/architektur.md`.

## Starten

Voraussetzung ist ein aktuelles Rust, unter Windows mit der MSVC-Toolchain (wegen SQLite braucht es den C-Compiler aus den Visual Studio Build Tools). Fertig gebaut gibt es den Server im Setup ("Server für lokale Tests") und als `spedhub-server-linux-x86_64` in den Releases.

```powershell
cd C:\dev\SpedHub
rustup update
cargo test --workspace
cargo run --release -p spedhub-server
```

Danach läuft der Server auf http://127.0.0.1:8787 und legt im aktuellen Ordner die Datei `spedhub.db` an. Beenden mit Strg+C.

### Aus dem Setup

Mit dem Teil "Server für lokale Tests" aus dem Setup gibt es im Startmenü "SpedHub-Server (lokaler Test)". Das startet `spedhub-server.exe --local`: Adresse `127.0.0.1:8787`, Datenbank in `%LOCALAPPDATA%\SpedHub\server\spedhub.db` des Benutzers, der die Verknüpfung startet. Der Ordner wird angelegt. Beenden mit Strg+C oder Fenster schließen.

### Kommandozeile

| Aufruf | Was passiert |
|---|---|
| `spedhub-server` | Datenbank `spedhub.db` im aktuellen Ordner, Adresse `127.0.0.1:8787` |
| `spedhub-server --local` | Datenbank in `%LOCALAPPDATA%\SpedHub\server`, unter Linux `~/.local/share/SpedHub/server`, Adresse `127.0.0.1:8787` |
| `spedhub-server --version` | Version ausgeben, etwa `spedhub-server 0.2.0` |

`SPEDHUB_DB` und `SPEDHUB_ADDR` (siehe Einstellungen) gehen immer vor, auch vor `--local`. `--local` gibt es, weil eine Verknüpfung keine Umgebungsvariablen setzen kann.

## Dashboard

Unter http://127.0.0.1:8787 liegt die Weboberfläche. Dort kannst du:

- ein Konto anlegen und dich anmelden
- eine Firma gründen oder mit einem Einladungscode beitreten
- den Fuhrpark sehen, Lkw ausleihen und zurückgeben, darunter eingeklappt die ausgemusterten und verkauften Lkw
- bei den Fahrern sehen, wer gerade online ist (grüner Punkt), mit welchem Lkw, in welcher Stadt und mit welchem Auftrag, dazu Level, Firmenkilometer, Lohnstufe, Lohnkonto und offenen Lohn
- im Fuhrpark sehen, ob ein verliehener Lkw gerade gefahren wird, wie viel Schaden jeder Lkw hat (reparierbar und dauerhaft) und welcher in der Werkstatt steht
- in der Kopfzeile das Passwort ändern
- im Kassenbuch alle Buchungen sehen, die Lohnabrechnungen als eine Zeile je Abrechnung zum Aufklappen
- als Chef: sehen, wer einen Firmen-Lkw fährt, ohne ihn geliehen zu haben, einen Lkw ohne Kilometerstand mit dem letzten bekannten Zustand zurücknehmen, Lkw anlegen, Lkw reparieren lassen, Lkw verkaufen (mit Erlös) und ausmustern, im Büro ein Anschaffungsbudget freigeben und schließen, im Kassenbuch Betriebskosten als Lkw-Kauf mit einem Lkw verknüpfen, Tagespauschale, Uhrzeit der Abrechnung, Betriebsvorschuss, Erlösfaktor, Lohnstufen, Schadensgrenze, Satz der Werkstatt und Haftung einstellen, Geld zwischen Kasse und Fahrern überweisen, ungeklärte Beträge aus dem Geldabgleich sehen und schließen
- im Testbereich "Ereignis von Hand buchen" Ereignisse mit Strecke buchen, einen Geldabgleich durchspielen, einen Herzschlag senden und die Lohnabrechnung sofort auslösen. Im Spiel melden Ereignisse und Herzschlag das Plugin aus `crates/spedhub-plugin` und den Abgleich der Launcher

Die Seite holt sich alle 5 Sekunden den neuen Stand. Sie steckt als eine einzelne Datei in `server/web/index.html` und wird beim Bauen fest in die .exe eingebaut. Nach einer Änderung an der Datei also neu starten:

```powershell
cd C:\dev\SpedHub
cargo run --release -p spedhub-server
```

## Einstellungen

| Variable | Standard | Bedeutung |
|---|---|---|
| `SPEDHUB_ADDR` | `127.0.0.1:8787` | Adresse und Port. Für Zugriff aus dem Netz `0.0.0.0:8787` |
| `SPEDHUB_DB` | `spedhub.db` | Pfad zur Datenbank |

Beispiel mit anderem Port:

```powershell
cd C:\dev\SpedHub
$env:SPEDHUB_ADDR = "127.0.0.1:9000"
cargo run --release -p spedhub-server
```

Achtung: Der Server spricht nur HTTP. Solange er auf 127.0.0.1 läuft, ist das egal. Für den Betrieb im Internet gehört ein Reverse Proxy mit HTTPS davor.

## Auf einem VPS

Für Linux-Server gibt es in jedem Release `spedhub-server-linux-x86_64`: x86_64, statisch mit musl gelinkt, braucht also keine Bibliotheken auf dem Server. Ein Beispiel mit systemd:

```bash
sudo useradd --system --home /opt/spedhub --shell /usr/sbin/nologin spedhub
sudo mkdir -p /opt/spedhub && cd /opt/spedhub
sudo curl -fLO https://github.com/LSSippler/SpedHub/releases/download/v0.2.0/spedhub-server-linux-x86_64
sudo curl -fLO https://github.com/LSSippler/SpedHub/releases/download/v0.2.0/SHA256SUMS.txt
sha256sum -c --ignore-missing SHA256SUMS.txt
sudo chmod +x spedhub-server-linux-x86_64
sudo chown -R spedhub: /opt/spedhub
./spedhub-server-linux-x86_64 --version
```

`/etc/systemd/system/spedhub.service`:

```ini
[Unit]
Description=SpedHub-Server
After=network-online.target

[Service]
User=spedhub
WorkingDirectory=/opt/spedhub
Environment=SPEDHUB_DB=/opt/spedhub/spedhub.db
Environment=SPEDHUB_ADDR=127.0.0.1:8787
ExecStart=/opt/spedhub/spedhub-server-linux-x86_64
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now spedhub
```

Davor gehört ein Reverse Proxy mit HTTPS (siehe oben).

Für ein Update die neue Datei erst in einem eigenen Ordner laden und prüfen (wie oben, aber zum Beispiel in `/tmp/spedhub-update` statt in `/opt/spedhub`). Nicht direkt über die laufende Datei laden, das scheitert mit "Text file busy". Dann:

```bash
sudo systemctl stop spedhub
sudo mkdir -p /opt/spedhub/backup/vor-0.2.1
sudo sh -c 'cp -p /opt/spedhub/spedhub.db* /opt/spedhub/backup/vor-0.2.1/'
sudo install -m 755 -o spedhub spedhub-server-linux-x86_64 /opt/spedhub/
sudo systemctl start spedhub
```

`spedhub.db*` nimmt eine vorhandene `spedhub.db-wal` mit, sie gehört zur Datenbank. Bei laufendem Dienst geht die Sicherung auch mit `sudo -u spedhub sqlite3 /opt/spedhub/spedhub.db ".backup /opt/spedhub/backup/spedhub-vorher.db"`. Eine vorhandene Datenbank erweitert der Server beim Start selbst.

Selbst bauen geht unter Linux mit `rustup target add x86_64-unknown-linux-musl`, `sudo apt-get install -y musl-tools` und `cargo build --release -p spedhub-server --target x86_64-unknown-linux-musl`.

## Versionsprüfung

Launcher und Plugin schicken bei jeder Anfrage ihre Version im Header `X-SpedHub-Version`. `POST /api/register` und `POST /api/login` antworten zusätzlich mit `server_version` und `compatible`. Zusammen passen Versionen mit gleicher Haupt- und Nebenversion, `0.2.x` mit `0.2.y`. Passt die Version nicht, meldet sich der Client trotzdem an, der Launcher zeigt einen gelben Hinweis und der Server schreibt eine Warnung ins Log, etwa `Warnung: Alex meldet sich mit Version 0.3.0 an, der Server hat 0.2.0`. Ohne Header, also beim Dashboard, das vom Server selbst kommt, ist `compatible` immer `true`. `GET /api/health` nennt die Version unter `version`.

## Ausprobieren ohne Dashboard

Dasselbe geht auch direkt über die Schnittstelle. Der Server muss dafür in einem zweiten Fenster laufen.

```powershell
$b = "http://127.0.0.1:8787"
$j = "application/json"

# Account anlegen und Firma gründen
$r = Invoke-RestMethod -Method Post -Uri "$b/api/register" -ContentType $j -Body (@{ name = "Alex"; password = "geheim1234" } | ConvertTo-Json)
$h = @{ Authorization = "Bearer $($r.token)" }
$f = Invoke-RestMethod -Method Post -Uri "$b/api/companies" -Headers $h -ContentType $j -Body (@{ name = "Nordfracht Logistik" } | ConvertTo-Json)
$f

# Lkw anlegen und ausleihen
$t = Invoke-RestMethod -Method Post -Uri "$b/api/companies/$($f.id)/trucks" -Headers $h -ContentType $j -Body (@{ plate = "SPED 001"; model = "scania.s_2016"; odometer_km = 3865 } | ConvertTo-Json)
Invoke-RestMethod -Method Post -Uri "$b/api/trucks/$($t.id)/checkout" -Headers $h

# Auftrag abliefern: 1000 Euro Erlös, 500 km
Invoke-RestMethod -Method Post -Uri "$b/api/companies/$($f.id)/events" -Headers $h -ContentType $j -Body (@{ event_id = "job-1"; type = "job_delivered"; amount = 1000; distance_km = 500 } | ConvertTo-Json)

# Lohn sofort abrechnen statt bis morgen 4:00 zu warten
Invoke-RestMethod -Method Post -Uri "$b/api/companies/$($f.id)/payrolls/run" -Headers $h | ConvertTo-Json -Depth 5

# Lkw zurückgeben und Firma ansehen
Invoke-RestMethod -Method Post -Uri "$b/api/trucks/$($t.id)/checkin" -Headers $h -ContentType $j -Body (@{ odometer_km = 4200 } | ConvertTo-Json)
Invoke-RestMethod -Uri "$b/api/companies/$($f.id)" -Headers $h | ConvertTo-Json -Depth 5
```

Am Ende sollte die Firmenkasse 750 Euro zeigen und dein Lohnkonto 250 Euro: 500 km mal 40 Cent in Stufe 1 sind 200 Euro, dazu 50 Euro Tagespauschale.

## Schnittstelle

Alle Anfragen und Antworten sind JSON. Außer bei `register`, `login` und `health` gehört der Header `Authorization: Bearer <token>` dazu. Fehler kommen als `{ "error": "..." }`.

| Methode und Pfad | Wer | Was |
|---|---|---|
| `GET /api/health` | jeder | Lebenszeichen mit `version` |
| `POST /api/register` | jeder | Account anlegen: `name`, `password` (mindestens 8 Zeichen). Liefert `token`, `server_version` und `compatible` (siehe Versionsprüfung) |
| `POST /api/login` | jeder | Anmelden, liefert einen neuen `token`, `server_version` und `compatible` |
| `POST /api/logout` | angemeldet | Abmelden, der benutzte Token wird ungültig |
| `GET /api/me` | angemeldet | Eigener Account und Firmen |
| `POST /api/me/password` | angemeldet | Passwort ändern: `old_password`, `new_password` (mindestens 8 Zeichen). Alle anderen Tokens des Nutzers werden ungültig, der benutzte bleibt. Liefert `other_tokens_removed`. Falsches altes Passwort: 403 |
| `POST /api/companies` | angemeldet | Firma gründen: `name`. Wer gründet, ist Chef. Die Firma bekommt die fünf Standard-Lohnstufen |
| `POST /api/companies/join` | angemeldet | Beitreten mit `invite_code`, Rolle Fahrer |
| `GET /api/companies/{id}` | Mitglied | Kassenstand, Einstellungen (darunter `damage_limit`, `repair_rate`, `liability_percent`), Lohnstufen, Mitglieder mit Lohnkonto, offenem Lohn, Erfahrung, Level, Firmenkilometern und Lohnstufe. Je Mitglied außerdem `online` und `last_seen`, solange online auch `plate`, `near_city`, `speed_kmh`, `job` und `truck_id` (der eigene geliehene Lkw), siehe Online. Einladungscode, Zahl der offenen ungeklärten Beträge, `foreign_truck` und `budget` (das offene Anschaffungsbudget mit Restbetrag oder `null`) sieht nur der Chef |
| `PATCH /api/companies/{id}` | Chef | Einstellungen ändern, alle optional: `revenue_factor`, `daily_flat`, `payroll_time` (wie `04:00`), `advance`, `damage_limit` (Schadensgrenze in Prozent, 0 bis 100, Standard 25), `repair_rate` (Euro je Prozentpunkt Schaden, 0 bis 1000000, Standard 150), `liability_percent` (Haftung in Prozent, 0 bis 100, Standard 0). Außerhalb der Grenzen 400 |
| `PUT /api/companies/{id}/wage-levels` | Chef | Lohnstufen ersetzen: `levels`, eine Liste aus `min_level`, `min_km`, `cents_per_km`. 1 bis 20 Stufen |
| `GET /api/companies/{id}/trucks` | Mitglied | Fuhrpark mit Info, wer was gerade geliehen hat, und `old_plates` (frühere Kennzeichen, älteste zuerst). Dazu `driving` (der Ausleiher ist online und meldet genau diesen Lkw), dann auch `speed_kmh` und `near_city`, und `last_known` (`at`, `odometer_km`) mit dem letzten bekannten Zustand. Je Lkw außerdem `damage` und `damage_unfixable` (Prozent, siehe Schaden und Werkstatt), `in_workshop` und `repair_cost` (was eine Reparatur jetzt kosten würde). `retired` ist `null` oder `{ reason, at, sale_price }` für ausgemusterte (`reason` `ausgemustert`) und verkaufte Lkw (`verkauft`, mit Erlös); sie stehen hinter den aktiven. Der Chef bekommt `foreign_drivers`: wer den Lkw gerade fährt, ohne ihn geliehen zu haben |
| `POST /api/companies/{id}/trucks` | Chef | Lkw anlegen: `plate`, `model`, optional `package`, `odometer_km`, `state`. 409, wenn ein aktiver Lkw der Firma das Kennzeichen trägt. Trägt es ein ausgemusterter oder verkaufter, bekommt der neue es trotzdem, siehe Anschaffungen |
| `POST /api/trucks/{id}/checkout` | Mitglied | Ausleihen. Liefert Bauplan und Zustand. 409, wenn schon verliehen, in der Werkstatt, ausgemustert oder verkauft. Merkt sich den Schaden beim Ausleihen |
| `GET /api/trucks/{id}/lease` | Ausleiher oder Chef | Die laufende Leihe noch einmal: `lease_id`, `user_id`, `taken_at` und `truck` wie beim Ausleihen, mit Bauplan, Zustand und Kennzeichen. 403 für alle anderen, 409, wenn der Lkw nicht verliehen ist |
| `POST /api/trucks/{id}/checkin` | Ausleiher oder Chef | Zurückgeben: `odometer_km`, optional `state` und `plate`. Mit `plate` bekommt der Lkw ein neues Kennzeichen, das alte kommt in `old_plates`. 409, wenn ein anderer Lkw der Firma das Kennzeichen gerade hat. Ohne `odometer_km` nimmt der Chef den Lkw mit dem letzten bekannten Zustand zurück, die Antwort sagt unter `from_last_known.at`, von wann er ist. Ohne letzten Zustand 409, ein Fahrer ohne Kilometerstand bekommt 400. Die Antwort enthält außerdem `damage`, `damage_unfixable`, `damage_out` (beim Ausleihen), `damage_new` (dazugekommen), `liability` (was der Ausleiher zahlt), `liability_user_id`, `damage_limit` und `in_workshop` |
| `POST /api/trucks/{id}/retire` | Chef | Lkw ausmustern, siehe Anschaffungen. Gebucht wird nichts. 409, wenn er verliehen oder schon ausgemustert oder verkauft ist. Liefert `retired` und `balance` |
| `POST /api/trucks/{id}/sell` | Chef | Lkw verkaufen: `price` (Erlös in Euro, 0 bis 1000000000, sonst 400). Wie ausmustern, dazu bucht der Server den Erlös als `lkw_verkauf` in die Firmenkasse, bei 0 nichts. Liefert `retired` und `balance` |
| `POST /api/trucks/{id}/repair` | Chef | Lkw reparieren lassen, siehe Schaden und Werkstatt. 409, wenn er verliehen, ausgemustert oder verkauft ist, keinen Schaden hat oder die Kasse nicht reicht. Liefert `cost`, `damage_before`, `damage`, `damage_unfixable`, `rev`, `in_workshop` und den neuen Kassenstand `balance` |
| `POST /api/companies/{id}/heartbeat` | Mitglied | Herzschlag aus dem Spiel, siehe Online: `plate`, optional `odometer_km`, `wear` (`engine`, `transmission`, `cabin`, `chassis`, `wheels`, je 0 bis 1), `fuel_relative` (0 bis 1), `fuel_liters`, `near_city`, `speed_kmh`, `job` (`cargo`, `source_city`, `target_city`) oder `null`. Ohne Kennzeichen oder mit unplausiblen Zahlen 400. Liefert `last_seen` und `leased_truck_id`, wenn das Kennzeichen zum eigenen geliehenen Lkw passt |
| `POST /api/companies/{id}/events` | Mitglied | Ereignis aus dem Spiel buchen: `event_id`, `type`, `amount`, optional `distance_km`, `note` und `occurred_at` (wann es im Spiel passiert ist, Unix-Sekunden). Bei `job_delivered` optional `cargo`, `source_city` und `target_city` (höchstens 64 Zeichen, ohne Steuerzeichen). War es vor einem schon abgeglichenen Spielstand passiert, korrigiert der Server diesen Abgleich, die Antwort sagt das unter `late`. Die Antwort enthält `occurred_at`, leer ohne plausiblen Zeitpunkt |
| `GET /api/companies/{id}/ledger` | Mitglied | Die letzten Buchungen, `?limit=50`. Jede mit `game_amount` (Betrag im Spiel), `distance_km`, `payroll_id`, `event_id`, `occurred_at` (leer, wenn der Client keinen plausiblen Zeitpunkt geschickt hat) und bei Aufträgen `cargo`, `source_city`, `target_city` (leer bei alten Buchungen). Als Lkw-Kauf verknüpfte Buchungen tragen `truck_id` und `truck_plate` (das aktuelle Kennzeichen), alle anderen `null` |
| `POST /api/companies/{id}/ledger/{entry}/link-truck` | Chef | Betriebskosten-Buchung als Kauf eines Lkw verknüpfen: `truck_id`. Die Buchung bleibt unverändert. 404, wenn es die Buchung in der Firma oder den Lkw in der Firma nicht gibt, 400, wenn die Buchung keine `betriebskosten` der Firmenkasse ist, 409, wenn sie schon verknüpft ist |
| `DELETE /api/companies/{id}/ledger/{entry}/link-truck` | Chef | Verknüpfung lösen. 404, wenn die Buchung nicht verknüpft ist |
| `GET /api/companies/{id}/purchase-candidates` | Chef | Die letzten unverknüpften `betriebskosten` des Chefs, neueste zuerst, `?limit=10`: `id`, `created_at`, `amount`, `note` |
| `GET /api/companies/{id}/budgets` | Chef | Alle Anschaffungsbudgets, neueste zuerst, und unter `open` das offene oder `null`. Je Budget `id`, `user_id`, `amount`, `note`, `created_at`, `closed_at`, `closed_by`, `consumed` und `rest` |
| `POST /api/companies/{id}/budgets` | Chef | Anschaffungsbudget freigeben: `amount` (1 bis 1000000000, sonst 400), optional `note`. Gebucht wird nichts. 409, wenn schon eins offen ist oder die Kasse den Betrag nicht deckt |
| `POST /api/companies/{id}/budgets/{bid}/close` | Chef | Budget schließen. Gebucht wird nichts. Die Antwort sagt unter `budget.rest`, was entfällt. 409, wenn es schon geschlossen ist |
| `POST /api/companies/{id}/transfer` | Chef | Geld verschieben: `user_id`, `amount`, optional `note`. Positiv geht aus der Kasse an den Fahrer, negativ zurück |
| `GET /api/companies/{id}/payrolls` | Mitglied | Die letzten Lohnabrechnungen mit einer Zeile je Fahrer, `?limit=30` |
| `POST /api/companies/{id}/payrolls/run` | Chef | Lohn sofort abrechnen: erst fällige Tage, dann alles seit der letzten Abrechnung |
| `POST /api/companies/{id}/sync` | Mitglied | Geldabgleich: `ingame_money`, optional `xp` und `saved_at` (Änderungszeit des gelesenen Spielstands, Unix-Sekunden). Liefert unter anderem `sync_id`, `expected`, `booked`, `unexplained`, `budget` (Rest eines offenen Anschaffungsbudgets, nur beim Chef, sonst 0), `write_money` und `saved_at` |
| `POST /api/companies/{id}/sync/{sync_id}/written` | wer abgeglichen hat | Der Betrag aus dem Abgleich steht jetzt im Spielstand. Optional `written_at`: wann der Launcher geschrieben hat, Unix-Sekunden. Nur für den neuesten eigenen Abgleich |
| `GET /api/companies/{id}/unexplained` | Chef | Offene ungeklärte Beträge, mit `?all=true` auch die geschlossenen |
| `POST /api/companies/{id}/unexplained/{eid}/close` | Chef | Ungeklärten Betrag schließen, optional mit `note`. Gebucht wird nichts |

## Anmeldung und Tokens

Ein Token läuft nach 30 Tagen ohne Benutzung ab. Jede Anfrage mit dem Token zählt als Benutzung (geschrieben wird höchstens einmal je Minute). Ein abgelaufener Token wird gelöscht, die Anfrage bekommt 401 mit "Die Anmeldung ist abgelaufen, bitte neu anmelden". Tokens aus einer Datenbank von vor dieser Fassung gelten ab der Umstellung als benutzt.

`POST /api/me/password` ändert das Passwort und meldet alle anderen Anmeldungen des Nutzers ab, zum Beispiel ein Dashboard auf einem anderen Rechner. Der benutzte Token bleibt gültig. Wird das Passwort im Launcher geändert, bleiben Launcher und Plugin angemeldet, weil sie sich einen Token teilen. Das Dashboard hat einen eigenen Token: Wird das Passwort dort geändert, sind auch Launcher und Plugin abgemeldet, auch auf demselben Rechner. Ein laufendes Plugin liest seinen Token nur beim Start, seine Herzschläge scheitern dann, der Fahrer erscheint offline und es gibt keinen neuen letzten bekannten Zustand. Buchungen bleiben im Postausgang. Danach im Launcher neu anmelden und das Spiel neu starten.

## Online

Das Plugin schickt alle 10 Sekunden einen Herzschlag an `POST /api/companies/{id}/heartbeat`, solange das Spiel läuft und nicht pausiert ist. Online ist, wer sich in den letzten 30 Sekunden gemeldet hat (Uhr des Servers). Je Fahrer und Firma liegt nur der letzte Herzschlag in der Tabelle `presence`, jede Meldung überschreibt die vorige. Das ist Zustand, kein Journal.

Das Kennzeichen wird mit den aktuellen und den alten Kennzeichen (`old_plates`) des Fuhrparks verglichen. Passen mehrere Lkw, gewinnt der, den dieser Fahrer gerade geliehen hat, egal über welches Kennzeichen. Sonst gewinnt ein aktuelles Kennzeichen, dann das zuletzt ersetzte alte. Das ist nötig, weil der Launcher einen Lkw mit dem Kennzeichen aus dem Bauplan einsetzt: Hat das Spiel es einmal ersetzt, fährt der Lkw wieder mit dem alten, das inzwischen ein anderer Lkw tragen kann. Markup wie `<img src=...>`, ein Land hinter `|`, Leerraum und Groß- und Kleinschreibung zählen dabei nicht, genau wie in `spedhub_save::plate_text`.

- Passt es zu einem Firmen-Lkw, den dieser Fahrer gerade geliehen hat, wird der Herzschlag zum letzten bekannten Zustand des Lkw (`trucks.last_seen_at`, `last_odometer_km`, `last_state`). `last_state` hat die Feldnamen von `TruckState` und startet beim Zustand vom Ausleihen. Nur gemeldete Werte überschreiben ihn. Den einen Wert für die Räder bekommt jede Achse. Ein Kilometerstand unter dem beim Ausleihen wird nicht übernommen.
- Passt es zu einem Firmen-Lkw, den er nicht geliehen hat, sieht der Chef eine Markierung: `foreign_truck` beim Mitglied und `foreign_drivers` in der Fuhrparkliste. Fahrer sehen das nicht.
- Nimmt der Chef einen Lkw ohne Kilometerstand zurück, etwa weil das Spiel des Fahrers abgestürzt ist, gelten Kilometerstand und Zustand aus dem letzten bekannten Zustand. Jede Rückgabe leert ihn, damit er nicht beim nächsten Fahrer gilt.

Eine Stadt in der Nähe liefert das Telemetry SDK nicht. Das Feld `near_city` gibt es trotzdem, das Plugin lässt es leer, im Dashboard lässt es sich im Testbereich setzen.

## Kennzeichen

Das Spiel kann das Kennzeichen eines Lkw ersetzen (siehe `docs/save-format.md`, "Im Spiel bestätigt"). Der Launcher meldet das neue deshalb bei der Rückgabe mit. Der Server speichert es als aktuelles Kennzeichen und hängt das alte an `old_plates` an, damit sich spätere Telemetrie mit dem alten Kennzeichen noch zuordnen lässt. Eindeutig je Firma ist nur das aktuelle Kennzeichen. Ein altes darf ein anderer Lkw wieder bekommen.

## Buchungsregeln

Beträge sind ganze Euro. `amount` ist immer positiv, das Vorzeichen ergibt sich aus dem Typ.

| `type` | Lohnkonto des Fahrers | Firmenkasse | `game_amount` |
|---|---|---|---|
| `job_delivered` | | plus Erlös mal Erlösfaktor | plus Erlös |
| `job_cancelled` | minus Betrag | | minus Betrag |
| `fined` | minus Betrag | | minus Betrag |
| `tollgate`, `ferry`, `train` | | minus Betrag | minus Betrag |

Jede Buchung merkt sich in `game_amount` den vollen Betrag aus dem Spiel, mit dem Vorzeichen, mit dem er den Kontostand im Spiel verändert hat, und in `occurred_at`, wann das Ereignis im Spiel passiert ist. Der Erlösfaktor wirkt nur auf die Firmenkasse. Bei `job_delivered` zählt `distance_km` als Firmenkilometer des Fahrers. Die Lohnabrechnung zählt Aufträge nach dem Empfang beim Server (`created_at`), damit abgeschlossene Abrechnungen unberührt bleiben.

Dazu kommen Buchungen aus dem Geldabgleich, alle auf die Firmenkasse und mit dem Fahrer als `user_id`:

| `kind` | Betrag | Wann |
|---|---|---|
| `betriebskosten` | negativ | Im Spiel fehlt Geld, das die gemeldeten Ereignisse nicht erklären |
| `betriebskosten_korrektur` | positiv | Ein spät gemeldetes Ereignis zeigt, dass ein Abgleich zu viel Betriebskosten gebucht hat. Im Kassenbuch "Betriebskosten, Korrektur" |

Und aus Werkstatt und Haftung (siehe Schaden und Werkstatt):

| `kind` | Konto | Betrag | Wann |
|---|---|---|---|
| `werkstatt` | Firmenkasse, `user_id` ist der Chef, der reparieren lässt | negativ | Der Chef lässt einen Lkw reparieren. Notiz wie "Werkstatt SPED 001, 28,5 %". Im Kassenbuch "Werkstatt" |
| `haftung` | Lohnkonto des Ausleihers | negativ | Rückgabe mit neuem Schaden, Haftung über 0. Im Kassenbuch "Haftung" |
| `haftung` | Firmenkasse, `user_id` ist der Ausleiher | positiv | dieselbe Rückgabe, gleicher Betrag. Notiz wie "Haftung SPED 001, 6 Prozentpunkte neuer Schaden" |

Und aus dem Fuhrpark (siehe Anschaffungen):

| `kind` | Konto | Betrag | Wann |
|---|---|---|---|
| `lkw_verkauf` | Firmenkasse, `user_id` ist der Chef | positiv | Der Chef verkauft einen Lkw mit Erlös über 0. Notiz wie "Verkauf SPED 002". Im Kassenbuch "Lkw-Verkauf" |

Einen Lkw-Kauf gibt es nicht als eigene Buchungsart: Der Kauf ist eine `betriebskosten`-Buchung aus dem Geldabgleich, die der Chef mit dem Lkw verknüpft. Das Kassenbuch zeigt sie dann als "Lkw-Kauf SPED 004".

Die Typen entsprechen den Gameplay-Events aus dem Telemetrie-SDK. Jede `event_id` wird pro Firma nur einmal gebucht. Schickt der Client ein Ereignis doppelt, kommt `duplicate: true` zurück und es passiert nichts.

Bei einer Überweisung kann die Kasse nicht ins Minus gehen. Ein Lohnkonto schon, das sind dann Schulden beim Chef.

Die Kasse ist ein Journal. Kassenstand und Lohnkonten sind immer die Summe der Buchungen, es gibt keine einzelne Zahl, die man überschreiben könnte. Offene Lohnforderungen sind ebenfalls ein Journal (`wage_claims`).

Bis zu dieser Fassung bekam der Fahrer bei jedem Auftrag einen Lohnanteil in Prozent (`wage_percent`), der Rest ging in die Kasse. Das ist ersetzt. Alte Buchungen bleiben, wie sie sind, und stehen im Kassenbuch als "Lohnanteil (altes Modell)". Eine vorhandene Datenbank wird beim Start erweitert, abgerechnet wird ab dem Zeitpunkt der Umstellung.

## Lohn

Einmal am Tag zur eingestellten Uhrzeit (Standard 4:00, Zeitzone Europe/Berlin) rechnet der Server den Vortag ab. Je Fahrer:

- Kilometer aus abgelieferten Aufträgen mal Cent je km seiner Lohnstufe, auf ganze Euro gerundet
- plus Tagespauschale (Standard 50 Euro) für jeden Tag mit mindestens einem abgelieferten Auftrag

Das Geld geht aus der Firmenkasse auf das Lohnkonto. Reicht die Kasse nicht, bekommt jeder denselben Anteil, der Rest wird offene Lohnforderung. Offene Forderungen werden bei der nächsten Abrechnung zuerst bedient.

Jeder Tag wird genau einmal abgerechnet, auch nach einem Neustart: Ein eindeutiger Index auf Firma und Tag verhindert die zweite, und die Abrechnung läuft in einer Transaktion. War der Server aus, holt er verpasste Tage der Reihe nach nach. Der Server sieht jede Minute nach, ob ein Tag fällig ist. Eine Abrechnung von Hand rechnet alles seit der letzten ab, die nächste tägliche macht dort weiter und zahlt die Pauschale für denselben Tag nicht zweimal.

Lohnstufen: Ein Fahrer hat die letzte Stufe in der Liste, bei der er beide Bedingungen erfüllt, ein Mindestlevel im Spiel und Mindestkilometer für die Firma. Standard:

| Stufe | ab Level | ab Firmen-km | Cent je km |
|---|---|---|---|
| 1 | 0 | 0 | 40 |
| 2 | 5 | 5.000 | 50 |
| 3 | 10 | 20.000 | 60 |
| 4 | 20 | 50.000 | 70 |
| 5 | 30 | 100.000 | 80 |

Das Level rechnet der Server aus den Erfahrungspunkten, die der Launcher beim Geldabgleich mitschickt (`src/levels.rs`, Quelle in `docs/save-format.md`). Ohne Abgleich gilt Level 0.

## Geldabgleich

Im Spiel hat der Fahrer sein Lohnkonto plus den Betriebsvorschuss (Standard 2000 Euro). Davon zahlt er im Spiel Tanken, Reparaturen und was sonst anfällt. Beim Abgleich meldet der Launcher den Kontostand aus dem neuesten Spielstand und dessen Änderungszeit (`saved_at`), und der Server rechnet:

    erwartet = Stand nach dem letzten Abgleich + Summe game_amount im Fenster

Das Fenster reicht vom letzten Abgleich bis `saved_at`: Es zählen die Ereignisse dieses Fahrers, deren `occurred_at` dazwischen liegt. Wo der letzte Abgleich beginnt, steht unten. Ereignisse ohne `occurred_at` zählen im ersten Abgleich, der nach ihnen beim Server ankommt.

- Fehlt Geld, bucht die Firma die Differenz als Betriebskosten.
- Ist mehr da, wird nichts gutgeschrieben. Es entsteht ein ungeklärter Betrag, den der Chef sieht und schließt.
- Der erste Abgleich eines Fahrers bucht nichts und setzt nur den Stand.

Die Antwort sagt in `write_money`, was in den Spielstand kommt: Lohnkonto plus Vorschuss, beim Chef plus der Rest eines offenen Anschaffungsbudgets (siehe Anschaffungen), nie unter 0. Hat der Launcher das geschrieben, meldet er es über `.../written`, mit `written_at`. Ab dann rechnet der nächste Abgleich von diesem Betrag, und sein Fenster beginnt bei `written_at`. Kam keine Meldung, etwa weil das Spiel inzwischen lief, rechnet er vom vorgefundenen Kontostand weiter, und das Fenster beginnt bei `saved_at`.

Zeitpunkte: `occurred_at`, `saved_at` und `written_at` kommen alle von der Uhr des Fahrer-PCs. Liegt einer vor 2020 oder mehr als einen Tag nach der Serveruhr, wird er nicht abgelehnt, sondern behandelt, als fehlte er. Fehlt `saved_at` oder `written_at`, gilt der Empfang beim Server. `saved_at` gilt nie früher als der Beginn des Fensters, damit sich Fenster nicht überlappen, wenn Abgleiche mit verschiedenen Uhren kommen (Launcher und Dashboard). Fehlt `occurred_at`, zählt das Ereignis nach der Reihenfolge des Empfangs im nächsten Abgleich und korrigiert nie einen alten.

Spät gemeldete Ereignisse: Kommt ein Ereignis erst an, nachdem sein Fenster schon abgeglichen ist, etwa weil das Plugin den Server nicht erreichte, steckte sein Betrag schon im damaligen Kontostand. Der Server bucht es normal und rechnet den Abgleich neu, ohne alte Buchungen zu ändern:

- Waren dadurch zu viel Betriebskosten gebucht, gibt es eine Gutschrift `betriebskosten_korrektur`. Waren es zu wenig, etwa weil ein später Auftrag Tanken verdeckt hat, weitere `betriebskosten`.
- Ein offener ungeklärter Betrag wird kleiner oder geschlossen (ohne Namen, Notiz "Durch spät gemeldete Ereignisse erklärt"). Was der Chef schon geschlossen hat, bleibt geschlossen. Bleibt mehr übrig, als er hingenommen hat, kommt ein neuer offener Eintrag dazu.
- Ereignisse vor dem ersten Abgleich stecken im Anfangsstand, Ereignisse zwischen gelesenem und neu geschriebenem Spielstand zählen nirgends. Dort wird nichts korrigiert.

## Schaden und Werkstatt

Reparaturen im Spiel bezahlt der Fahrer mit Geld aus dem Spiel, das landet über den Geldabgleich als Betriebskosten bei der Firma. Gibt ein Fahrer einen Lkw beschädigt zurück, bekäme ohne Werkstatt der nächste Fahrer den Schaden mit. Deshalb rechnet der Server je Lkw einen Schaden aus und sperrt Lkw mit zu viel davon.

**Schaden.** Aus dem gespeicherten Zustand (`trucks.state`, Feldnamen von `TruckState`, Verschleiß 0 bis 1) rechnet `src/damage.rs` die gewichtete Summe der reparierbaren Verschleißwerte, in Prozent auf eine Nachkommastelle gerundet. Die Gewichtung steht dort an einer Stelle (`WEIGHTS`):

| Bauteil | Feld | Gewicht |
|---|---|---|
| Motor | `engine_wear` | 30 |
| Getriebe | `transmission_wear` | 20 |
| Kabine | `cabin_wear` | 10 |
| Fahrgestell | `chassis_wear` | 25 |
| Räder | `wheels_wear`, Mittelwert über die Achsen | 15 |

Ein ganz verschlissener Lkw hat also 100 Prozent Schaden. Fehlende Felder und Werte, die keine Zahl sind, zählen als 0, Werte außerhalb von 0 bis 1 werden begrenzt. Der dauerhafte Verschleiß (`*_unfixable`, `wheels_wear_unfixable`) zählt nicht als Schaden. Er wird mit derselben Gewichtung getrennt berechnet (`damage_unfixable`) und nur angezeigt. Der Schaden in der Fuhrparkliste ist der vom gespeicherten Zustand, also der von der letzten Rückgabe oder Reparatur, nicht der aus dem Herzschlag während einer Leihe.

**Werkstatt.** Nach jeder Rückgabe, auch wenn der Chef mit dem letzten bekannten Zustand zurücknimmt, rechnet der Server den Schaden des neuen Zustands. Liegt er über der Schadensgrenze der Firma (`damage_limit`, Standard 25 Prozent, genau auf der Grenze zählt noch nicht), steht der Lkw in der Werkstatt (`trucks.in_workshop`) und lässt sich nicht ausleihen (409). Eine geänderte Grenze gilt ab der nächsten Rückgabe, ein Lkw in der Werkstatt bleibt dort, bis er repariert ist.

**Reparatur.** `POST /api/trucks/{id}/repair`, nur der Chef, nur wenn der Lkw nicht verliehen ist und Schaden hat. Kosten = Schaden mal `repair_rate` (Standard 150 Euro je Prozentpunkt), auf ganze Euro gerundet. Reicht die Kasse nicht, ändert sich nichts. Sonst in einer Transaktion: Buchung `werkstatt` aus der Firmenkasse, im gespeicherten Zustand `engine_wear`, `transmission_wear`, `cabin_wear`, `chassis_wear` und jede Achse in `wheels_wear` auf 0 (fehlende Felder werden nicht angelegt), der dauerhafte Verschleiß bleibt. Die Revision steigt, weil sich der Zustand geändert hat, und der Lkw ist wieder frei. Der nächste Fahrer bekommt beim Ausleihen den reparierten Zustand. Kosten von 0 Euro (Satz 0 oder auf 0 gerundet) sind kostenlos: Dann wird nichts gebucht, und die Reparatur geht auch, wenn die Kasse im Minus ist.

**Haftung.** Beim Ausleihen merkt sich der Server den Schaden in `leases.damage_out`. Bei der Rückgabe ist der neue Schaden `max(0, Schaden bei Rückgabe - damage_out)` in Prozentpunkten. Der Ausleiher zahlt davon den Haftungsanteil der Firma (`liability_percent`, Standard 0):

    Haftung = runden(neuer Schaden * repair_rate * liability_percent / 100)

Gebucht wird vom Lohnkonto des Ausleihers in die Firmenkasse, beides `haftung` und in derselben Transaktion wie die Rückgabe. Das zahlt immer der Ausleiher, auch wenn der Chef den Lkw zurücknimmt. Schaden, der beim Ausleihen schon da war, zählt nicht. Bei 0 wird nichts gebucht. Das Lohnkonto darf dabei ins Minus. Leihen von vor dieser Fassung haben kein `damage_out`, dann gilt der gespeicherte Zustand, der sich während einer Leihe nicht ändert.

## Anschaffungen

Kauft der Chef im Spiel einen Lkw, bezahlt er mit Geld aus dem Spiel, und der Geldabgleich bucht den Kaufpreis als Betriebskosten aus der Firmenkasse. Im Spiel hat er aber nur Lohnkonto plus Vorschuss. Dafür gibt es das Anschaffungsbudget.

**Budget freigeben.** `POST /api/companies/{id}/budgets` mit `amount` und optional `note`, nur der Chef. Es gibt höchstens ein offenes Budget je Firma (ein zweites gibt 409), es gehört dem Chef, der es freigibt. Die Kasse muss den Betrag decken: `amount` höchstens Kassenstand minus die Restbeträge aller offenen Budgets, und weil es höchstens eins gibt, heißt das: höchstens der Kassenstand. Gebucht wird bei der Freigabe nichts, die Kasse bleibt, wie sie ist.

**Restbetrag.** So viel vom Budget ist noch nicht ausgegeben:

    verbraucht = -(Summe betriebskosten und betriebskosten_korrektur des Chefs in der Firmenkasse
                   nach der Freigabe), begrenzt auf 0 bis amount
    Rest       = amount - verbraucht

"Nach der Freigabe" heißt: Buchungen mit einer größeren id als die größte bei der Freigabe (`purchase_budgets.ledger_from`). Beim Schließen merkt sich das Budget die größte id (`ledger_to`), was danach kommt, zählt nicht mehr. Damit zählen auch Tanken und Reparaturen des Chefs nach der Freigabe als verbraucht, ebenso alle Betriebskosten, die der erste Abgleich nach der Freigabe bucht, selbst wenn das Geld im Spiel schon vorher ausgegeben wurde. Das Budget wird so nie zu groß, höchstens zu klein.

Spät gemeldete Ereignisse rechnen das Fenster eines früheren Abgleichs neu (siehe oben). Die Buchungen dazu tragen diesen Abgleich in `ledger.sync_id` und zählen nur, wenn er nach der Freigabe lag (`sync_id` größer als `purchase_budgets.sync_from`, der größte Abgleich bei der Freigabe). Dann gilt: Eine spät gemeldete Maut, Fähre oder Zugfahrt gibt Betriebskosten mit `betriebskosten_korrektur` zurück und macht den Rest wieder größer. Ein spät gemeldeter Auftrag bucht weitere `betriebskosten` und macht ihn kleiner. Betrifft die späte Meldung einen Abgleich von vor der Freigabe, ändert sie den Rest nicht: Dessen Betriebskosten waren nie vom Budget abgezogen, eine Gutschrift dafür darf also auch nichts zurückgeben.

**Geldabgleich.** Für den Chef ist `write_money = max(0, Lohnkonto + Vorschuss + Rest)`, die Antwort nennt den Rest unter `budget`. Der Rest wird nach den Betriebskosten berechnet, die dieser Abgleich gerade gebucht hat.

**Kein Doppelabzug.** Ein Beispiel mit Lohnkonto 0, Vorschuss 2000 und Budget 60000:

1. Freigabe: nichts gebucht. Der nächste Abgleich findet 2000 im Spiel, erwartet 2000, bucht nichts und schreibt 62000.
2. Der Chef kauft im Spiel für 45000. Der nächste Abgleich rechnet vom geschriebenen Stand 62000, findet 17000 und bucht 45000 Betriebskosten, einmal. Der Rest ist jetzt 15000, geschrieben werden 2000 + 15000 = 17000, also genau, was im Spiel steht.
3. Ohne weitere Ausgaben findet der nächste Abgleich 17000, erwartet 17000 und bucht nichts. Der Kauf wird kein zweites Mal abgezogen, und der ausgegebene Teil kommt nicht erneut ins Spiel.

**Schließen.** `POST /api/companies/{id}/budgets/{bid}/close`, nur der Chef. Gebucht wird nichts. Der nächste Abgleich rechnet weiter vom geschriebenen Stand (im Beispiel 17000), findet ihn im Spiel und bucht deshalb auch keine Betriebskosten. Er schreibt nur noch Lohnkonto plus Vorschuss (2000), der Rest entfällt so im Spiel. Danach lässt sich ein neues Budget freigeben.

**Lkw-Kauf verknüpfen.** Übernimmt der Chef den gekauften Lkw im Launcher in den Fuhrpark, bietet der Launcher an, die Betriebskosten-Buchung des Kaufs mit dem Lkw zu verknüpfen (`GET .../purchase-candidates`, `POST .../ledger/{entry}/link-truck`). Im Dashboard geht das im Kassenbuch an jeder unverknüpften Betriebskosten-Buchung. Die Verknüpfung liegt in `purchase_links`, die Buchung selbst bleibt unverändert (die Kasse ist ein Journal). Im Kassenbuch steht dann "Lkw-Kauf SPED 004" mit dem aktuellen Kennzeichen. Hat ein Abgleich Kauf und Tanken zusammen gebucht, steht beides in derselben Buchung. Eine Verknüpfung lässt sich wieder lösen.

**Verkaufen und ausmustern.** Nur der Chef, nur für Lkw, die nicht verliehen sind (sonst 409). Ausmustern (`POST /api/trucks/{id}/retire`) bucht nichts. Verkaufen (`POST /api/trucks/{id}/sell` mit `price`) bucht den Erlös in derselben Transaktion als `lkw_verkauf` in die Firmenkasse, bei 0 Euro nichts. Danach steht in `trucks` `retired_at`, `retired_reason` (`ausgemustert` oder `verkauft`) und `sale_price`. Solche Lkw:

- lassen sich nicht mehr ausleihen und nicht reparieren (409), und nicht noch einmal ausmustern oder verkaufen
- stehen weiter in der Fuhrparkliste, hinter den aktiven, mit `retired`. Das Dashboard zeigt sie eingeklappt unter "Ausgemustert und verkauft", der Launcher ebenso, ohne Knöpfe
- behalten ihre Geschichte: Buchungen, Notizen und alte Kennzeichen bleiben
- zählen beim Herzschlag nicht als Firmen-Lkw. Fährt jemand einen verkauften Lkw, der noch in seinem Spielstand steht, ist er nicht als fremd gefahren markiert

**Kennzeichen.** Kennzeichen bleiben je Firma eindeutig, auch über ausgemusterte Lkw. Ein neuer Lkw darf das Kennzeichen eines ausgemusterten oder verkauften trotzdem bekommen, beim Anlegen und bei der Rückgabe mit neuem Kennzeichen vom Spiel: Das Kennzeichen wandert beim alten Lkw zu `old_plates`, und der alte heißt danach `SPED 001 (Lkw 7)` mit seiner id. Eine Rückgabe scheitert so nie an einem Lkw, den es nicht mehr gibt. Trägt ein aktiver Lkw das Kennzeichen, bleibt es bei 409.

## Was noch fehlt

- Firma gründen und beitreten im Launcher, bisher nur im Dashboard
- Im Spiel ungeprüft ist, ob die Werte aus dem Herzschlag stimmen und ob das Spiel einen geschriebenen Kontostand übernimmt, auch mit Anschaffungsbudget. Die Liste steht in `docs/architektur.md`, Abschnitt "Offene Tests im Spiel und auf Windows"

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Start |
| `src/config.rs` | Kommandozeile und Umgebung: Datenbank, Adresse, `--local`, `--version` |
| `src/version.rs` | Versionsprüfung mit Launcher und Plugin |
| `src/api.rs` | Alle Endpunkte, Buchungsregeln und Geldabgleich |
| `src/presence.rs` | Herzschlag, Online, Kennzeichen vergleichen, letzter bekannter Zustand |
| `src/damage.rs` | Schaden aus dem Zustand, Gewichtung, Kosten der Werkstatt, Haftung |
| `src/reconcile.rs` | Abgleichfenster, Zeitpunkte und Korrektur bei spät gemeldeten Ereignissen |
| `src/purchases.rs` | Anschaffungsbudget: Restbetrag und Verbrauch |
| `src/payroll.rs` | Tägliche Lohnabrechnung |
| `src/levels.rs` | Level aus Erfahrungspunkten, Lohnstufen |
| `src/clock.rs` | Uhr, in Tests stellbar, und Kalendertage in Europe/Berlin |
| `src/db.rs` | Datenbankschema und Erweiterung alter Datenbanken |
| `src/error.rs` | Fehlerantworten |
| `web/index.html` | Das Dashboard, eine Datei ohne Build-Schritt |
| `tests/flow.rs` | Automatische Tests für ganze Abläufe, mit gestellter Uhr |
| `tests/config.rs` | Wahl von Datenbank und Adresse |
