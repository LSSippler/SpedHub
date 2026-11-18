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
5. Jeder Fahrer hat ein eigenes Firmenprofil, das der Launcher verwaltet. Steam Cloud ist dafür aus. Für die Firma zählt nur, was in diesem Profil passiert.

## Bausteine

| Baustein | Aufgabe | Stand |
|---|---|---|
| Server (`server/`) | Accounts, Firma, Kasse mit Storno, Fuhrpark, Leihen, tägliche Lohnabrechnung, Geldabgleich mit spät gemeldeten Ereignissen, Herzschlag und Online-Status, Schaden und Werkstatt, Anschaffungen, Verkauf und Ausmusterung | läuft |
| Dashboard (`server/web/`) | Weboberfläche für Chef und Fahrer | läuft |
| Save-Bibliothek (`crates/spedhub-save`) | Spielstand lesen und schreiben, Lkw exportieren, wiedererkennen, einsetzen, entfernen, Kontostand setzen, Wirtschafts-Mod bauen | erste Fassung. Einsetzen in einen Spielstand ohne eigenen Lkw, neuer Speicherplatz und Kontostand im Spiel bestätigt (1.62), Einsetzen neben eigene Lkw und Entfernen noch nicht |
| Launcher (`crates/spedhub-launcher`) | Anmelden, Abgleich, Spielstand vorbereiten, Spiel starten, Abgleich beim Beenden, Plugin installieren, Updates | erste Fassung: Anmelden, Passwort ändern, Fuhrpark mit Schaden und Werkstatt, Ausleihen und Zurückgeben, Lkw übernehmen und Kauf verknüpfen, Lkw in den Spielstand schreiben, Geldabgleich, `plugin.json`, Plugin installieren, Updates, Spiel starten. Aufräumen des Profils und Abgleich beim Beenden: offen |
| Spiel-Plugin (`crates/spedhub-plugin`) | Telemetrie-Ereignisse und Herzschlag melden, Quicksave und Quickload auslösen (Telemetry SDK und Input SDK) | Gameplay-Events und Herzschlag: erste Fassung, im Spiel bestätigt (DLL lädt, Herzschlag kommt an, `job.delivered` vollständig). Meldet nur aus dem Firmenprofil, das Erkennen über `game.log.txt` ist im Spiel noch nicht geprüft. Quicksave und Quickload: offen |
| Setup (`installer/`, `.github/workflows/release.yml`) | Launcher, Plugin und optional Server installieren, Plugin ins Spiel kopieren, Releases mit Prüfsummen | erste Fassung. Die CI baut das Setup auf dem Windows-Runner, installiert ist es auf einem echten Windows noch nicht |
| Overlay | Kasse und Fuhrpark im Spiel | offen |
| Convoy Dedicated Server | Die gemeinsame Session, offizieller Server von SCS | offen |

```
Spiel (ETS2)  <->  Plugin  <->  Launcher  <->  Server
     |                             |
  Spielstand  <-- Save-Bibliothek -+
     |
Convoy Dedicated Server (nur gemeinsames Fahren, kennt weder Geld noch Fuhrpark)
```

In der ersten Fassung schickt das Plugin Gameplay-Events direkt an den Server, über `POST /api/companies/{id}/events`, und alle 10 Sekunden einen Herzschlag über `POST /api/companies/{id}/heartbeat`. Server, Token, Firma, Firmenprofil und den Pfad zu `game.log.txt` liest es aus `%LOCALAPPDATA%\SpedHub\plugin.json`, die der Launcher schreibt, sobald eine Firma gewählt ist, und neu bei jedem Wechsel von Firma oder Firmenprofil. Gemeldet wird nur aus dem Firmenprofil, siehe unten. Ist der Server nicht erreichbar, sammelt das Plugin die Events in einem Postausgang und schickt sie nach. Details in `crates/spedhub-plugin/README.md`.

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

Beim Wechsel und beim Beenden liest der Launcher Kilometerstand, Verschleiß und Tank aus dem Spielstand und meldet sie dem Server. Der zählt die Revision des Lkw hoch und gibt ihn frei.

Nach einem Absturz gibt es keinen Spielstand mit dem letzten Stand. Dafür gibt es den letzten bekannten Zustand aus dem Herzschlag (siehe unten): Der Chef nimmt den Lkw ohne Kilometerstand zurück, der Server übernimmt Kilometerstand, Verschleiß und Tank aus dem letzten Herzschlag des Ausleihers mit diesem Lkw und sagt, von wann er ist. Gibt es keinen, muss der Kilometerstand angegeben werden. Fahrer selbst geben immer einen Kilometerstand an. Jede Rückgabe leert den letzten bekannten Zustand.

### Online und Herzschlag

Das Plugin schickt alle 10 Sekunden einen Herzschlag an `POST /api/companies/{id}/heartbeat`, solange das Spiel läuft und nicht pausiert ist: Kennzeichen, Marke und Modell (`brand_id` und `id` der Konfiguration `truck`), Kilometerstand, Verschleiß, Tank, Geschwindigkeit und den laufenden Auftrag aus der Job-Konfiguration des SDK. Eine Stadt in der Nähe liefert das SDK nicht. Herzschläge gehen nie in den Postausgang, ein verlorener wird nicht nachgeschickt.

- Online ist, wer sich in den letzten 30 Sekunden gemeldet hat. Der Server hält nur den letzten Herzschlag je Fahrer und Firma (`presence`).
- Ein Herzschlag gehört zu einem Firmen-Lkw nur, wenn Kennzeichen und Modell passen. Kennzeichen können kollidieren, ein eigener Lkw des Fahrers kann wie ein Firmen-Lkw heißen. Passt nur das Kennzeichen, ist es kein Firmen-Lkw: kein letzter bekannter Zustand, keine Markierung. Ohne Modell (älteres Plugin) wird nichts zugeordnet.
- Das Kennzeichen wird ohne Markup und Land mit den aktuellen und den alten Kennzeichen des Fuhrparks verglichen, das Modell über `spedhub_save::model_key` mit dem aus dem Spielstand (klein, ohne `vehicle.`, Marke davor, wenn das Modell keinen Punkt hat). Welche Form `id` im Spiel hat, ist nicht geprüft. Passen mehrere Lkw, gewinnt der eigene geliehene, auch über ein altes Kennzeichen, das inzwischen ein anderer Lkw trägt.
- Fährt der Fahrer seinen geliehenen Firmen-Lkw, wird der Herzschlag zum letzten bekannten Zustand des Lkw (`trucks.last_*`). Nur gemeldete Werte überschreiben den Zustand vom Ausleihen, ein Kilometerstand unter dem vom Ausleihen zählt nicht.
- Fährt er einen Firmen-Lkw (Kennzeichen und Modell passen), den er nicht geliehen hat, sieht der Chef eine Markierung. Zurückgeschrieben wird nichts, der Spielstand bleibt Sache des Launchers.
- Im Fuhrpark sehen alle, ob ein verliehener Lkw gerade gefahren wird.

### Nur das Firmenprofil zählt

Ein Fahrer kann in ETS2 mehrere Profile haben, das Plugin läuft in jedem. Beim zweiten Test im Spiel kamen 68 Ereignisse aus einem privaten Profil in die Firmenkasse und aufs Lohnkonto, und der Geldabgleich des Firmenprofils hat daraus falsche Betriebskosten gebucht. Deshalb zählt nur, was im Firmenprofil passiert, an drei Stellen:

1. **Launcher:** schreibt den angezeigten Namen des gewählten Firmenprofils (`profile_name`) und den Pfad zu `Dokumente\Euro Truck Simulator 2\game.log.txt` (`game_log`) in `plugin.json` und meldet das Profil dem Server (`PUT /api/companies/{id}/profile`, gespeichert je Mitgliedschaft).
2. **Plugin:** liest aus `game.log.txt` die letzte Zeile `New profile selected: '<Name>'`, beim Annehmen jedes Ereignisses und vor jedem Herzschlag, immer nur die neuen Bytes. Nur aus dem Firmenprofil nimmt es Ereignisse an und schickt Herzschläge, beide tragen dann `profile`. Bei einem anderen oder unbekannten Profil steht nur etwas im Log, der Postausgang enthält nur Ereignisse aus dem Firmenprofil. Das Telemetry SDK nennt das Profil nicht, deshalb der Umweg über das Log. Gelesen wird nur die Datei, kein Eingriff ins Spiel.
3. **Server:** Ist ein Firmenprofil gemeldet, bucht er nur Ereignisse mit genau diesem `profile`. Andere und solche ohne lehnt er mit 422 ab, legt sie in `rejected_events` und ins Log, der Chef sieht sie im Büro. Herzschläge aus anderen Profilen verwirft er, ohne Fehlerstatus. Ohne gemeldetes Profil (ältere Launcher) bucht er wie bisher und merkt sich das Profil nur an der Buchung.

Abgelehnte Ereignisse zählen auch im Geldabgleich nicht. Verglichen wird nach dem Trimmen genau, mit Groß- und Kleinschreibung. Die 68 Ereignisse, die vor dieser Prüfung gebucht wurden, nimmt der Chef mit einem Storno zurück (siehe Kasse, Storno). Das korrigiert auch die Betriebskosten, die der Abgleich daraus gebucht hat. Details in `crates/spedhub-plugin/README.md` und `server/README.md`, jeweils "Nur das Firmenprofil".

### Anmeldung

Tokens laufen nach 30 Tagen ohne Benutzung ab. Wer sein Passwort ändert, meldet alle anderen Anmeldungen ab, die benutzte bleibt. Launcher und Plugin teilen sich einen Token und bleiben deshalb angemeldet, wenn das Passwort im Launcher geändert wird. Das Dashboard hat einen eigenen Token. Wird das Passwort dort geändert, sind Launcher und Plugin abgemeldet, auch auf demselben Rechner: Bis zur neuen Anmeldung im Launcher und zum nächsten Spielstart gehen keine Herzschläge durch, Buchungen warten im Postausgang.

### Schaden und Werkstatt

Reparaturen im Spiel zahlt der Fahrer mit Geld aus dem Spiel, das über den Geldabgleich als Betriebskosten bei der Firma landet. Gibt er einen Lkw beschädigt zurück, bekäme der nächste Fahrer den Schaden mit. Deshalb:

- **Schaden:** Der Server rechnet je Lkw aus dem gespeicherten Zustand einen Schaden in Prozent, die gewichtete Summe der reparierbaren Verschleißwerte, auf eine Nachkommastelle gerundet. Gewichtung (`server/src/damage.rs`, `WEIGHTS`): Motor 30, Getriebe 20, Kabine 10, Fahrgestell 25, Räder 15 (Mittelwert über die Achsen). Fehlende Felder zählen als 0. Der dauerhafte Verschleiß (`*_unfixable`) zählt nicht als Schaden und wird mit derselben Gewichtung getrennt angezeigt.
- **Werkstatt:** Liegt der Schaden nach einer Rückgabe über der Schadensgrenze der Firma (Standard 25 Prozent), steht der Lkw in der Werkstatt und lässt sich nicht ausleihen. Das gilt auch, wenn der Chef mit dem letzten bekannten Zustand zurücknimmt.
- **Reparatur:** Der Chef lässt einen Lkw reparieren, der nicht verliehen ist. Kosten: Schaden mal Satz je Prozentpunkt (Standard 150 Euro), aus der Firmenkasse. Reicht sie nicht, geht es nicht. Kostet die Reparatur 0 Euro, geht sie immer, auch bei einer Kasse im Minus. Danach ist der reparierbare Verschleiß im gespeicherten Zustand 0, der dauerhafte bleibt, die Revision steigt und der Lkw ist frei.
- **Haftung:** Je Firma ein Anteil in Prozent (Standard 0). Bei der Rückgabe zahlt der Ausleiher diesen Anteil des Schadens, der während seiner Leihe dazugekommen ist (Kosten der Werkstatt dafür), aus seinem Lohnkonto an die Firmenkasse. Der Schaden beim Ausleihen steht dafür in `leases.damage_out` und zählt nicht.

Einstellungen, Formeln und Buchungsarten (`werkstatt`, `haftung`) im Detail in `server/README.md`, Abschnitt "Schaden und Werkstatt".

### Kasse

Das Plugin meldet Ereignisse aus dem Spiel, der Server bucht sie:

| Ereignis | Lohnkonto des Fahrers | Firmenkasse |
|---|---|---|
| Auftrag abgeliefert | | plus Erlös mal Erlösfaktor, die Kilometer zählen für den Fahrer |
| Auftrag abgebrochen, Strafe | minus Betrag | |
| Maut, Fähre, Zug | | minus Betrag |

Der Erlösfaktor ist je Firma einstellbar, Standard 1. Gedacht ist er für eine Wirtschafts-Mod, die die Auszahlung im Spiel auf 1 Prozent senkt: Mit Faktor 100 kommt in der Kasse wieder der volle Erlös an, im Spiel bekommt der Fahrer fast nichts. Die Mod baut `spedhub-save economy-mod` aus der `economy_data.sii` des Spiels, die jeder selbst aus `def.scs` entpackt (sie gehört SCS und liegt nicht im Repo). Anleitung, auch für den Convoy-Server, in `docs/wirtschafts-mod.md`. Im Spiel ist die Mod noch nicht geprüft.

Jede Buchung merkt sich neben dem gebuchten Betrag den vollen Betrag aus dem Spiel (`game_amount`), mit dem Vorzeichen, mit dem er den Kontostand im Spiel verändert hat. Alte Buchungen bleiben unverändert, die Spalte ist dort leer.

Ein Ereignis fürs Tanken gibt es im SDK nicht. Tanken und Reparaturen ergeben sich aus dem Geldabgleich.

Ersetzt: Bis November 2026 bekam der Fahrer bei jedem Auftrag einen Lohnanteil in Prozent, der Rest ging in die Kasse. Die Buchungen von damals stehen weiter im Journal.

#### Storno

Buchungen werden nie geändert oder gelöscht. Eine falsche Buchung, etwa ein Ereignis aus einem privaten Profil, storniert der Chef im Kassenbuch des Dashboards: eine oder mehrere auswählen (mit Zeitraum und "Alle sichtbaren auswählen" auch viele auf einmal), Grund angeben, bestätigen. Das Storno ist eine neue Buchung `storno` mit umgekehrtem Betrag auf demselben Konto, mit Verweis auf das Original, Grund und Chef. Jede Buchung lässt sich nur einmal stornieren, ein Storno gar nicht, alles oder nichts je Aufruf. Kasse und Lohnkonto sind Summen und stimmen damit sofort.

- **Paare:** Überweisung und Haftung werden mit der Gegenseite storniert. Eine Lohnbuchung aus einer Lohnabrechnung bekommt eine Gegenbuchung in der Kasse, die Summe der Abrechnung in der Kasse selbst lässt sich nicht stornieren.
- **Geldabgleich:** Stornierte Ereignisse zählen nicht mehr. Lag eins in einem schon abgeglichenen Fenster, rechnet der Server das Fenster neu wie bei einem spät gemeldeten Ereignis, nur umgekehrt, und bucht zu viel gebuchte Betriebskosten als Korrektur zurück. Korrekturen selbst lassen sich nicht stornieren.
- **Betriebskosten stornieren** heißt, die Firma erlässt sie. Der Abgleichpunkt merkt sich das, damit eine spätere Korrektur sie weder doppelt zurückgibt noch wieder anlegt. Sind sie durch stornierte Ereignisse schon korrigiert, lehnt der Server das Storno ab.
- **Lohn:** Stornierte Aufträge zählen nicht für Firmenkilometer, Lohnstufe und noch nicht abgerechnete Tage. Gezahlten Lohn holt der Chef bei Bedarf mit einem Storno der Lohnbuchung zurück.
- **Anschaffungsbudget:** Storniert der Chef eigene Betriebskosten, die das offene Budget verbraucht haben, ist dieser Teil wieder frei.

Regeln, Beispiele und Schnittstelle in `server/README.md`, Abschnitt "Storno".

### Lohn

Einmal am Tag (Standard 4:00, Europe/Berlin) rechnet der Server den Vortag ab: je Fahrer Kilometer aus abgelieferten Aufträgen mal Cent je km seiner Lohnstufe, plus Tagespauschale für jeden Tag mit mindestens einem Auftrag. Das Geld geht aus der Firmenkasse auf das Lohnkonto.

- Lohnstufe: die letzte in der Liste des Chefs, deren Mindestlevel im Spiel und Mindest-Firmenkilometer der Fahrer erreicht. Standard sind fünf Stufen von 40 bis 80 Cent.
- Level: rechnet der Server aus den Erfahrungspunkten, die der Launcher beim Geldabgleich aus `info.sii` liest. Formel und Quelle in `docs/save-format.md`.
- Reicht die Kasse nicht, bekommt jeder denselben Anteil. Der Rest wird offene Lohnforderung und bei der nächsten Abrechnung zuerst bedient.
- Jeder Tag läuft genau einmal, auch nach einem Neustart. Verpasste Tage holt der Server nach.

Stornierte Aufträge zählen nicht (siehe Kasse, Storno). Abrechnungen, Zeilen je Fahrer, offene Forderungen und bezahlte Pauschaltage liegen in eigenen Tabellen (`payroll_runs`, `payroll_lines`, `wage_claims`, `flat_days`). Forderungen sind wie die Kasse ein Journal.

### Geldabgleich

Das Konto im Spiel ist nur eine Anzeige: Lohnkonto plus Betriebsvorschuss (je Firma, Standard 2000 Euro). Vom Vorschuss zahlt der Fahrer im Spiel Tanken und Reparaturen. Bei jedem Abgleich:

1. Der Launcher liest Kontostand, Erfahrungspunkte und die Änderungszeit von `game.sii` aus dem neuesten Spielstand und schickt sie an `POST /api/companies/{id}/sync`.
2. Erwarteter Stand = Stand nach dem letzten Abgleich plus `game_amount` aller Ereignisse dieses Fahrers im Fenster des Abgleichs (siehe unten). Der Erlösfaktor zählt hier nicht, im Spiel kam ja der gemeldete Betrag an.
3. Fehlt Geld, bucht die Firma die Differenz als Betriebskosten.
4. Ist mehr Geld da als erklärbar, wird nichts gutgeschrieben. Es entsteht ein ungeklärter Betrag, den der Chef im Dashboard sieht und schließt.
5. Der Server antwortet mit dem neuen Kontostand: Lohnkonto plus Vorschuss, beim Chef plus der Rest eines offenen Anschaffungsbudgets (siehe Anschaffungen), nie unter 0. Der Launcher schreibt ihn in `bank.money_account` und `info_money_account` eines neuen Speicherplatzes, abgesichert wie beim Einsetzen, und meldet danach, dass er geschrieben hat.

Der erste Abgleich eines Fahrers bucht nichts und setzt nur den Stand. Strafen und Abbrüche zahlt der Fahrer aus dem Lohnkonto, Maut, Fähre und Zug die Firma.

Die Abgleichpunkte liegen in `sync_points`: was im Spielstand stand, was geschrieben werden sollte und ob der Launcher das bestätigt hat. Ohne Bestätigung, etwa weil das Spiel zwischendurch gestartet wurde, rechnet der nächste Abgleich vom vorgefundenen Stand.

Welche Ereignisse zu welchem Abgleich gehören, entscheidet der Zeitpunkt, zu dem sie im Spiel passiert sind, nicht wann sie beim Server ankommen. Alle Zeitpunkte kommen dafür von derselben Uhr, der des Fahrer-PCs:

- `occurred_at` eines Ereignisses: Systemzeit, zu der das Plugin es angenommen hat. Sie bleibt im Postausgang erhalten.
- `cutoff` eines Abgleichs: Änderungszeit von `game.sii` des gelesenen Spielstands. Der Kontostand darin enthält genau die Ereignisse bis hier.
- `written_at`: wann der Launcher den neuen Spielstand geschrieben hat, mit der Bestätigung gemeldet.

Fehlt `cutoff` oder `written_at` (älterer Launcher, Dashboard) oder ist er unplausibel, gilt der Empfang beim Server. Ein `cutoff` liegt nie vor dem Beginn seines Fensters, so überlappen sich Fenster auch dann nicht, wenn Abgleiche mit verschiedenen Uhren kommen. Jeder Abgleich k beginnt das nächste Fenster bei `written_at`, wenn bestätigt, sonst bei `cutoff`. Das Fenster von Abgleich k reicht von dort bis `cutoff` von k. Zwischen `cutoff` und `written_at` war das Spiel zu, Ereignisse mit Zeitpunkt dort zählen in keinem Fenster.

Kommt ein Ereignis an, dessen Zeitpunkt in einem schon abgeglichenen Fenster liegt, steckte sein Spielbetrag schon im damaligen Kontostand. Der Server bucht es wie jedes andere und korrigiert dazu diesen Abgleich: Er rechnet die Differenz neu und bucht nur den Unterschied, als `betriebskosten_korrektur` (Gutschrift) oder weitere `betriebskosten`. Ein offener ungeklärter Betrag wird kleiner oder geschlossen. Alte Buchungen bleiben unverändert. Ereignisse vor dem ersten Abgleich stecken im Anfangsstand und ändern nichts. Je Abgleich merkt sich der Server dafür die aktuelle Differenz, die gebuchten Betriebskosten und den ungeklärten Betrag. Die Lohnabrechnung zählt weiter nach dem Empfang beim Server, damit abgeschlossene Abrechnungen bleiben, wie sie sind.

Storniert der Chef ein Ereignis, zählt es in keinem Fenster mehr. Lag es in einem schon abgeglichenen, korrigiert der Server diesen Abgleich genauso, nur mit umgekehrtem Vorzeichen (siehe Kasse, Storno).

Ein Ereignis ohne plausiblen eigenen Zeitpunkt (älteres Plugin, Dashboard) zählt nach dem Empfang: im ersten Abgleich, der nach ihm beim Server ankommt. Der Server vergleicht dafür nur die Reihenfolge des Empfangs, nie seine Uhr mit der des Fahrer-PCs, und ein solches Ereignis korrigiert nie einen alten Abgleich.

### Anschaffungen

Kauft der Chef im Spiel einen Lkw, bezahlt er ihn mit Geld aus dem Spiel, und der Geldabgleich bucht den Kaufpreis wie Tanken als Betriebskosten aus der Firmenkasse. Damit er dafür im Spiel genug Geld hat:

- **Anschaffungsbudget:** Der Chef gibt im Büro des Dashboards einen Betrag frei. Die Kasse muss ihn decken, gebucht wird aber nichts. Höchstens ein Budget je Firma ist offen. Beim Geldabgleich des Chefs kommt der Rest des Budgets zusätzlich zu Lohnkonto und Vorschuss in den Spielstand.
- **Rest:** Betrag minus alle Betriebskosten des Chefs, die nach der Freigabe gebucht wurden (abzüglich Korrekturen), nie unter 0. Spät gebuchte Betriebskosten und Korrekturen zählen nur, wenn ihr Abgleich nach der Freigabe lag. Auch Tanken und Reparaturen nach der Freigabe zählen. So kommt ein schon ausgegebener Teil nie ein zweites Mal ins Spiel.
- **Kein Doppelabzug:** Der nächste Abgleich rechnet vom geschriebenen Stand inklusive Budget. Was der Chef ausgibt, fehlt dort genau einmal und wird genau einmal als Betriebskosten gebucht. Danach sinkt der Rest um denselben Betrag, und der neu geschriebene Stand stimmt mit dem im Spiel überein.
- **Schließen:** bucht nichts. Der nächste Abgleich findet im Spiel den erwarteten Stand, bucht also keine Betriebskosten, und schreibt ohne den Rest.
- **Lkw-Kauf verknüpfen:** Übernimmt der Chef den gekauften Lkw im Launcher, verknüpft er die Betriebskosten-Buchung des Kaufs mit dem Lkw (auch im Kassenbuch des Dashboards). Die Buchung bleibt, wie sie ist, das Kassenbuch zeigt "Lkw-Kauf SPED 004". Die Verknüpfung liegt getrennt vom Journal in `purchase_links`.
- **Verkaufen und ausmustern:** Der Chef nimmt einen Lkw, der nicht verliehen ist, aus dem Fuhrpark. Beim Verkauf geht der Erlös als `lkw_verkauf` in die Kasse. Solche Lkw lassen sich nicht mehr ausleihen, bleiben aber mit Buchungen und alten Kennzeichen in der Liste und zählen beim Herzschlag nicht mehr als Firmen-Lkw. Ihr Kennzeichen darf ein neuer Lkw bekommen, beim alten wandert es zu den früheren Kennzeichen.

Rechenweg, Schnittstelle und Beispiel in `server/README.md`, Abschnitt "Anschaffungen".

## Installation und Updates

Eine Versionsnummer für alles: `[workspace.package] version` in der `Cargo.toml` im Wurzelordner gilt für Server, Save-Bibliothek, Plugin, Launcher und Setup.

```
Tag v0.2.1 --> GitHub Actions (release.yml)
                 Windows: Tests, Release-Build, installer/build.ps1 + Inno Setup --> SpedHub-Setup-0.2.1.exe
                 Linux:   spedhub-server, musl, statisch                      --> spedhub-server-linux-x86_64
                 Release: SHA256SUMS.txt, Notes aus git log                    --> GitHub-Release
```

- **Setup** (`installer/spedhub.iss`, Inno Setup 6, deutsch): Launcher, Plugin-DLL und `spedhub-save.exe` (für die Wirtschafts-Mod, mit `Wirtschafts-Mod.md`) nach `Programme\SpedHub`, Verknüpfungen im Startmenü und, abwählbar, auf dem Desktop. Optional, standardmäßig abgewählt, "Server für lokale Tests" mit einer Verknüpfung, die `spedhub-server.exe --local` startet. Am Ende ruft das Setup mit seinen Adminrechten `spedhub-launcher.exe --install-plugin` auf.
- **Plugin installieren**: Die Suche nach ETS2 steckt im Launcher (`src/steam.rs`), damit sie in Rust getestet ist: Steam-Ordner aus der Registry, alle Bibliotheken aus `libraryfolders.vdf`, App-ID 227300, `installdir` aus dem App-Manifest. Ziel ist `bin\win_x64\plugins`. Kopiert wird über eine temporäre Datei, eine ältere DLL wird ersetzt, bei laufendem Spiel nichts. Der Launcher zeigt "Plugin installiert / veraltet / fehlt" über die SHA-256 und startet sich für "Plugin installieren" über UAC neu. Ist das Spiel nicht gefunden, wählt er den Ordner vorher ohne Adminrechte und übergibt ihn, denn mit Adminrechten kann er unter einem anderen Konto laufen. Aus demselben Grund meldet das Setup einen von Hand gewählten Ordner über eine Datei zurück und merkt ihn als ursprünglicher Benutzer (`--report-game-dir`, `--remember-game-dir`).
- **Deinstallation**: entfernt Programm und Plugin (`--uninstall-plugin`, still mit `--quiet`), das Plugin zuerst, solange der Launcher noch da ist. Läuft das Spiel, fragt der Launcher mit Wiederholen und Abbrechen. Bei Abbrechen und bei einer stillen Deinstallation wird die DLL umbenannt, sodass das Spiel sie nicht mehr lädt, und zum Löschen beim nächsten Neustart von Windows vorgemerkt. Reste räumt die nächste Installation weg. "Anmeldung, Einstellungen und Backups behalten?" steht auf Ja, mit Nein wird `%LOCALAPPDATA%\SpedHub` des deinstallierenden Benutzers gelöscht.
- **Updates**: Der Launcher fragt beim Start die öffentliche API der GitHub-Releases ab. Bei einer neueren Version zeigt er einen Hinweis. Erst auf Knopfdruck lädt er Setup und `SHA256SUMS.txt`, prüft die SHA-256 und startet das Setup nur, wenn sie passt. Kein stilles Update.
- **Versionsprüfung**: Launcher und Plugin schicken `X-SpedHub-Version`. Beim Anmelden antwortet der Server mit `server_version` und `compatible`. Gleiche Haupt- und Nebenversion passen zusammen. Sonst warnen Launcher (gelber Hinweis) und Server (Log), gesperrt wird nichts.
- **VPS**: Der Server kommt als statisch gelinktes Linux-Binary ins Release, siehe `server/README.md`, Abschnitt "Auf einem VPS".

Wie man ein Release macht, steht in `README.md`, Abschnitt "So veröffentlichst du eine Version".

## Das Lkw-Paket

So liegt ein Firmen-Lkw auf dem Server (`trucks.package` und `trucks.state`):

- **Bauplan:** die `vehicle`-Unit und alle Zubehör-Units als Text. Beim Einsetzen bekommen sie neue, eindeutige Unit-Namen.
- **Zustand:** Kilometerstand, Wartungs-Kilometer, Verschleiß je Bauteil, Tank.
- **Kennzeichen:** eindeutig je Lkw und Firma. Der Chef legt es beim Anlegen fest, oder der Server schlägt eins vor (Kürzel der Firma und nächste freie Nummer, etwa `NL 001`), und der Chef kann es ändern, wenn der Lkw nicht verliehen ist. Beim Einsetzen schreibt der Launcher das Kennzeichen vom Server als selbst vergebenes Kennzeichen in den Spielstand (`NL 001|_germany`, Land aus dem Bauplan), nicht das Kennzeichen im Bauplan. Das stammt aus dem Spielstand, aus dem der Chef den Lkw übernommen hat, und hat im Spiel einem Actros das Kennzeichen eines anderen Firmen-Lkw gebracht. Dauerhaft ist das Kennzeichen trotzdem nicht: Das Spiel kann es ersetzen (im Spiel bestätigt). Der Launcher meldet das neue Kennzeichen bei der Rückgabe, der Server behält die alten in `old_plates`. So lässt sich auch Telemetrie mit einem alten Kennzeichen noch zuordnen. Im Spielstand erkennt SpedHub den Lkw über Modell und Teile wieder, nicht über das Kennzeichen allein.
- **Modell:** aus dem `data_path`, etwa `scania.s_2016`. Zusammen mit dem Kennzeichen erkennt der Server einen Firmen-Lkw im Herzschlag, siehe Online und Herzschlag.
- **Abhängigkeiten:** alle `data_path`-Einträge. Daraus ergibt sich, welche DLCs und Mods ein Fahrer braucht.
- **Revision:** steigt bei jeder Rückgabe und bei jeder Reparatur in der Werkstatt.

## Reihenfolge

1. Tests von Hand im Spiel: Einsetzen bestätigt, der Rest steht unten
2. Save-Bibliothek in Rust: erste Fassung
3. Launcher mit Leihe vor dem Start, Plugin für Telemetrie, Geldabgleich: erste Fassung. Dazu gekommen sind spät gemeldete Ereignisse, Herzschlag und Online-Status, Werkstatt und Haftung, Setup und Updates
4. Aufräumen des Profils beim Start, Lkw-Wechsel im laufenden Spiel, Overlay: offen
5. Lkw-Kauf für die Firma (Anschaffungsbudget, Kauf verknüpfen, Verkauf): erste Fassung. Wirtschafts-Mod: Werkzeug vorhanden (`spedhub-save economy-mod`, `docs/wirtschafts-mod.md`), im Spiel ungeprüft. Convoy-Server: offen
6. Härten: Backups vor jedem Schreiben gibt es. Versionssperre und Mod-Lkw: offen

## Offene Tests im Spiel und auf Windows

Bestätigt ist mit 1.62: Ein mit `spedhub-save` eingesetzter Lkw lädt als eigener, aktiver Lkw in der Heimatgarage, mit Zubehör, Kilometern, Verschleiß und Tank, und der nächste Autosave ist intakt. Das Spiel vergibt dabei alle `_nameless`-Namen neu und kann das Kennzeichen ersetzen, auch durch eins, das ein anderer Firmen-Lkw trägt. Der zweite Test hat außerdem bestätigt: Ein vom Launcher angelegter Speicherplatz erscheint im Spiel und lädt, das Spiel übernimmt den beim Geldabgleich geschriebenen Kontostand, mit einem frischen Profil gibt es kein `Economy reset - company mismatch` (der Reset im ersten Test lag am alten Spielstand), und das Plugin lädt, Herzschläge kommen an und `job.delivered` kommt mit Fracht, Städten, Strecke und Zeitpunkt. Details in `docs/save-format.md`, Abschnitt "Im Spiel bestätigt". Offen ist:

| Test | Frage |
|---|---|
| Kennzeichen | Warum ersetzt das Spiel das Kennzeichen, bei jedem Laden oder nur einmal? Behält es ein vom Launcher gesetztes Firmen-Kennzeichen (`NL 001\|_germany`) nach dem Laden und nach dem nächsten Autosave? Prüfschritte in `docs/save-format.md`, "Kennzeichen" |
| Modell im Herzschlag | Welche Form hat `id` in der Konfiguration `truck` (`scania.s_2016`, `vehicle.scania.s_2016` oder `s_2016`)? Steht im Dashboard bei einem geliehenen Firmen-Lkw "wird gerade gefahren"? |
| Einsetzen neben eigene Lkw | Lädt ein Spielstand, in dem der bisherige eigene Lkw in einen freien Stellplatz gezogen ist? |
| Save-Cache | Übernimmt das laufende Spiel einen geänderten Spielstand zuverlässig, auch beim zweiten Mal? Seit 1.50 gibt es einen Datei-Cache, der alte Versionen liefern kann |
| Kontostand mit Budget | Übernimmt das Spiel den beim Geldabgleich geschriebenen Kontostand auch mit dem Rest eines Anschaffungsbudgets beim Chef? Ohne Budget ist es bestätigt |
| Level | Passt das aus `info_players_experience` berechnete Level zu dem, das das Spiel anzeigt? |
| Reparierter Lkw | Nimmt das Spiel einen nach der Werkstatt mit reparierbarem Verschleiß 0 eingesetzten Lkw so an? |
| Lkw entfernen | Lädt ein Spielstand, dessen aktiver Lkw ersatzlos entfernt wurde? |
| Herzschlag | Liefern die Kanäle die erwarteten Einheiten (Kilometer, Verschleiß 0 bis 1, Liter, Meter je Sekunde)? Welches Format hat `license.plate` genau? |
| Firmenprofil im Log | Heißt die Zeile in `game.log.txt` genau `New profile selected: '<Name>'`, und steht der Name so drin wie im Ordnernamen (Groß- und Kleinschreibung)? Schreibt das Spiel die Datei während des Spiels laufend (Flush), sodass ein Profilwechsel sofort zu lesen ist? Erkennt das Plugin einen Profilwechsel im laufenden Spiel? Lässt das Spiel die Datei zum Lesen offen, während es schreibt? |
| Plugin-Installation | Findet der Launcher ETS2 auf einem echten Windows über Registry und `libraryfolders.vdf`? Klappen Neustart über UAC, Ersetzen und Entfernen bei laufendem Spiel? |
| Setup | Installation, Deinstallation und Update über den Launcher auf einem echten Windows, auch mit einem Standardbenutzer, der im UAC-Dialog einen Admin angibt |
| Laden im Convoy | Bleibt die Verbindung beim Laden stehen? |
| Sicht der anderen | Sehen Mitspieler nach einem Lkw-Wechsel sofort den neuen Lkw? |
| Session-Ende | Welcher Spielstand liegt nach "behalten" oder "zurücksetzen" auf der Platte? |
| Wirtschafts-Mod | Lädt das Spiel `spedhub_economy.scs` (unkomprimiertes ZIP, Kategorie `other`)? Zahlen Aufträge dann 1 Prozent? Meldet die Telemetrie bei `job.delivered` den gekürzten Betrag? Nimmt `fixed_revenue` eine Kommazahl an? Macht `mp_mod_optional: false` die Mod im Convoy zur Pflicht? Liste in `docs/wirtschafts-mod.md`, "Ungeprüft" |

Was hier offen ist, gilt als vorläufig. Code am Spielstand bleibt so gebaut, dass sich die Feldliste in `crates/spedhub-save/src/layout.rs` leicht anpassen lässt.

## Fremde Bausteine

| Projekt | Wofür | Lizenz |
|---|---|---|
| `decrypt_truck` | Spielstände entschlüsseln | MIT, direkt nutzbar |
| `scs-sdk-crates` | Telemetry und Input SDK in Rust | MIT/Apache-2.0, im Plugin als `scs-sdk-plugin` 0.1.1 genutzt |
| TS SE Tool | Aufbau der Save-Units nachlesen, Erfahrungspunkte je Level | Apache-2.0 |
| truck-tools | Schema seit 1.60 nachlesen | GPL-3.0, kein Code übernehmen |
| NMC-SCS-LAUNCHER | Logik für den Lkw-Wechsel nachlesen | ohne Lizenz, kein Code übernehmen |
