# SpedHub-Launcher

Desktop-App für Windows. Hier meldest du dich an, siehst den Fuhrpark der Firma, leihst Lkw aus und gibst sie zurück. Geliehene Lkw schreibt der Launcher in deinen Spielstand, danach startet er das Spiel.

Im Spiel bestätigt ist (ETS2 1.62, siehe `docs/save-format.md`), dass ETS2 einen eingesetzten Lkw lädt, dass der neue Speicherplatz des Launchers erscheint und dass das Spiel den Kontostand aus dem Geldabgleich übernimmt. Noch nicht bestätigt ist das mit dem Rest eines Anschaffungsbudgets. Er legt immer einen neuen Speicherplatz an und sichert den alten vorher. Leg trotzdem selbst ein Backup deines Profils an, bevor du es zum ersten Mal ausprobierst.

## Installation

Am einfachsten über das Setup, siehe `README.md` im Wurzelordner, Abschnitt Installation: `SpedHub-Setup-<Version>.exe` aus den Releases auf GitHub. Es installiert den Launcher nach `Programme\SpedHub`, legt die Plugin-DLL daneben und kopiert sie ins Spiel (siehe Plugin). Zwischen zwei Releases gibt es das Setup auch als Artefakt `spedhub-setup-windows-x64` aus dem letzten Lauf der CI (Actions, Lauf öffnen, unten bei Artifacts).

Selbst bauen geht auch:

```powershell
cd C:\dev\SpedHub
cargo build --release -p spedhub-launcher -p spedhub-plugin
```

Dann liegen `spedhub-launcher.exe` und `spedhub_plugin.dll` unter `target\release`. Der Launcher braucht keine Installation, für "Plugin installieren" muss die DLL aber neben ihm liegen. Das Setup selbst baut `installer\build.ps1`, siehe `README.md`.

Unten in der Seitenleiste steht die Version des Launchers, etwa `Version 0.2.1, aktuell`.

## Aussehen

Der Launcher sieht aus wie das Dashboard: dieselben Farben, Schriftgrößen und Bausteine aus `docs/design-system.md`, im Code an einer Stelle in `src/theme.rs`. Links die Seitenleiste mit den Bereichen Fuhrpark, Geld und Spielstand, Übernehmen (nur Chef) und Einstellungen, unten Nutzer, Version und Server. Ein Klick auf den Nutzer öffnet ein Menü mit Einstellungen, Abmelden und, bei mehreren Firmen, Firma wechseln. Oben im Inhalt stehen Hinweise als farbige Karten (Update, Version, Plugin, Firmenprofil) und die Kennzahlen Lohnkonto, Firmenkasse, Plugin und Firmenprofil.

Hell oder dunkel richtet sich nach der Einstellung von Windows. Mit der Umgebungsvariablen `SPEDHUB_THEME=hell` oder `SPEDHUB_THEME=dunkel` lässt sich das festlegen, etwa für Screenshots unter Linux, wo das System nichts meldet (dann ist es hell). Als Schrift nimmt der Launcher Segoe UI aus `C:\Windows\Fonts` (normal und Semibold), fehlt sie, die eingebaute Schrift von egui.

## Plugin

Den Stand des Plugins zeigt der Launcher oben als Kennzahl "Plugin", vor der Anmeldung unter dem Anmelden und in den Einstellungen in einer eigenen Karte. Fehlt das Plugin, ist es veraltet oder ist das Spiel nicht gefunden, steht oben zusätzlich eine Hinweiskarte mit den Knöpfen:

| Anzeige | Bedeutung |
|---|---|
| Plugin installiert | Im Spiel liegt dieselbe DLL wie neben dem Launcher (gleiche SHA-256) |
| Plugin veraltet | Im Spiel liegt eine andere DLL, meist eine ältere |
| Plugin fehlt | Im Spielordner unter `bin\win_x64\plugins` liegt keine `spedhub_plugin.dll` |
| Spiel nicht gefunden | Weder über Steam noch in den Einstellungen ist ein Spielordner bekannt. Darunter steht, warum |
| Keine Plugin-DLL neben dem Launcher | Selbst gebauter Launcher ohne DLL daneben, es lässt sich nichts vergleichen |

"Plugin installieren" startet den Launcher über die Nachfrage nach Adminrechten (UAC) mit `--install-plugin` neu. Das Fenster bleibt offen und zeigt danach den neuen Stand. Läuft das Spiel, wird nichts ersetzt: Die DLL ist dann geladen, die Meldung sagt "Spiel beenden". "Ordner wählen" erscheint, wenn das Spiel nicht gefunden wurde. Der gewählte Ordner muss `bin\win_x64` enthalten und wird in `launcher.json` gemerkt. "Plugin installieren" lässt in diesem Fall zuerst den Ordner wählen, ohne Adminrechte, und übergibt ihn dann.

So sucht der Launcher das Spiel (`src/steam.rs`):

1. Steam-Ordner aus der Registry: `HKCU\Software\Valve\Steam` (`SteamPath`), `HKLM\SOFTWARE\WOW6432Node\Valve\Steam` und `HKLM\SOFTWARE\Valve\Steam` (`InstallPath`).
2. Alle Bibliotheken aus `steamapps\libraryfolders.vdf`, im neuen und im alten Format. Eine kaputte Datei bricht nichts ab, der Steam-Ordner selbst wird trotzdem durchsucht.
3. ETS2 (App-ID 227300) liegt in der Bibliothek, deren `apps` die ID nennt, sonst in der ersten mit `steamapps\appmanifest_227300.acf`.
4. Spielordner ist `steamapps\common\<installdir>` aus dem App-Manifest, sonst `Euro Truck Simulator 2`. Darin muss `bin\win_x64` liegen.

Ein von Hand gewählter Ordner aus den Einstellungen geht der Suche vor, solange er noch passt.

## Updates

Beim Start fragt der Launcher im Hintergrund die GitHub-Releases von `LSSippler/SpedHub` ab (`GET https://api.github.com/repos/LSSippler/SpedHub/releases/latest`, ohne Anmeldung). Unten in der Seitenleiste steht leise "aktuell", warum die Prüfung nicht ging (offline, noch kein Release, Abfragelimit von GitHub) steht im Tooltip und in den Einstellungen unter "Version". Das stört nichts.

Ist das neueste Release neuer als der Launcher, steht oben eine Hinweiskarte "Version X verfügbar" mit dem Knopf "Herunterladen und installieren". Erst der Knopf

1. lädt `SHA256SUMS.txt` und `SpedHub-Setup-<Version>.exe` aus dem Release in `%TEMP%\SpedHub-Update`,
2. vergleicht die SHA-256 des Setups mit der Zeile in `SHA256SUMS.txt`,
3. startet das Setup nur, wenn sie passt, und schließt sich dann, damit das Setup ihn ersetzen kann.

Fehlt das Setup oder `SHA256SUMS.txt` im Release oder passt die Prüfsumme nicht, wird die Datei gelöscht, nichts gestartet und der Grund angezeigt. Ein stilles Update gibt es nicht.

## Version und Server

Der Launcher schickt bei jeder Anfrage seine Version im Header `X-SpedHub-Version`. Beim Anmelden und beim Laden des Fuhrparks vergleicht er sie mit der Version des Servers. Passen Haupt- und Nebenversion nicht (`0.2.x` gegen `0.3.x`), oder meldet der Server keine Version, steht oben eine gelbe Hinweiskarte. Anmelden geht trotzdem, aber manches kann fehlschlagen.

## Kommandozeile

| Aufruf | Was passiert |
|---|---|
| `spedhub-launcher.exe` | Oberfläche |
| `spedhub-launcher.exe --install-plugin` | Plugin ins Spiel kopieren, eine ältere DLL ersetzen. Findet er das Spiel nicht: Meldung und Ordnerdialog |
| `spedhub-launcher.exe --install-plugin --game-dir "D:\Spiele\ETS2"` | Dasselbe in diesen Spielordner, der Ordner wird gemerkt |
| `spedhub-launcher.exe --uninstall-plugin` | Plugin aus dem gemerkten und dem über Steam gefundenen Spielordner entfernen. Fehlt es, ist das kein Fehler. Läuft das Spiel: Nachfrage mit Wiederholen und Abbrechen, bei Abbrechen wird die DLL umbenannt (endet nicht mehr auf `.dll`, das Spiel lädt sie nicht mehr) und zum Löschen beim nächsten Neustart von Windows vorgemerkt |
| `spedhub-launcher.exe --install-plugin --report-game-dir <Datei>` | Für das Setup: einen von Hand gewählten Spielordner zusätzlich in diese Datei schreiben (UTF-8 mit BOM) |
| `spedhub-launcher.exe --remember-game-dir "D:\Spiele\ETS2"` | Für das Setup: den Spielordner prüfen und in `launcher.json` des aufrufenden Benutzers merken |
| `--quiet` zusätzlich | Keine Fenster, nur der Exit-Code. Für eine stille Installation. Beim Entfernen wird dann nicht gefragt, sondern gleich umbenannt |
| `spedhub-launcher.exe --version` | Version in einem Fenster |

Ohne Oberfläche gibt es keine Ausgabe auf der Konsole, Rückmeldung kommt über den Exit-Code: `0` erledigt, `1` Fehler (stand in einer MessageBox), `2` Spiel nicht gefunden und kein Ordner gewählt, `3` unbekannte Option. In den Spielordner unter `Programme` darf je nach Rechner nur ein Admin schreiben, deshalb ruft das Setup `--install-plugin` mit seinen Adminrechten auf und die Deinstallation `--uninstall-plugin`, still mit `--quiet`.

Mit Adminrechten kann der Launcher unter einem anderen Konto laufen, wenn ein Standardbenutzer im UAC-Dialog die Anmeldung eines Admins eingibt. Ein dort gewählter Spielordner landete in dessen `launcher.json`. Deshalb wählt die Oberfläche den Ordner bei "Spiel nicht gefunden" selbst, bevor sie sich über UAC neu startet, und übergibt ihn mit `--game-dir`. Der Launcher mit Adminrechten fragt dann nicht noch einmal. Das Setup lässt sich einen von Hand gewählten Ordner mit `--report-game-dir` melden und merkt ihn mit `--remember-game-dir` als der Benutzer, der das Setup gestartet hat.

## Bedienung

1. **Anmelden:** Server-Adresse (Standard `http://127.0.0.1:8787`), Name und Passwort. Mit "Konto anlegen" gibt es ein neues Konto. Die Anmeldung bleibt gespeichert.
2. **Firma wählen:** im Menü des Nutzers unten in der Seitenleiste oder in den Einstellungen. Eine Firma gründen oder ihr beitreten geht bisher nur im Dashboard.
3. **Firmenprofil wählen** (Geld und Spielstand): das ETS2-Profil, in dem du für die Firma fährst. Der Launcher sucht unter `Dokumente\Euro Truck Simulator 2\profiles` und `steam_profiles`. Liegt das Profil unter `steam_profiles`, erscheint eine Warnung: Steam Cloud kann einen bearbeiteten Spielstand mit einer älteren Fassung überschreiben. Am besten nimmst du ein eigenes Profil ohne Steam Cloud. Nur was in diesem Profil passiert, zählt für die Firma (siehe Firmenprofil). Ohne gewähltes Profil meldet das Plugin nichts.
4. **Ausleihen:** Der Server vergibt den Lkw. Der Launcher setzt ihn sofort in den neuesten Spielstand des Profils ein, mit dem Kennzeichen, das der Lkw in der Firma hat (siehe Kennzeichen), gleicht dabei das Geld ab (siehe unten) und legt das Ergebnis als neuen Speicherplatz `SpedHub: <Kennzeichen>` an. Im Spiel diesen Speicherplatz laden.
5. **In Spielstand schreiben:** dasselbe noch einmal, zum Beispiel wenn beim Ausleihen das Spiel lief. Liegt der Bauplan nicht auf diesem Rechner, holt der Launcher ihn vom Server.
6. **Geld abgleichen** (Geld und Spielstand): Liest Kontostand und Erfahrungspunkte aus dem neuesten Spielstand, meldet sie dem Server und legt einen neuen Speicherplatz `SpedHub: Geldabgleich` mit dem Kontostand an, den der Server nennt: dein Lohnkonto plus den Betriebsvorschuss der Firma.
7. **Zurückgeben:** Der Launcher liest Kilometerstand und Zustand des Lkw aus dem neuesten Spielstand und meldet sie dem Server. Er sucht den Lkw zuerst über das Kennzeichen, und nur einen mit dem Modell aus dem Bauplan: Ein eigener Lkw mit zufällig gleichem Kennzeichen zählt nicht. Das Spiel kann das Kennzeichen aber ersetzen. Findet er so keinen, oder haben zwei Lkw das Kennzeichen, erkennt er den Lkw über Modell, Teile und Kilometerstand wieder und meldet das neue Kennzeichen mit. Trägt das neue Kennzeichen schon ein anderer Firmen-Lkw, gibt er ohne neues Kennzeichen zurück und sagt das. Dafür braucht er den Bauplan, aus `leases\` oder vom Server. Passen zwei Lkw gleich gut, sagt er das und rät nicht. Erst wenn beides nichts findet, gibst du den Kilometerstand von Hand ein.
8. **Lkw in den Fuhrpark übernehmen** (Bereich Übernehmen, nur Chef): Spielstand wählen, Lkw wählen, Kennzeichen in der Firma festlegen, übernehmen. Das Feld ist mit dem Vorschlag des Servers vorbelegt (Kürzel der Firma und nächste freie Nummer, etwa `NL 003`) und lässt sich ändern. Das Kennzeichen aus dem Spielstand wird nicht das Firmen-Kennzeichen, es kann einem anderen Lkw gehören, etwa das Wunschkennzeichen eines Profils. Der Lkw kommt mit Bauplan, Kilometerstand und Zustand auf den Server. Danach bietet der Launcher an, den Kauf zu verknüpfen (siehe unten).
9. **Spiel starten:** oben rechts in jedem Bereich, öffnet `steam://rungameid/227300`.

Läuft das Spiel, schreibt der Launcher nichts in den Spielstand und sagt das auch.

Oben steht dein Lohnkonto als Kennzahl, unter Geld und Spielstand mit offenem Lohn, Level, Firmenkilometern und Lohnstufe. Der Fuhrpark zeigt jeden Lkw als Karte mit Kennzeichen, Modell, einer Plakette für den Zustand (frei, bei dir, verliehen, wird gefahren, in der Werkstatt), Kilometerstand, Schadensbalken und einer Hauptaktion. Darüber filtern Pillen nach Alle, Frei, Unterwegs und Werkstatt. Bei einem verliehenen Lkw steht "fährt gerade" mit der Geschwindigkeit, wenn das Plugin des Ausleihers gerade Herzschläge mit genau diesem Lkw schickt.

Der Balken "Schaden" zeigt den Schaden jedes Lkw in Prozent, so wie der Server ihn aus dem Zustand der letzten Rückgabe rechnet: grün unter 10 %, bernstein bis zur Schadensgrenze der Firma (der Strich im Balken), rot darüber. Darunter steht der dauerhafte Verschleiß, etwa `dauerhaft 3 %`. Liegt ein Lkw nach der Rückgabe über der Schadensgrenze der Firma, steht er "in der Werkstatt" und "Ausleihen" ist ausgegraut, bis der Chef ihn im Dashboard reparieren lässt. Nach der Rückgabe sagt der Launcher, wie viel Schaden der Lkw hat, ob er in die Werkstatt muss und was du an Haftung für neuen Schaden aus dem Lohnkonto zahlst, wenn die Firma eine Haftung eingestellt hat. Details in `server/README.md`, Abschnitt "Schaden und Werkstatt".

## Kennzeichen

Jeder Firmen-Lkw hat auf dem Server ein eigenes Kennzeichen. Beim Einsetzen setzt der Launcher es ins Paket, bevor der Lkw in den Spielstand geht (`savegame::set_company_plate`, in `package_from_checkout` und `flows::lease_package`): als selbst vergebenes Kennzeichen `<TEXT>|_<land>`, mit dem Land aus dem Kennzeichen im Bauplan. Aus `NL 001` auf dem Server wird `NL 001|_germany`. Das gilt auch für einen Bauplan aus `leases\` von einem älteren Launcher. Der Bauplan selbst bleibt, wie er ist.

Vorher hat der Launcher das Kennzeichen aus dem Bauplan eingesetzt, also das aus dem Spielstand, aus dem der Chef den Lkw übernommen hat. So kam beim Test im Spiel ein Actros mit dem Kennzeichen eines anderen Firmen-Lkw in den Spielstand (siehe `docs/save-format.md`, "Kennzeichen").

Lässt sich das Kennzeichen vom Server nicht setzen (andere Zeichen als A bis Z, Ziffern, Leerzeichen, Punkt und Bindestrich, oder mehr als 16), bricht das Einsetzen mit einer Meldung ab. Der Chef ändert es dann im Dashboard ("Kennzeichen ändern"). Ob das Spiel ein so gesetztes Kennzeichen behält, ist im Spiel noch nicht geprüft.

## Firmenprofil

Ein Fahrer kann in ETS2 mehrere Profile haben, auch private. Gebucht wird nur, was im Firmenprofil passiert. Dafür sorgt der Launcher an zwei Stellen:

- **`plugin.json`:** Er schreibt den angezeigten Namen des gewählten Profils als `profile_name` hinein (aus dem Hex-Ordnernamen gelesen, wie in der Auswahl) und den vollständigen Pfad zu `Dokumente\Euro Truck Simulator 2\game.log.txt` als `game_log`. Daraus liest das Plugin, welches Profil im Spiel gerade aktiv ist, und meldet nur aus dem Firmenprofil. Die Datei schreibt er neu, sobald Firma oder Firmenprofil gewechselt werden, und bei jedem Laden des Fuhrparks, also auch beim Start. Eine `plugin.json` von einem älteren Launcher ohne `profile_name` wird so beim ersten Start ersetzt. Das Spiel muss danach neu starten, das Plugin liest die Datei nur beim Start.
- **Server:** Er meldet das Firmenprofil über `PUT /api/companies/{id}/profile`, wenn der Server ein anderes oder keins kennt: beim Wählen des Profils, beim Wechsel der Firma und beim Start. Das läuft nebenher und sperrt nichts. Ab dann bucht der Server für dich nur noch Ereignisse aus diesem Profil, alle anderen lehnt er ab und zeigt sie dem Chef. Scheitert die Meldung, etwa weil der Server kurz weg ist, steht `plugin.json` schon auf dem neuen Profil, der Server kennt aber noch das alte und lehnt Aufträge aus dem neuen ab. Der Launcher sagt das in der Fehlermeldung und versucht es beim nächsten Laden wieder ("Aktualisieren", nach jeder Aktion). Ein gemeldetes Profil löscht der Launcher nie, auch nicht, wenn keins gewählt ist.

Klappt die Meldung nicht, etwa mit einem älteren Server, steht der Grund in der Meldung und der Launcher versucht es für dieses Profil erst nach einem Neustart wieder. Details in `crates/spedhub-plugin/README.md` und `server/README.md`, Abschnitt "Nur das Firmenprofil zählt".

## Lkw-Kauf verknüpfen

Hat der Chef den Lkw im Spiel gekauft, hat der Geldabgleich den Kaufpreis als Betriebskosten aus der Firmenkasse gebucht. Das Geld dafür bekommt er vorher über ein Anschaffungsbudget, das er im Büro des Dashboards freigibt: Beim nächsten Geldabgleich schreibt der Launcher dann Lohnkonto plus Vorschuss plus den Rest des Budgets in den Spielstand, und die Meldung nach dem Abgleich nennt das Budget.

Nach "Lkw übernehmen" holt der Launcher die letzten Betriebskosten-Buchungen des Chefs, die noch mit keinem Lkw verknüpft sind (Betrag, wie lange her, Notiz des Abgleichs). Gibt es welche, erscheint ein Dialog "Kauf von <Kennzeichen> verknüpfen": Buchung wählen und "Als Lkw-Kauf verknüpfen", oder "Nicht verknüpfen". Die Buchung bleibt unverändert, im Kassenbuch steht danach "Lkw-Kauf <Kennzeichen>" statt "Betriebskosten". Später geht das auch im Kassenbuch des Dashboards.

Verkaufte und ausgemusterte Lkw stehen eingeklappt unter "Ausgemustert und verkauft", ohne Knöpfe. Verkaufen und ausmustern geht im Dashboard. Details in `server/README.md`, Abschnitt "Anschaffungen".

## Einstellungen

Eigener Bereich in der Seitenleiste mit Konto (Firma, Abmelden), Plugin und Version. Dort lässt sich auch das Passwort ändern: altes Passwort, neues (mindestens 8 Zeichen) und die Wiederholung, dann "Passwort ändern". Danach sind alle anderen Anmeldungen mit diesem Konto abgemeldet, zum Beispiel das Dashboard auf einem anderen Rechner. Der Launcher behält seinen Token und bleibt angemeldet, ebenso das Plugin, das denselben Token aus `plugin.json` nimmt. Ein falsches altes Passwort meldet der Launcher im Klartext und meldet dich dabei nicht ab.

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

Das Spiel übernimmt den geschriebenen Kontostand (im Spiel bestätigt, ETS2 1.62). Noch nicht geprüft ist das mit dem Rest eines Anschaffungsbudgets und ob das Level aus den Erfahrungspunkten zum Level im Spiel passt.

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
| `launcher.json` | Server, Name, Firma, Profil, ein von Hand gewählter Spielordner und der Token. Der Token ist mit DPAPI für deinen Windows-Benutzer verschlüsselt |
| `plugin.json` | Server, Token, Firma, Firmenprofil (`profile_name`) und Pfad zu `game.log.txt` (`game_log`) für das Plugin. Wird geschrieben, sobald eine Firma gewählt ist, neu bei jedem Wechsel von Firma oder Firmenprofil, und beim Abmelden entfernt. Das Plugin liest den Token im Klartext |
| `leases\<id>.json` | Bauplan geliehener Lkw, als Zwischenspeicher. Fehlt er, holt der Launcher ihn vom Server. Das dürfen nur du als Ausleiher und der Chef |
| `backups\` | Kopien der Speicherplätze, aus denen der Launcher einen neuen gemacht hat |

Die Deinstallation fragt "Anmeldung, Einstellungen und Backups behalten?", Standard ist Ja. Mit Nein löscht sie diesen Ordner. Gemeint ist der Ordner des Benutzers, der deinstalliert. Läuft `--install-plugin` aus dem Setup mit einem anderen Admin-Konto, schreibt es einen gewählten Spielordner zusätzlich in eine Datei, und das Setup merkt ihn mit `--remember-game-dir` in der `launcher.json` des Benutzers, der das Setup gestartet hat.

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Oberfläche (egui/eframe) und Weiche zur Kommandozeile. Alles, was dauert, läuft in einem eigenen Thread |
| `src/cli.rs` | Kommandozeile ohne Oberfläche: `--install-plugin`, `--uninstall-plugin`, `--remember-game-dir`, `--version`, Argumente, Ablauf mit Ordnerwahl, Exit-Codes |
| `src/steam.rs` | ETS2 über Registry-Werte, `libraryfolders.vdf` und App-Manifest finden, eigener VDF-Leser |
| `src/plugin_install.rs` | Plugin kopieren, ersetzen, entfernen, Stand über SHA-256 |
| `src/update.rs` | GitHub-Releases abfragen, Setup laden und gegen `SHA256SUMS.txt` prüfen |
| `src/version.rs` | Versionen lesen und vergleichen |
| `src/client.rs` | Client für den Server |
| `src/flows.rs` | Bauplan aus der Ablage oder vom Server mit dem Kennzeichen vom Server, Rückgabe mit neuem Kennzeichen, falls frei, Geldabgleich mit neuem Speicherplatz, ob das Firmenprofil gemeldet werden muss |
| `src/profiles.rs` | Profile, Hex-Namen, Name zum Profilordner, Pfad zu `game.log.txt`, Speicherplätze |
| `src/savegame.rs` | Lkw übernehmen, Kennzeichen der Firma ins Paket setzen, einsetzen, Kontostand setzen, für die Rückgabe lesen, über `spedhub-save` |
| `src/settings.rs` | `launcher.json`, `plugin.json`, Ablage der Baupläne |
| `src/system.rs` | Windows-Anbindung: Dokumente-Ordner, laufendes Spiel, Steam, DPAPI, Registry, MessageBox, Ordnerdialog, Neustart mit Adminrechten, Systemschrift |
| `src/theme.rs` | Aussehen nach `docs/design-system.md`: Farben hell und dunkel, Schrift, Abstände, Bausteine (Karte, Plakette, Kennzahl, Hinweiskarte, Kennzeichen, Schadensbalken, Knöpfe, Seitenleiste), Zahlen und Modellnamen formatieren |

Außer `main.rs` lässt sich alles ohne Fenster testen:

```powershell
cd C:\dev\SpedHub
cargo test -p spedhub-launcher
```

Die Tests bauen einen Dokumente-Ordner mit der echten Profilstruktur aus `testdata` und starten einen echten `spedhub-server`. Für Steam bauen sie Registry-Werte, `libraryfolders.vdf`, App-Manifeste und Spielordner in Testordnern nach, für Updates spielt ein lokaler HTTP-Server die GitHub-API nach. Kein Test braucht Windows, Steam oder das Internet.

Nicht ohne Windows prüfbar sind die Teile in `src/system.rs`: Registry lesen, MessageBox, Ordnerdialog (`SHBrowseForFolderW`), Neustart über UAC und das Starten des Setups.
