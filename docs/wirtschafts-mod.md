# Wirtschafts-Mod

Stand November 2026, ETS2 1.62. Im Spiel ist die Mod noch nicht geprüft, siehe [Ungeprüft](#ungeprüft).

## Wozu

Im Spiel bekommt der Fahrer den Erlös eines Auftrags auf sein Konto. Bei SpedHub gehört der Erlös aber der Firma, und das Geld im Spiel ist nur Lohnkonto plus Betriebsvorschuss (siehe `docs/architektur.md`, Kasse und Geldabgleich). Ohne Mod hat der Fahrer nach jedem Auftrag viel mehr Geld im Spiel, als ihm gehört, und der Geldabgleich muss das jedes Mal wieder abziehen.

Mit der Mod `spedhub_economy.scs` zahlen Aufträge im Spiel nur 1 Prozent des üblichen Erlöses. Das Plugin meldet diesen Betrag an den Server, und der rechnet ihn mit dem Erlösfaktor 100 zurück:

| | Ohne Mod | Mit Mod und Erlösfaktor 100 |
|---|---|---|
| Im Spiel aufs Konto | 5000 € | 50 € |
| Vom Plugin gemeldet | 5000 € | 50 € |
| In die Firmenkasse | 5000 € (Faktor 1) | 5000 € |
| `game_amount` der Buchung | 5000 € | 50 € |

Der Geldabgleich rechnet mit `game_amount`, also mit dem, was im Spiel wirklich angekommen ist. Er bleibt also richtig.

Weil das Spiel ganze Euro auszahlt, ist der zurückgerechnete Erlös nur auf etwa 50 € genau (ein halber Euro mal 100). Ob das Spiel rundet oder abschneidet, ist nicht geprüft.

## Warum ein Werkzeug statt einer fertigen Mod

Die Werte stehen in `def/economy_data.sii` im Archiv `def.scs` des Spiels. Die Datei gehört SCS und kommt nicht ins Repo, auch nicht verändert. Deshalb baut `spedhub-save economy-mod` die Mod aus der Datei, die du selbst aus deinem Spiel entpackst:

- Alle Werte, die den Erlös bestimmen, werden mit 0,01 multipliziert. Alles andere bleibt Byte für Byte gleich: Zeilenenden, Einrückung, Kommentare, Reihenfolge, `@include`-Zeilen, andere Units.
- Daraus entsteht `spedhub_economy.scs`, ein ZIP-Archiv mit `manifest.sii`, `description.txt`, `def/economy_data.sii` und, wenn du eins übergibst, einem Vorschaubild.

Die fertige Mod enthält eine geänderte Datei von SCS. Gib sie an die Fahrer deiner Firma weiter, aber lade sie nicht ins Repo hoch.

## 1. economy_data.sii aus def.scs holen

1. **SCS Extractor laden.** Das offizielle Werkzeug von SCS heißt "Game Archive Extractor" und steht im SCS-Modding-Wiki unter modding.scssoft.com, Seite `Documentation/Tools/Game_Archive_Extractor`. Das ZIP laden und `scs_extractor.exe` zum Beispiel nach `C:\SpedHub\werkzeug\` entpacken. Ältere Extractor-Fassungen können die Archive seit 1.50 nicht mehr lesen, also immer die aktuelle Fassung von dort nehmen.
2. **def.scs finden.** Sie liegt im Spielordner, bei einer Standard-Installation unter `C:\Program Files (x86)\Steam\steamapps\common\Euro Truck Simulator 2\def.scs`. In Steam: Rechtsklick auf Euro Truck Simulator 2, Verwalten, Lokale Dateien durchsuchen.
3. **Entpacken**, in PowerShell, in einen eigenen Ordner außerhalb des Spielordners:

   ```powershell
   cd C:\SpedHub\werkzeug
   .\scs_extractor.exe "C:\Program Files (x86)\Steam\steamapps\common\Euro Truck Simulator 2\def.scs" C:\SpedHub\entpackt
   ```

   Laut Wiki ist der Aufruf `scs_extractor <archiv> [<zielordner>]`. Gebraucht wird nur die eine Datei `C:\SpedHub\entpackt\def\economy_data.sii`. Den Rest kannst du danach löschen. `scs_extractor.exe` ohne Angaben zeigt, welche Optionen deine Fassung kennt. Kann sie einzelne Dateien auswählen, reicht `def/economy_data.sii`.
4. **Prüfen:** Die Datei mit dem Editor öffnen. Sie muss mit `SiiNunit` beginnen. Ist sie verschlüsselt (`ScsC`), binär (`BSII`) oder mit `3nK` kodiert, bricht das Werkzeug ab. Aus `def.scs` kommt sie normalerweise als Klartext.

## 2. Mod bauen

`spedhub-save.exe` installiert das Setup neben den Launcher, normalerweise nach `C:\Program Files\SpedHub\`. Ohne Setup gibt es sie in der CI als Artefakt `spedhub-save-windows-x64`, oder du baust sie selbst:

```powershell
cd C:\dev\SpedHub
cargo build --release -p spedhub-save
.\target\release\spedhub-save.exe help
```

Dann die Mod bauen:

```powershell
& "C:\Program Files\SpedHub\spedhub-save.exe" economy-mod C:\SpedHub\entpackt\def\economy_data.sii -o C:\SpedHub\spedhub_economy.scs
```

Ohne `-o` landet `spedhub_economy.scs` im aktuellen Ordner. Ohne Datei oder mit einem falschen Pfad bricht der Befehl ab und zeigt die Schritte aus Abschnitt 1.

| Option | Bedeutung |
|---|---|
| `-o <ziel.scs>`, `--output <ziel.scs>` | Wohin die Mod geschrieben wird, Standard `spedhub_economy.scs` |
| `--factor <zahl>` | Faktor für die Erlös-Werte, Standard `0.01`. Erlaubt ist 0.0001 bis 1, mit Punkt. Der passende Erlösfaktor im Dashboard ist 1 durch diese Zahl |
| `--also <schlüssel>` | Weitere Schlüssel mitskalieren, mehrfach oder mit Komma getrennt (`--also a,b`). Ohne Index angeben, Listen werden ganz skaliert |
| `--version <x>` | `package_version` im Manifest, Standard die Version von SpedHub |
| `--compatible <muster>` | `compatible_versions[]` im Manifest, Standard `1.62.*` |
| `--icon <bild.jpg>` | Vorschaubild für den Mod-Manager. Laut Wiki ein JPG mit 276 x 162 Pixeln. Nur dann steht `icon` im Manifest |
| `--force` | Eine vorhandene Zieldatei ersetzen. Ohne `--force` bricht der Befehl ab und lässt sie, wie sie ist |

Die Ausgabe sieht so aus (mit der selbst geschriebenen Beispieldatei `testdata/economy_beispiel.sii`, nicht mit dem Original):

```
Gelesen: testdata/economy_beispiel.sii. Faktor 0.01, also 1 Prozent des Erlöses
Geändert:
  Zeile 23: fixed_revenue: 450 -> 4.5  (war eine ganze Zahl, bitte prüfen, ob das Spiel hier eine Kommazahl annimmt)
  Zeile 24: revenue_coef_per_km: 2.75 -> 0.0275
  Zeile 25: cargo_market_revenue_coef_per_km: &40200000 -> &3ccccccd
  ...
Nicht verändert, bitte prüfen (mit --also <schlüssel> mitskalieren):
  Zeile 32: job_bonus_per_km: 0.4
  ...
Hinweis: Zeile 10 bindet eine Datei ein (@include "beispiel_zusatz.sui"). Was dort steht, ist nicht geprüft und nicht verändert
Geschrieben: C:\SpedHub\spedhub_economy.scs (...)
```

Was du dir ansehen solltest:

- **Geändert:** Jede geänderte Zahl mit altem und neuem Wert. Skaliert werden `revenue_coef_per_km`, `cargo_market_revenue_coef_per_km`, `driver_revenue_coef_per_km`, `driver_cargo_market_revenue_coef_per_km` und `fixed_revenue`, in jeder Unit der Datei. Die Liste steht in `crates/spedhub-save/src/layout.rs` (`economy_data`).
- **Warnung, ein Schlüssel kommt nicht vor:** Dann hat SCS die Datei geändert. Die Mod wird trotzdem gebaut, aber der fehlende Teil des Erlöses ist nicht gekürzt. Steht keiner der Schlüssel in der Datei, bricht der Befehl ab, dann ist es nicht die richtige Datei.
- **Nicht verändert, bitte prüfen:** Schlüssel, deren Name nach Geld für Aufträge klingt (`revenue`, `reward`, `payment`, `pay`, `bonus`, `income`, `profit`, `salary`, `wage`, `earning`), mit ihrem Wert. Gehört einer davon zum Erlös, die Mod mit `--also <schlüssel> --force` neu bauen. Was davon in der echten Datei steht, ist hier nicht nachgelesen.
- **`@include`:** Bindet die Datei andere Dateien ein, holt das Spiel sie weiter aus `def.scs`. Stehen dort Erlös-Werte, sind sie nicht gekürzt.
- **Ganze Zahl wird Kommazahl:** Steht bei `fixed_revenue` eine ganze Zahl, die mit 0,01 nicht ganz bleibt (450 wird 4.5), ist ungeprüft, ob das Spiel an dieser Stelle eine Kommazahl liest.

### Zahlenformate

- Dezimalzahlen (`12`, `1.5`, `-0.25`, `2e3`) werden exakt gerechnet und als Dezimalzahl ohne Exponent geschrieben: `2.75` wird `0.0275`, `3e1` wird `0.3`, `1200` wird `12`.
- Hex-Zahlen mit `&` (die Bits eines 32-Bit-Floats, `&40200000` ist 2,5) bleiben Hex und werden auf den nächsten 32-Bit-Float gerundet: `&40200000` wird `&3ccccccd` (0,025). Groß- oder Kleinschreibung der Hex-Ziffern bleibt.
- Listen (`schluessel[0]: ...` oder `schluessel[]: ...`): Alle Einträge werden skaliert, die Zählzeile `schluessel: N` nicht.
- Vektoren in Klammern, `(1, 2.5, &3f800000)`: Jede Komponente wird skaliert.
- Ein Wert, der keine Zahl ist (Zeichenkette, `nil`), führt bei einem zu skalierenden Schlüssel zum Abbruch.
- Kommentare (`#`, `//`, `/* */`) und Zeichenketten werden nie angefasst, auch wenn darin ein Schlüssel steht.

Vor dem Schreiben prüft das Werkzeug das Ergebnis: Die geänderte Datei wird noch einmal gelesen, dieselben Felder in derselben Reihenfolge, außerhalb der geänderten Zahlen derselbe Text, jede neue Zahl mit dem erwarteten Wert. Das ZIP wird gebaut, wieder gelesen und mit CRC-32 geprüft, dann über eine temporäre Datei geschrieben.

### Das ZIP

Die Einträge sind unkomprimiert (Methode "stored", mit CRC-32), Namen mit `/` und ohne führenden Schrägstrich, Datum fest 2026-01-01, sodass dieselbe Eingabe dieselbe Datei ergibt. ETS2 lädt Mods als ZIP-Archive mit der Endung `.scs`. Unkomprimierte Archive sind in der Community üblich (7-Zip mit "Speichern"). Geschrieben wird das ZIP von einem kleinen eigenen Schreiber (`crates/spedhub-save/src/zip.rs`), ohne neue Abhängigkeit.

### manifest.sii

```
SiiNunit
{
mod_package : .package_name
{
	package_version: "0.2.0"
	display_name: "SpedHub Wirtschaft"
	author: "SpedHub"

	category[]: "other"

	compatible_versions[]: "1.62.*"

	description_file: "description.txt"

	mp_mod_optional: false
}
}
```

- **`category[]: "other"`:** Eine Kategorie für Wirtschaft gibt es nicht. Die gültigen Werte laut SCS-Modding-Wiki, Seite `Documentation/Engine/Mod_manager` (zitiert in Community-Quellen, das Wiki selbst war beim Schreiben nur über Suchergebnisse lesbar): `truck`, `trailer`, `interior`, `tuning_parts`, `ai_traffic`, `sound`, `paint_job`, `cargo_pack`, `map`, `ui`, `weather_setup`, `physics`, `graphics`, `models`, `movers`, `walkers`, `prefabs`, `other`. Ein unbekannter Wert wie `economy` wird laut einem Bericht aus der Community vom Spiel verworfen. Andere Wirtschafts-Mods nehmen ebenfalls `other`.
- **`mp_mod_optional: false`:** Laut demselben Wiki-Abschnitt ein optionaler Wahrheitswert. `true` erlaubt das Beitreten zu einer Sitzung auch ohne die Mod, wenn der Server optionale Mods zulässt und der Spieler sie im Sitzungsbrowser erlaubt. Mit `false` ist die Mod im Convoy Pflicht. Ob das Spiel das so umsetzt, ist ungeprüft.

### description.txt

Deutsch, für den Mod-Manager: dass Aufträge nur noch 1 Prozent zahlen, welche Werte geändert sind, dass SpedHub mit dem Erlösfaktor 100 zurückrechnet und wo er eingestellt wird, dass die Mod im Convoy Pflicht ist, und der Link zum Repo.

## 3. Mod installieren

Jeder Fahrer der Firma braucht dieselbe Datei. Am einfachsten baut der Chef sie einmal und gibt sie weiter.

1. `spedhub_economy.scs` nach `Dokumente\Euro Truck Simulator 2\mod\` kopieren. Fehlt der Ordner `mod`, anlegen. Bei einem verschobenen Dokumente-Ordner (OneDrive) liegt er dort.
2. ETS2 starten, das Firmenprofil wählen (das Profil, das der Launcher verwaltet) und im Profilmenü den Mod-Manager öffnen.
3. "SpedHub Wirtschaft" in die Liste der aktiven Mods übernehmen und mit "Änderungen bestätigen" speichern. Die Mod-Liste gilt je Profil.

Das Spiel kann beim ersten Laden mit einer neuen Mod nachfragen oder Aufträge auf dem Frachtmarkt neu würfeln. Bereits angenommene Aufträge können noch mit dem alten Erlös zahlen, das ist nicht geprüft.

## 4. Pflicht-Mod auf dem Convoy-Server

Ein Dedicated Server lädt selbst keine Mods. Er bekommt eine Liste der Pakete, die jeder Beitretende haben muss:

1. Im Spiel die Konsole einschalten: in `Dokumente\Euro Truck Simulator 2\config.cfg` `uset g_developer "1"` und `uset g_console "1"` setzen.
2. Das Firmenprofil mit aktiver Mod laden und ins Spiel gehen. Die Konsole geht erst auf der Karte, nicht im Hauptmenü.
3. In der Konsole `export_server_packages` eingeben.
4. Das Spiel schreibt `server_packages.sii` und `server_packages.dat` nach `Dokumente\Euro Truck Simulator 2\`. Beide Dateien in den Ordner des Dedicated Servers kopieren, in dem auch `server_config.sii` liegt, und den Server neu starten.
5. In `server_packages.sii` steht unter `mod_list` jede Mod, die beim Export aktiv war. Alle davon werden Pflicht. Deshalb aus einem Profil exportieren, in dem nur die Mods der Firma aktiv sind.

Wer ohne die Mod beitreten will, wird abgewiesen oder bekommt die Aufforderung, sie zu aktivieren. Nach jedem neuen Bauen der Mod und nach jedem Update des Spiels die Pakete neu exportieren und neu hochladen.

Diese Schritte stammen aus Anleitungen von Server-Anbietern, nicht von SCS selbst, und sind mit SpedHub nicht geprüft.

## 5. Erlösfaktor im Dashboard

Der Chef stellt im Dashboard im Büro den Erlösfaktor auf 100 (bei `--factor 0.01`) und klickt "Einstellungen speichern". Den passenden Wert nennt auch die letzte Zeile der Ausgabe von `economy-mod`.

Der Faktor gilt für jeden Auftrag, der danach beim Server ankommt, nicht rückwirkend. Mod und Faktor also gleichzeitig umstellen: Ein Auftrag ohne Mod mit Faktor 100 bucht das Hundertfache, einer mit Mod und Faktor 1 ein Hundertstel. Eine falsche Buchung storniert der Chef im Kassenbuch.

## 6. Nach einem Update des Spiels

SCS kann `economy_data.sii` mit jedem Update ändern. Dann:

1. Die neue `economy_data.sii` wie in Abschnitt 1 holen.
2. Die Mod neu bauen, mit der neuen Spielversion und einer neuen Version der Mod, zum Beispiel `--compatible 1.63.* --version 0.2.1 --force`.
3. Die Ausgabe mit der vom letzten Mal vergleichen: Fehlt ein Schlüssel, gibt es neue Kandidaten, hat sich ein Wert stark verändert?
4. Die neue Datei an alle Fahrer geben und die Server-Pakete neu exportieren.

Eine Mod mit der alten `economy_data.sii` überschreibt neue Werte von SCS mit den alten, auch die, die nichts mit dem Erlös zu tun haben.

## Ungeprüft

Im Spiel ist noch nichts davon getestet:

- Ob das Spiel die Mod lädt: unkomprimiertes ZIP, Manifest, Kategorie `other`, Vorschaubild.
- Ob Aufträge dann 1 Prozent zahlen, für Frachtmarkt, Schnellaufträge und Speditionsaufträge mit eigenem Lkw. Ob die Liste der Schlüssel vollständig ist.
- Ob die Telemetrie bei `job.delivered` den gekürzten Betrag meldet (und nicht den vollen).
- Ob `fixed_revenue` eine Kommazahl annimmt, wenn aus einer ganzen Zahl eine wird.
- Ob angestellte Fahrer (`driver_revenue_coef_per_km`, `driver_cargo_market_revenue_coef_per_km`) dann ebenfalls nur 1 Prozent einfahren. SpedHub zählt sie nicht.
- Ob die Strafe beim Abbrechen eines Auftrags vom Erlös abhängt. Dann wäre sie mit der Mod auch nur noch 1 Prozent, und der Server bucht sie ohne Faktor aufs Lohnkonto.
- Ob `mp_mod_optional: false` die Mod im Convoy zur Pflicht macht und `export_server_packages` sie so aufnimmt.
- Welche Optionen `scs_extractor.exe` in der aktuellen Fassung kennt.
- Ob ein Spielstand mit der Mod nach dem Abschalten der Mod normal weiterläuft.
