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
- die Lohnkonten der Fahrer und das Kassenbuch sehen
- als Chef: Lkw anlegen, den Lohnanteil ändern, Geld zwischen Kasse und Fahrern überweisen
- Ereignisse von Hand buchen, zum Beispiel zum Ausprobieren ohne Spiel. Im Spiel meldet sie das Plugin aus `crates/spedhub-plugin`

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
$f = Invoke-RestMethod -Method Post -Uri "$b/api/companies" -Headers $h -ContentType $j -Body (@{ name = "Nordfracht Logistik"; wage_percent = 30 } | ConvertTo-Json)
$f

# Lkw anlegen und ausleihen
$t = Invoke-RestMethod -Method Post -Uri "$b/api/companies/$($f.id)/trucks" -Headers $h -ContentType $j -Body (@{ plate = "SPED 001"; model = "scania.s_2016"; odometer_km = 3865 } | ConvertTo-Json)
Invoke-RestMethod -Method Post -Uri "$b/api/trucks/$($t.id)/checkout" -Headers $h

# Auftrag abliefern: 1000 Euro Erlös
Invoke-RestMethod -Method Post -Uri "$b/api/companies/$($f.id)/events" -Headers $h -ContentType $j -Body (@{ event_id = "job-1"; type = "job_delivered"; amount = 1000 } | ConvertTo-Json)

# Lkw zurückgeben und Firma ansehen
Invoke-RestMethod -Method Post -Uri "$b/api/trucks/$($t.id)/checkin" -Headers $h -ContentType $j -Body (@{ odometer_km = 4200 } | ConvertTo-Json)
Invoke-RestMethod -Uri "$b/api/companies/$($f.id)" -Headers $h | ConvertTo-Json -Depth 5
```

Am Ende sollte die Firmenkasse 700 Euro zeigen und dein Lohnkonto 300 Euro.

## Schnittstelle

Alle Anfragen und Antworten sind JSON. Außer bei `register`, `login` und `health` gehört der Header `Authorization: Bearer <token>` dazu. Fehler kommen als `{ "error": "..." }`.

| Methode und Pfad | Wer | Was |
|---|---|---|
| `GET /api/health` | jeder | Lebenszeichen |
| `POST /api/register` | jeder | Account anlegen: `name`, `password` (mindestens 8 Zeichen). Liefert `token` |
| `POST /api/login` | jeder | Anmelden, liefert einen neuen `token` |
| `POST /api/logout` | angemeldet | Abmelden, der benutzte Token wird ungültig |
| `GET /api/me` | angemeldet | Eigener Account und Firmen |
| `POST /api/companies` | angemeldet | Firma gründen: `name`, optional `wage_percent`. Wer gründet, ist Chef |
| `POST /api/companies/join` | angemeldet | Beitreten mit `invite_code`, Rolle Fahrer |
| `GET /api/companies/{id}` | Mitglied | Kassenstand, Mitglieder mit Lohnkonto. Den Einladungscode sieht nur der Chef |
| `PATCH /api/companies/{id}` | Chef | Lohnanteil ändern: `wage_percent` |
| `GET /api/companies/{id}/trucks` | Mitglied | Fuhrpark mit Info, wer was gerade fährt |
| `POST /api/companies/{id}/trucks` | Chef | Lkw anlegen: `plate`, `model`, optional `package`, `odometer_km`, `state` |
| `POST /api/trucks/{id}/checkout` | Mitglied | Ausleihen. Liefert Bauplan und Zustand. 409, wenn schon verliehen |
| `POST /api/trucks/{id}/checkin` | Ausleiher oder Chef | Zurückgeben: `odometer_km`, optional `state` |
| `POST /api/companies/{id}/events` | Mitglied | Ereignis aus dem Spiel buchen: `event_id`, `type`, `amount`, optional `note` |
| `GET /api/companies/{id}/ledger` | Mitglied | Die letzten Buchungen, `?limit=50` |
| `POST /api/companies/{id}/transfer` | Chef | Geld verschieben: `user_id`, `amount`, optional `note`. Positiv geht aus der Kasse an den Fahrer, negativ zurück |

## Buchungsregeln

Beträge sind ganze Euro. `amount` ist immer positiv, das Vorzeichen ergibt sich aus dem Typ.

| `type` | Fahrer | Firma |
|---|---|---|
| `job_delivered` | plus Lohnanteil | plus Rest |
| `job_cancelled` | minus Betrag | |
| `fined` | minus Betrag | |
| `tollgate`, `ferry`, `train` | | minus Betrag |

Die Typen entsprechen den Gameplay-Events aus dem Telemetrie-SDK. Jede `event_id` wird pro Firma nur einmal gebucht. Schickt der Client ein Ereignis doppelt, kommt `duplicate: true` zurück und es passiert nichts.

Bei einer Überweisung kann die Kasse nicht ins Minus gehen. Ein Lohnkonto schon, das sind dann Schulden beim Chef.

Die Kasse ist ein Journal. Kassenstand und Lohnkonten sind immer die Summe der Buchungen, es gibt keine einzelne Zahl, die man überschreiben könnte.

## Was noch fehlt

- Geldabgleich mit dem Spielstand (Tanken, Reparaturen) und der Betriebsvorschuss
- Lkw-Kauf für die Firma
- Online-Status der Fahrer
- Ablauf von Tokens, Passwort ändern
- Firma gründen und beitreten im Launcher, bisher nur im Dashboard. Launcher, Save-Bibliothek und Spiel-Plugin gibt es in einer ersten Fassung

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Start, Einstellungen |
| `src/api.rs` | Alle Endpunkte und die Buchungsregeln |
| `src/db.rs` | Datenbankschema |
| `src/error.rs` | Fehlerantworten |
| `web/index.html` | Das Dashboard, eine Datei ohne Build-Schritt |
| `tests/flow.rs` | Automatischer Test für den ganzen Ablauf |
