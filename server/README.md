# SpedHub-Server

Das Backend von SpedHub: Accounts, Firmenkasse und Fuhrpark. Stand v0.1, gedacht zum lokalen Ausprobieren.

Der Server ist die Wahrheit. Was im Spielstand steht, ist nur eine Kopie, die der Launcher später bei jedem Abgleich neu aufbaut. Mehr dazu in `docs/architektur.md`.

## Starten

Voraussetzung ist ein aktuelles Rust mit dem MSVC-Toolchain (wegen SQLite braucht es den C-Compiler aus den Visual Studio Build Tools).

```powershell
cd C:\dev\SpedHub
rustup update
cargo test --workspace
cargo run --release -p spedhub-server
```

Danach läuft der Server auf http://127.0.0.1:8787 und legt im aktuellen Ordner die Datei `spedhub.db` an. Beenden mit Strg+C.

## Dashboard

Unter http://127.0.0.1:8787 liegt die Weboberfläche. Dort kannst du:

- ein Konto anlegen und dich anmelden
- eine Firma gründen oder mit einem Einladungscode beitreten
- den Fuhrpark sehen, Lkw ausleihen und zurückgeben
- bei den Fahrern Level, Firmenkilometer, Lohnstufe, Lohnkonto und offenen Lohn sehen
- im Kassenbuch alle Buchungen sehen, die Lohnabrechnungen als eine Zeile je Abrechnung zum Aufklappen
- als Chef: Lkw anlegen, Tagespauschale, Uhrzeit der Abrechnung, Betriebsvorschuss, Erlösfaktor und Lohnstufen einstellen, Geld zwischen Kasse und Fahrern überweisen, ungeklärte Beträge aus dem Geldabgleich sehen und schließen
- im Testbereich "Ereignis von Hand buchen" Ereignisse mit Strecke buchen, einen Geldabgleich durchspielen und die Lohnabrechnung sofort auslösen. Im Spiel melden die Ereignisse das Plugin aus `crates/spedhub-plugin` und den Abgleich der Launcher

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
| `GET /api/health` | jeder | Lebenszeichen |
| `POST /api/register` | jeder | Account anlegen: `name`, `password` (mindestens 8 Zeichen). Liefert `token` |
| `POST /api/login` | jeder | Anmelden, liefert einen neuen `token` |
| `POST /api/logout` | angemeldet | Abmelden, der benutzte Token wird ungültig |
| `GET /api/me` | angemeldet | Eigener Account und Firmen |
| `POST /api/companies` | angemeldet | Firma gründen: `name`. Wer gründet, ist Chef. Die Firma bekommt die fünf Standard-Lohnstufen |
| `POST /api/companies/join` | angemeldet | Beitreten mit `invite_code`, Rolle Fahrer |
| `GET /api/companies/{id}` | Mitglied | Kassenstand, Einstellungen, Lohnstufen, Mitglieder mit Lohnkonto, offenem Lohn, Erfahrung, Level, Firmenkilometern und Lohnstufe. Einladungscode und Zahl der offenen ungeklärten Beträge sieht nur der Chef |
| `PATCH /api/companies/{id}` | Chef | Einstellungen ändern, alle optional: `revenue_factor`, `daily_flat`, `payroll_time` (wie `04:00`), `advance` |
| `PUT /api/companies/{id}/wage-levels` | Chef | Lohnstufen ersetzen: `levels`, eine Liste aus `min_level`, `min_km`, `cents_per_km`. 1 bis 20 Stufen |
| `GET /api/companies/{id}/trucks` | Mitglied | Fuhrpark mit Info, wer was gerade fährt, und `old_plates` (frühere Kennzeichen, älteste zuerst) |
| `POST /api/companies/{id}/trucks` | Chef | Lkw anlegen: `plate`, `model`, optional `package`, `odometer_km`, `state` |
| `POST /api/trucks/{id}/checkout` | Mitglied | Ausleihen. Liefert Bauplan und Zustand. 409, wenn schon verliehen |
| `GET /api/trucks/{id}/lease` | Ausleiher oder Chef | Die laufende Leihe noch einmal: `lease_id`, `user_id`, `taken_at` und `truck` wie beim Ausleihen, mit Bauplan, Zustand und Kennzeichen. 403 für alle anderen, 409, wenn der Lkw nicht verliehen ist |
| `POST /api/trucks/{id}/checkin` | Ausleiher oder Chef | Zurückgeben: `odometer_km`, optional `state` und `plate`. Mit `plate` bekommt der Lkw ein neues Kennzeichen, das alte kommt in `old_plates`. 409, wenn ein anderer Lkw der Firma das Kennzeichen gerade hat |
| `POST /api/companies/{id}/events` | Mitglied | Ereignis aus dem Spiel buchen: `event_id`, `type`, `amount`, optional `distance_km` und `note` |
| `GET /api/companies/{id}/ledger` | Mitglied | Die letzten Buchungen, `?limit=50`. Jede mit `game_amount` (Betrag im Spiel), `distance_km` und `payroll_id` |
| `POST /api/companies/{id}/transfer` | Chef | Geld verschieben: `user_id`, `amount`, optional `note`. Positiv geht aus der Kasse an den Fahrer, negativ zurück |
| `GET /api/companies/{id}/payrolls` | Mitglied | Die letzten Lohnabrechnungen mit einer Zeile je Fahrer, `?limit=30` |
| `POST /api/companies/{id}/payrolls/run` | Chef | Lohn sofort abrechnen: erst fällige Tage, dann alles seit der letzten Abrechnung |
| `POST /api/companies/{id}/sync` | Mitglied | Geldabgleich: `ingame_money`, optional `xp`. Liefert unter anderem `sync_id`, `expected`, `booked`, `unexplained` und `write_money` |
| `POST /api/companies/{id}/sync/{sync_id}/written` | wer abgeglichen hat | Der Betrag aus dem Abgleich steht jetzt im Spielstand. Nur für den neuesten eigenen Abgleich |
| `GET /api/companies/{id}/unexplained` | Chef | Offene ungeklärte Beträge, mit `?all=true` auch die geschlossenen |
| `POST /api/companies/{id}/unexplained/{eid}/close` | Chef | Ungeklärten Betrag schließen, optional mit `note`. Gebucht wird nichts |

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

Jede Buchung merkt sich in `game_amount` den vollen Betrag aus dem Spiel, mit dem Vorzeichen, mit dem er den Kontostand im Spiel verändert hat. Der Erlösfaktor wirkt nur auf die Firmenkasse. Bei `job_delivered` zählt `distance_km` als Firmenkilometer des Fahrers.

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

Im Spiel hat der Fahrer sein Lohnkonto plus den Betriebsvorschuss (Standard 2000 Euro). Davon zahlt er im Spiel Tanken, Reparaturen und was sonst anfällt. Beim Abgleich meldet der Launcher den Kontostand aus dem neuesten Spielstand, und der Server rechnet:

    erwartet = Stand nach dem letzten Abgleich + Summe game_amount seitdem

- Fehlt Geld, bucht die Firma die Differenz als Betriebskosten.
- Ist mehr da, wird nichts gutgeschrieben. Es entsteht ein ungeklärter Betrag, den der Chef sieht und schließt.
- Der erste Abgleich eines Fahrers bucht nichts und setzt nur den Stand.

Die Antwort sagt in `write_money`, was in den Spielstand kommt: Lohnkonto plus Vorschuss, nie unter 0. Hat der Launcher das geschrieben, meldet er es über `.../written`. Ab dann rechnet der nächste Abgleich von diesem Betrag. Kam keine Meldung, etwa weil das Spiel inzwischen lief, rechnet er vom vorgefundenen Kontostand weiter.

## Was noch fehlt

- Lkw-Kauf für die Firma
- Online-Status der Fahrer
- Ablauf von Tokens, Passwort ändern
- Firma gründen und beitreten im Launcher, bisher nur im Dashboard. Launcher, Save-Bibliothek und Spiel-Plugin gibt es in einer ersten Fassung

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Start, Einstellungen |
| `src/api.rs` | Alle Endpunkte, Buchungsregeln und Geldabgleich |
| `src/payroll.rs` | Tägliche Lohnabrechnung |
| `src/levels.rs` | Level aus Erfahrungspunkten, Lohnstufen |
| `src/clock.rs` | Uhr, in Tests stellbar, und Kalendertage in Europe/Berlin |
| `src/db.rs` | Datenbankschema und Erweiterung alter Datenbanken |
| `src/error.rs` | Fehlerantworten |
| `web/index.html` | Das Dashboard, eine Datei ohne Build-Schritt |
| `tests/flow.rs` | Automatische Tests für ganze Abläufe, mit gestellter Uhr |
