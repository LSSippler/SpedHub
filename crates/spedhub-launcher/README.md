# SpedHub-Launcher

Desktop-App für Windows. Hier meldest du dich an, siehst den Fuhrpark der Firma, leihst Lkw aus und gibst sie zurück. Geliehene Lkw schreibt der Launcher in deinen Spielstand, danach startet er das Spiel.

Im Spiel bestätigt ist, dass ETS2 einen eingesetzten Lkw lädt (siehe `docs/save-format.md`). Noch nicht bestätigt ist, ob der neue Speicherplatz im Spiel erscheint und ob das Spiel einen geänderten Kontostand übernimmt. Der Launcher zeigt einen Hinweis dazu an. Er legt immer einen neuen Speicherplatz an und sichert den alten vorher. Leg trotzdem selbst ein Backup deines Profils an, bevor du es zum ersten Mal ausprobierst.

## Installation

1. Die .exe holen: entweder als Artefakt `spedhub-launcher-windows-x64` aus dem letzten Lauf der CI auf GitHub (Actions, Lauf öffnen, unten bei Artifacts), oder selbst bauen:

   ```powershell
   cd C:\dev\SpedHub
   cargo build --release -p spedhub-launcher
   ```

   Die Datei liegt dann unter `target\release\spedhub-launcher.exe`. Sie braucht keine Installation und kann an einem beliebigen Ort liegen.

2. Für die Buchungen aus dem Spiel zusätzlich das Plugin installieren, siehe `crates/spedhub-plugin/README.md`. Die Konfiguration für das Plugin schreibt der Launcher selbst.

## Bedienung

1. **Anmelden:** Server-Adresse (Standard `http://127.0.0.1:8787`), Name und Passwort. Mit "Konto anlegen" gibt es ein neues Konto. Die Anmeldung bleibt gespeichert.
2. **Firma wählen:** oben in der Leiste. Eine Firma gründen oder ihr beitreten geht bisher nur im Dashboard.
3. **Firmenprofil wählen:** das ETS2-Profil, in dem du für die Firma fährst. Der Launcher sucht unter `Dokumente\Euro Truck Simulator 2\profiles` und `steam_profiles`. Liegt das Profil unter `steam_profiles`, erscheint eine Warnung: Steam Cloud kann einen bearbeiteten Spielstand mit einer älteren Fassung überschreiben. Am besten nimmst du ein eigenes Profil ohne Steam Cloud.
4. **Ausleihen:** Der Server vergibt den Lkw. Der Launcher setzt ihn sofort in den neuesten Spielstand des Profils ein, gleicht dabei das Geld ab (siehe unten) und legt das Ergebnis als neuen Speicherplatz `SpedHub: <Kennzeichen>` an. Im Spiel diesen Speicherplatz laden.
5. **In Spielstand schreiben:** dasselbe noch einmal, zum Beispiel wenn beim Ausleihen das Spiel lief. Liegt der Bauplan nicht auf diesem Rechner, holt der Launcher ihn vom Server.
6. **Geld abgleichen:** Liest Kontostand und Erfahrungspunkte aus dem neuesten Spielstand, meldet sie dem Server und legt einen neuen Speicherplatz `SpedHub: Geldabgleich` mit dem Kontostand an, den der Server nennt: dein Lohnkonto plus den Betriebsvorschuss der Firma.
7. **Zurückgeben:** Der Launcher liest Kilometerstand und Zustand des Lkw aus dem neuesten Spielstand und meldet sie dem Server. Er sucht den Lkw zuerst über das Kennzeichen. Das Spiel kann das Kennzeichen aber ersetzen. Findet er es nicht, erkennt er den Lkw über Modell, Teile und Kilometerstand wieder und meldet das neue Kennzeichen mit. Dafür braucht er den Bauplan, aus `leases\` oder vom Server. Passen zwei Lkw gleich gut, sagt er das und rät nicht. Erst wenn beides nichts findet, gibst du den Kilometerstand von Hand ein.
8. **Lkw in den Fuhrpark übernehmen** (nur Chef): Spielstand wählen, Lkw wählen, übernehmen. Der Lkw kommt mit Bauplan, Kilometerstand und Zustand auf den Server. Danach bietet der Launcher an, den Kauf zu verknüpfen (siehe unten).
9. **Spiel starten:** öffnet `steam://rungameid/227300`.

Läuft das Spiel, schreibt der Launcher nichts in den Spielstand und sagt das auch.

Über dem Fuhrpark zeigt der Launcher dein Lohnkonto, offenen Lohn, Level, Firmenkilometer und Lohnstufe. Im Fuhrpark steht bei einem verliehenen Lkw "fährt gerade" mit der Geschwindigkeit, wenn das Plugin des Ausleihers gerade Herzschläge mit genau diesem Lkw schickt.

Die Spalte "Schaden" zeigt den Schaden jedes Lkw in Prozent, so wie der Server ihn aus dem Zustand der letzten Rückgabe rechnet, dazu den dauerhaften Verschleiß, etwa `12,5 %, dauerhaft 3 %`. Liegt ein Lkw nach der Rückgabe über der Schadensgrenze der Firma, steht er "in der Werkstatt" und "Ausleihen" ist ausgegraut, bis der Chef ihn im Dashboard reparieren lässt. Nach der Rückgabe sagt der Launcher, wie viel Schaden der Lkw hat, ob er in die Werkstatt muss und was du an Haftung für neuen Schaden aus dem Lohnkonto zahlst, wenn die Firma eine Haftung eingestellt hat. Details in `server/README.md`, Abschnitt "Schaden und Werkstatt".

## Lkw-Kauf verknüpfen

Hat der Chef den Lkw im Spiel gekauft, hat der Geldabgleich den Kaufpreis als Betriebskosten aus der Firmenkasse gebucht. Das Geld dafür bekommt er vorher über ein Anschaffungsbudget, das er im Büro des Dashboards freigibt: Beim nächsten Geldabgleich schreibt der Launcher dann Lohnkonto plus Vorschuss plus den Rest des Budgets in den Spielstand, und die Meldung nach dem Abgleich nennt das Budget.

Nach "Lkw übernehmen" holt der Launcher die letzten Betriebskosten-Buchungen des Chefs, die noch mit keinem Lkw verknüpft sind (Betrag, wie lange her, Notiz des Abgleichs). Gibt es welche, erscheint über dem Fuhrpark "Kauf von <Kennzeichen> verknüpfen": Buchung wählen und "Als Lkw-Kauf verknüpfen", oder "Nicht verknüpfen". Die Buchung bleibt unverändert, im Kassenbuch steht danach "Lkw-Kauf <Kennzeichen>" statt "Betriebskosten". Später geht das auch im Kassenbuch des Dashboards.

Verkaufte und ausgemusterte Lkw stehen eingeklappt unter "Ausgemustert und verkauft", ohne Knöpfe. Verkaufen und ausmustern geht im Dashboard. Details in `server/README.md`, Abschnitt "Anschaffungen".

## Einstellungen

Unten im Fenster, zum Aufklappen. Dort lässt sich das Passwort ändern: altes Passwort, neues (mindestens 8 Zeichen) und die Wiederholung, dann "Passwort ändern". Danach sind alle anderen Anmeldungen mit diesem Konto abgemeldet, zum Beispiel das Dashboard auf einem anderen Rechner. Der Launcher behält seinen Token und bleibt angemeldet, ebenso das Plugin, das denselben Token aus `plugin.json` nimmt. Ein falsches altes Passwort meldet der Launcher im Klartext und meldet dich dabei nicht ab.

Ein Token läuft nach 30 Tagen ohne Benutzung ab. Dann meldet der Launcher, dass die Anmeldung abgelaufen ist, und du meldest dich neu an.

## Geldabgleich

Im Spiel hast du dein Lohnkonto plus den Betriebsvorschuss der Firma. Davon bezahlst du im Spiel Tanken und Reparaturen. Beim Abgleich:

1. Prüfen, ob das Spiel läuft. Wenn ja: abbrechen, der Server erfährt nichts.
2. Den neuesten Speicherplatz lesen: Kontostand aus `game.sii`, Erfahrungspunkte aus `info.sii`, dazu die Änderungszeit von `game.sii`. Beim Ausleihen wird hier auch der Lkw eingesetzt.
3. Kontostand, Erfahrung und die Änderungszeit als `saved_at` (Unix-Sekunden) an den Server. Der vergleicht mit dem, was die gemeldeten Ereignisse bis zu diesem Zeitpunkt erklären: Fehlt Geld, zahlt die Firma es als Betriebskosten. Ist mehr da, schreibt er nichts gut und zeigt es dem Chef. Der erste Abgleich bucht nichts.
4. Den Betrag vom Server in `game.sii` und `info.sii` setzen und den neuen Speicherplatz schreiben, genauso abgesichert wie beim Einsetzen (siehe unten).
5. Dem Server melden, dass der Betrag im Spielstand steht, mit `written_at`: derselben Zeit, die als `file_time` in `info.sii` steht. Ab da zählen Ereignisse zum nächsten Abgleich.

Beide Zeitpunkte kommen von der Uhr dieses Rechners, genau wie die Zeitpunkte, die das Plugin zu jedem Ereignis mitschickt. So ordnet der Server auch ein Ereignis richtig zu, das erst nach dem Abgleich ankommt. Details in `docs/architektur.md`, Abschnitt Geldabgleich.

Startet das Spiel zwischen Schritt 3 und 4, wird nichts geschrieben. Der Server rechnet beim nächsten Abgleich dann vom gelesenen Kontostand weiter.

Wichtig: Ob das Spiel den geänderten Kontostand übernimmt und ob das Level aus den Erfahrungspunkten zum Level im Spiel passt, ist noch nicht im Spiel geprüft.

## Was beim Schreiben in den Spielstand passiert

1. Prüfen, ob `eurotrucks2.exe` läuft. Wenn ja, oder wenn sich das nicht prüfen lässt: abbrechen.
2. Den neuesten Speicherplatz des Profils lesen, gemessen an der Änderungszeit von `game.sii`.
3. Prüfen, ob der Lkw schon im Spielstand steht, auch unter einem Kennzeichen, das das Spiel vergeben hat (über Modell und Teile). Wenn ja: abbrechen, sonst stünde er doppelt da.
4. Den Lkw mit `spedhub-save` einsetzen. Kilometerstand und Zustand kommen vom Server.
5. Geldabgleich mit dem Server und den neuen Kontostand in `game.sii` und `info.sii` setzen.
6. In `info.sii` den Namen `SpedHub: <Kennzeichen>` und die aktuelle Zeit setzen. Zeilenende und Aufbau bleiben, wie das Spiel sie geschrieben hat.
7. Den Ausgangs-Speicherplatz nach `%LOCALAPPDATA%\SpedHub\backups\<Profil>\<Zeit>-<Speicherplatz>\` kopieren.
8. Kurz vor dem Schreiben noch einmal prüfen, ob das Spiel läuft.
9. Einen neuen Speicherplatz anlegen, die nächste freie Nummer unter `save\` (`1`, `2`, …), und `game.sii` und `info.sii` hineinschreiben. Vorhandenes wird nie überschrieben. Scheitert das Schreiben, wird der neue Ordner wieder entfernt.

## Dateien

Alles liegt in `%LOCALAPPDATA%\SpedHub`:

| Datei | Inhalt |
|---|---|
| `launcher.json` | Server, Name, Firma, Profil und der Token. Der Token ist mit DPAPI für deinen Windows-Benutzer verschlüsselt |
| `plugin.json` | Server, Token und Firma für das Plugin. Wird geschrieben, sobald eine Firma gewählt ist, und beim Abmelden entfernt. Das Plugin liest den Token im Klartext |
| `leases\<id>.json` | Bauplan geliehener Lkw, als Zwischenspeicher. Fehlt er, holt der Launcher ihn vom Server. Das dürfen nur du als Ausleiher und der Chef |
| `backups\` | Kopien der Speicherplätze, aus denen der Launcher einen neuen gemacht hat |

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Oberfläche (egui/eframe). Alles, was dauert, läuft in einem eigenen Thread |
| `src/client.rs` | Client für den Server |
| `src/flows.rs` | Bauplan aus der Ablage oder vom Server, Geldabgleich mit neuem Speicherplatz |
| `src/profiles.rs` | Profile, Hex-Namen, Speicherplätze |
| `src/savegame.rs` | Lkw übernehmen, einsetzen, Kontostand setzen, für die Rückgabe lesen, über `spedhub-save` |
| `src/settings.rs` | `launcher.json`, `plugin.json`, Ablage der Baupläne |
| `src/system.rs` | Windows-Anbindung: Dokumente-Ordner, laufendes Spiel, Steam, DPAPI |

Außer `main.rs` lässt sich alles ohne Fenster testen:

```powershell
cd C:\dev\SpedHub
cargo test -p spedhub-launcher
```

Die Tests bauen einen Dokumente-Ordner mit der echten Profilstruktur aus `testdata` und starten einen echten `spedhub-server`.
