# Architektur

Stand November 2026, ETS2 1.62.

## Ziel

Eine virtuelle Spedition, die sich wie eine echte anfühlt:

- Die Fahrer fahren gemeinsam im Convoy und sehen sich auf der Straße.
- Einnahmen aus Aufträgen gehen ganz in die Firmenkasse. Die Fahrer bekommen einen Tageslohn nach Kilometern und Lohnstufe, dazu eine Tagespauschale.
- Das Geld im Spiel ist das Lohnkonto des Fahrers plus ein Betriebsvorschuss der Firma und wird regelmäßig mit dem Server abgeglichen.
- Wer keinen eigenen Lkw hat, leiht sich einen aus dem Fuhrpark. Der Lkw landet wirklich im Spielstand des Fahrers.
- Der Einstieg läuft über einen Launcher: starten, anmelden, Lkw wählen, losfahren.

Nicht Teil des Projekts: ein eigener Multiplayer, Eingriffe in den Arbeitsspeicher des Spiels, TruckersMP.

## Grundregeln

1. Der Server ist die Wahrheit. Der Spielstand enthält nur Kopien, die bei jedem Abgleich neu aufgebaut werden. Damit schaden zurückgespielte Backups, das Zurücksetzen am Convoy-Ende und doppelte Lkw nicht.
2. Ins Spiel geschrieben wird nur über den Spielstand.
3. Jeder Schreibvorgang: Backup vorher, Prüfung nachher, Rollback bei Fehler.
4. Versionssperre: Geschrieben wird nur in Spielstände von Spielversionen, die getestet sind. SCS hat das Schema mit 1.60 ohne Ankündigung geändert.
5. Jeder Fahrer hat ein eigenes Firmenprofil, das der Launcher verwaltet. Steam Cloud ist dafür aus.

## Bausteine

| Baustein | Aufgabe | Stand |
|---|---|---|
| Server (`server/`) | Accounts, Firma, Kasse, Fuhrpark, Leihen, tägliche Lohnabrechnung, Geldabgleich | läuft |
| Dashboard (`server/web/`) | Weboberfläche für Chef und Fahrer | läuft |
| Save-Bibliothek (`crates/spedhub-save`) | Spielstand lesen und schreiben, Lkw exportieren, einsetzen, entfernen, Kontostand setzen | erste Fassung, im Spiel noch nicht geprüft |
| Launcher (`crates/spedhub-launcher`) | Anmelden, Abgleich, Spielstand vorbereiten, Spiel starten, Abgleich beim Beenden | erste Fassung: Anmelden, Fuhrpark, Ausleihen und Zurückgeben, Lkw in den Spielstand schreiben, Geldabgleich, `plugin.json`, Spiel starten. Aufräumen des Profils: offen |
| Spiel-Plugin (`crates/spedhub-plugin`) | Telemetrie-Ereignisse melden, Quicksave und Quickload auslösen (Telemetry SDK und Input SDK) | Gameplay-Events: erste Fassung, im Spiel noch nicht geprüft. Quicksave und Quickload: offen |
| Overlay | Kasse und Fuhrpark im Spiel | offen |
| Convoy Dedicated Server | Die gemeinsame Session, offizieller Server von SCS | offen |

```
Spiel (ETS2)  <->  Plugin  <->  Launcher  <->  Server
     |                             |
  Spielstand  <-- Save-Bibliothek -+
     |
Convoy Dedicated Server (nur gemeinsames Fahren, kennt weder Geld noch Fuhrpark)
```

In der ersten Fassung schickt das Plugin Gameplay-Events direkt an den Server, über `POST /api/companies/{id}/events`. Server, Token und Firma liest es aus `%LOCALAPPDATA%\SpedHub\plugin.json`, die später der Launcher schreibt. Ist der Server nicht erreichbar, sammelt das Plugin die Events in einem Postausgang und schickt sie nach. Details in `crates/spedhub-plugin/README.md`.

## Abläufe

### Start über den Launcher

1. Fahrer meldet sich an und wählt die Firma.
2. Der Launcher holt Kontostand, geliehene Lkw und Fuhrpark vom Server.
3. Der Launcher räumt das Firmenprofil auf: Firmen-Lkw, die laut Server nicht an diesen Fahrer verliehen sind, kommen raus. Verliehene werden aus dem Serverstand neu eingesetzt. Der Kontostand wird auf den Serverwert gesetzt.
4. Der Launcher startet das Spiel. Den Convoy betritt der Fahrer selbst über den Session-Browser, ein automatischer Beitritt ist nicht möglich.

Stand der ersten Fassung: Vom Schritt 3 gibt es den Kontostand, das Aufräumen noch nicht. Der Launcher setzt einen geliehenen Lkw in den neuesten Spielstand ein, gleicht dabei das Geld ab und legt das Ergebnis als neuen Speicherplatz `SpedHub: <Kennzeichen>` an, statt einen vorhandenen zu überschreiben. Steht der Lkw schon im neuesten Spielstand, etwa nach einer früheren Leihe, bricht das Einsetzen mit einer Meldung ab. Nur den Kontostand gleicht "Geld abgleichen" ab, mit einem Speicherplatz `SpedHub: Geldabgleich`. Details in `crates/spedhub-launcher/README.md`.

Den Bauplan eines geliehenen Lkw legt der Launcher in `leases\` ab. Fehlt er dort, zum Beispiel auf einem anderen Rechner, holt er ihn über `GET /api/trucks/{id}/lease` vom Server. Das dürfen nur der Fahrer, der den Lkw hat, und der Chef.

### Lkw ausleihen

- **Vor dem Start:** Fahrer wählt im Launcher einen freien Lkw, der Server vergibt die Leihe, der Launcher setzt den Lkw in den letzten Spielstand ein. Das ist der sichere Weg und der Kern der ersten Version.
- **Über das Hauptmenü:** Fahrer verlässt die Session, wechselt den Lkw, lädt das Profil neu und tritt wieder bei. Das Spiel bleibt offen.
- **Im laufenden Spiel (später):** Quicksave, Lkw einsetzen, Quickload. Im Convoy ist Laden laut SCS instabil und braucht `g_convoy_allow_load 1`.

### Rückgabe

Beim Wechsel und beim Beenden liest der Launcher Kilometerstand, Verschleiß und Tank aus dem Spielstand und meldet sie dem Server. Der zählt die Revision des Lkw hoch und gibt ihn frei. Nach einem Absturz nimmt der Server den letzten Zustand aus der Telemetrie.

### Kasse

Das Plugin meldet Ereignisse aus dem Spiel, der Server bucht sie:

| Ereignis | Lohnkonto des Fahrers | Firmenkasse |
|---|---|---|
| Auftrag abgeliefert | | plus Erlös mal Erlösfaktor, die Kilometer zählen für den Fahrer |
| Auftrag abgebrochen, Strafe | minus Betrag | |
| Maut, Fähre, Zug | | minus Betrag |

Der Erlösfaktor ist je Firma einstellbar, Standard 1. Gedacht ist er für eine Wirtschafts-Mod, die die Auszahlung im Spiel auf 1 Prozent senkt: Mit Faktor 100 kommt in der Kasse wieder der volle Erlös an, im Spiel bekommt der Fahrer fast nichts.

Jede Buchung merkt sich neben dem gebuchten Betrag den vollen Betrag aus dem Spiel (`game_amount`), mit dem Vorzeichen, mit dem er den Kontostand im Spiel verändert hat. Alte Buchungen bleiben unverändert, die Spalte ist dort leer.

Ein Ereignis fürs Tanken gibt es im SDK nicht. Tanken und Reparaturen ergeben sich aus dem Geldabgleich.

Ersetzt: Bis November 2026 bekam der Fahrer bei jedem Auftrag einen Lohnanteil in Prozent, der Rest ging in die Kasse. Die Buchungen von damals stehen weiter im Journal.

### Lohn

Einmal am Tag (Standard 4:00, Europe/Berlin) rechnet der Server den Vortag ab: je Fahrer Kilometer aus abgelieferten Aufträgen mal Cent je km seiner Lohnstufe, plus Tagespauschale für jeden Tag mit mindestens einem Auftrag. Das Geld geht aus der Firmenkasse auf das Lohnkonto.

- Lohnstufe: die letzte in der Liste des Chefs, deren Mindestlevel im Spiel und Mindest-Firmenkilometer der Fahrer erreicht. Standard sind fünf Stufen von 40 bis 80 Cent.
- Level: rechnet der Server aus den Erfahrungspunkten, die der Launcher beim Geldabgleich aus `info.sii` liest. Formel und Quelle in `docs/save-format.md`.
- Reicht die Kasse nicht, bekommt jeder denselben Anteil. Der Rest wird offene Lohnforderung und bei der nächsten Abrechnung zuerst bedient.
- Jeder Tag läuft genau einmal, auch nach einem Neustart. Verpasste Tage holt der Server nach.

Abrechnungen, Zeilen je Fahrer, offene Forderungen und bezahlte Pauschaltage liegen in eigenen Tabellen (`payroll_runs`, `payroll_lines`, `wage_claims`, `flat_days`). Forderungen sind wie die Kasse ein Journal.

### Geldabgleich

Das Konto im Spiel ist nur eine Anzeige: Lohnkonto plus Betriebsvorschuss (je Firma, Standard 2000 Euro). Vom Vorschuss zahlt der Fahrer im Spiel Tanken und Reparaturen. Bei jedem Abgleich:

1. Der Launcher liest Kontostand, Erfahrungspunkte und die Änderungszeit von `game.sii` aus dem neuesten Spielstand und schickt sie an `POST /api/companies/{id}/sync`.
2. Erwarteter Stand = Stand nach dem letzten Abgleich plus `game_amount` aller Ereignisse dieses Fahrers im Fenster des Abgleichs (siehe unten). Der Erlösfaktor zählt hier nicht, im Spiel kam ja der gemeldete Betrag an.
3. Fehlt Geld, bucht die Firma die Differenz als Betriebskosten.
4. Ist mehr Geld da als erklärbar, wird nichts gutgeschrieben. Es entsteht ein ungeklärter Betrag, den der Chef im Dashboard sieht und schließt.
5. Der Server antwortet mit dem neuen Kontostand: Lohnkonto plus Vorschuss, nie unter 0. Der Launcher schreibt ihn in `bank.money_account` und `info_money_account` eines neuen Speicherplatzes, abgesichert wie beim Einsetzen, und meldet danach, dass er geschrieben hat.

Der erste Abgleich eines Fahrers bucht nichts und setzt nur den Stand. Strafen und Abbrüche zahlt der Fahrer aus dem Lohnkonto, Maut, Fähre und Zug die Firma.

Die Abgleichpunkte liegen in `sync_points`: was im Spielstand stand, was geschrieben werden sollte und ob der Launcher das bestätigt hat. Ohne Bestätigung, etwa weil das Spiel zwischendurch gestartet wurde, rechnet der nächste Abgleich vom vorgefundenen Stand.

Welche Ereignisse zu welchem Abgleich gehören, entscheidet der Zeitpunkt, zu dem sie im Spiel passiert sind, nicht wann sie beim Server ankommen. Alle Zeitpunkte kommen dafür von derselben Uhr, der des Fahrer-PCs:

- `occurred_at` eines Ereignisses: Systemzeit, zu der das Plugin es angenommen hat. Sie bleibt im Postausgang erhalten.
- `cutoff` eines Abgleichs: Änderungszeit von `game.sii` des gelesenen Spielstands. Der Kontostand darin enthält genau die Ereignisse bis hier.
- `written_at`: wann der Launcher den neuen Spielstand geschrieben hat, mit der Bestätigung gemeldet.

Fehlt `cutoff` oder `written_at` (älterer Launcher, Dashboard) oder ist er unplausibel, gilt der Empfang beim Server. Ein `cutoff` liegt nie vor dem Beginn seines Fensters, so überlappen sich Fenster auch dann nicht, wenn Abgleiche mit verschiedenen Uhren kommen. Jeder Abgleich k beginnt das nächste Fenster bei `written_at`, wenn bestätigt, sonst bei `cutoff`. Das Fenster von Abgleich k reicht von dort bis `cutoff` von k. Zwischen `cutoff` und `written_at` war das Spiel zu, Ereignisse mit Zeitpunkt dort zählen in keinem Fenster.

Kommt ein Ereignis an, dessen Zeitpunkt in einem schon abgeglichenen Fenster liegt, steckte sein Spielbetrag schon im damaligen Kontostand. Der Server bucht es wie jedes andere und korrigiert dazu diesen Abgleich: Er rechnet die Differenz neu und bucht nur den Unterschied, als `betriebskosten_korrektur` (Gutschrift) oder weitere `betriebskosten`. Ein offener ungeklärter Betrag wird kleiner oder geschlossen. Alte Buchungen bleiben unverändert. Ereignisse vor dem ersten Abgleich stecken im Anfangsstand und ändern nichts. Je Abgleich merkt sich der Server dafür die aktuelle Differenz, die gebuchten Betriebskosten und den ungeklärten Betrag. Die Lohnabrechnung zählt weiter nach dem Empfang beim Server, damit abgeschlossene Abrechnungen bleiben, wie sie sind.

Ein Ereignis ohne plausiblen eigenen Zeitpunkt (älteres Plugin, Dashboard) zählt nach dem Empfang: im ersten Abgleich, der nach ihm beim Server ankommt. Der Server vergleicht dafür nur die Reihenfolge des Empfangs, nie seine Uhr mit der des Fahrer-PCs, und ein solches Ereignis korrigiert nie einen alten Abgleich.

## Das Lkw-Paket

So liegt ein Firmen-Lkw auf dem Server (`trucks.package` und `trucks.state`):

- **Bauplan:** die `vehicle`-Unit und alle Zubehör-Units als Text. Beim Einsetzen bekommen sie neue, eindeutige Unit-Namen.
- **Zustand:** Kilometerstand, Wartungs-Kilometer, Verschleiß je Bauteil, Tank.
- **Kennzeichen:** eindeutig je Lkw und Firma, aber nicht dauerhaft: Das Spiel kann es ersetzen (im Spiel bestätigt). Der Launcher meldet das neue Kennzeichen bei der Rückgabe, der Server behält die alten in `old_plates`. So lässt sich auch Telemetrie mit einem alten Kennzeichen noch zuordnen. Im Spielstand erkennt SpedHub den Lkw über Modell und Teile wieder, nicht über das Kennzeichen.
- **Abhängigkeiten:** alle `data_path`-Einträge. Daraus ergibt sich, welche DLCs und Mods ein Fahrer braucht.
- **Revision:** steigt bei jeder Rückgabe.

## Reihenfolge

1. Tests von Hand im Spiel (siehe unten)
2. Save-Bibliothek in Rust
3. Launcher mit Leihe vor dem Start, Plugin für Telemetrie, Geldabgleich
4. Lkw-Wechsel im laufenden Spiel, Overlay
5. Convoy-Server, Lkw-Kauf für die Firma, Wirtschafts-Mod
6. Härten: Versionssperre, Backups, Mod-Lkw

## Offene Tests im Spiel

| Test | Frage |
|---|---|
| Lkw verpflanzen | Lädt 1.62 einen Spielstand mit eingesetztem fremdem Lkw fehlerfrei? Stimmen Kilometer, Schaden und Tuning? Wo steht der Lkw? |
| Save-Cache | Übernimmt das laufende Spiel einen geänderten Spielstand zuverlässig, auch beim zweiten Mal? Seit 1.50 gibt es einen Datei-Cache, der alte Versionen liefern kann |
| Kontostand | Übernimmt das Spiel den beim Geldabgleich geschriebenen Kontostand? |
| Level | Passt das aus `info_players_experience` berechnete Level zu dem, das das Spiel anzeigt? |
| Lkw entfernen | Lädt ein Spielstand, dessen aktiver Lkw ersatzlos entfernt wurde? |
| Laden im Convoy | Bleibt die Verbindung beim Laden stehen? |
| Sicht der anderen | Sehen Mitspieler nach einem Lkw-Wechsel sofort den neuen Lkw? |
| Session-Ende | Welcher Spielstand liegt nach "behalten" oder "zurücksetzen" auf der Platte? |

Solange der erste Test offen ist, sind alle Angaben zum Einsetzen vorläufig.

## Fremde Bausteine

| Projekt | Wofür | Lizenz |
|---|---|---|
| `decrypt_truck` | Spielstände entschlüsseln | MIT, direkt nutzbar |
| `scs-sdk-crates` | Telemetry und Input SDK in Rust | MIT/Apache-2.0, im Plugin als `scs-sdk-plugin` 0.1.1 genutzt |
| TS SE Tool | Aufbau der Save-Units nachlesen, Erfahrungspunkte je Level | Apache-2.0 |
| truck-tools | Schema seit 1.60 nachlesen | GPL-3.0, kein Code übernehmen |
| NMC-SCS-LAUNCHER | Logik für den Lkw-Wechsel nachlesen | ohne Lizenz, kein Code übernehmen |
