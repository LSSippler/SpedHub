# Der Spielstand von ETS2

Notizen aus der Arbeit mit echten Spielständen aus ETS2 1.62 (Save-Version 105, ohne Mods). Was aus fremden Quellen stammt und nicht selbst geprüft ist, ist als solches markiert.

## Dateien

Ein Spielstand liegt unter `Dokumente\Euro Truck Simulator 2\profiles\<Profil>\save\<Slot>\`. Der Profilordner ist der Profilname als Hex. Profile mit Steam Cloud liegen unter `steam_profiles`.

| Datei | Inhalt |
|---|---|
| `game.sii` | Der ganze Spielzustand |
| `info.sii` | Name des Spielstands, Zeit, Geld für die Anzeige im Ladebildschirm, Liste der DLCs und Mods |
| `preview.tga` | Vorschaubild, fehlt bei Autosaves |

Slots heißen `autosave`, `autosave_job`, `autosave_drive_1` bis `_9`, `quicksave`, `multiplayer_backup` und für eigene Spielstände `1`, `2`, `3`. Der angezeigte Name steht nur in `info.sii`.

## Im Spiel bestätigt

Test mit ETS2 1.62, Spielstand mit `spedhub-save` gebaut (Lkw eingesetzt):

- **Der Spielstand lädt.** Der Lkw ist eigener und aktiver Lkw in der Heimatgarage. Alle 75 Zubehör-Units, Kilometerstand, Verschleiß und Tank sind erhalten. Damit lädt das Spiel auch einen `game.sii` im Klartext, denn `spedhub-save` schreibt nur Klartext.
- **Der nächste Autosave des Spiels ist sauber.** Er besteht die Integritätsprüfung von `spedhub-save`.
- **Das Spiel vergibt beim Speichern alle `_nameless`-Namen neu.** Von den Namen, die `spedhub-save` vergeben hat, ist danach keiner mehr da. Unit-Namen taugen also nicht, um einen Lkw über mehrere Speichervorgänge wiederzuerkennen.
- **Das Spiel hat das Kennzeichen ersetzt.** Aus dem selbst vergebenen `"BZ PM 1411|_germany"` wurde ein vom Spiel erzeugtes Kennzeichen der Heimatstadt mit Markup (`"EF<offset ...>XU 577|germany"`). Das Kennzeichen taugt also nicht als dauerhafte Kennung eines Firmen-Lkw. SpedHub erkennt Lkw deshalb über Modell und Teile wieder, siehe [Lkw wiedererkennen](#lkw-wiedererkennen).

Noch offen dazu:

- Warum das Spiel das Kennzeichen ersetzt und ob es das bei jedem Laden tut oder nur einmal.
- Beim Laden stand im Log `Economy reset - company mismatch`. Dieselbe Meldung kam auch beim Laden eines unveränderten alten Spielstands mit laufendem Auftrag. Ob das Einsetzen sie auslöst, ist offen.
- Ob ein vom Launcher angelegter neuer Speicherplatz im Spiel erscheint, siehe [Neuer Speicherplatz](#neuer-speicherplatz).

`testdata/nach_spiel_neues_kennzeichen.sii` bildet das nach: `erwartet_nach_einsetzen.sii` mit neuen Unit-Namen, einem Kennzeichen mit Markup und 425 km mehr.

## Formate

| Kennung | Bedeutung |
|---|---|
| `SiiN` | Klartext |
| `ScsC` | verschlüsselt und komprimiert, darin steckt binär oder Text |
| `BSII` | binär |

Das Spiel schreibt im Standard `ScsC`. Mit `uset g_save_format "2"` in der `config.cfg` schreibt es Klartext. Zum Lesen ist das nicht nötig: `decrypt_truck` 1.3.6 wandelt alle Formate in Text um (siehe `spedhub-save decode`). Laut Community-Quellen lädt das Spiel Klartext unabhängig von der Einstellung. Selbst geprüft ist das noch nicht.

Ein echtes `game.sii` aus 1.62 ist `ScsC` mit `BSII` darin, ein echtes `info.sii` ist Klartext. Bei kaputten `BSII`-Daten gerät `decrypt_truck` manchmal in Panik statt einen Fehler zu melden. `spedhub-save` fängt das ab und meldet einen Lesefehler.

## Aufbau des Textformats

Wichtig vorweg: Das Layout unten stammt vom Serializer in `decrypt_truck`, also von dem Text, den die Bibliothek aus `BSII` macht. Es ist nicht das, was das Spiel schreibt. Wie das Spiel selbst ein `game.sii` als Text schreibt (`g_save_format 2`), ist noch nicht bekannt.

```
SiiNunit
{
economy : _nameless.20e.3442.4010 {
 bank: _nameless.20e.3442.4168
 garages: 2
 garages[0]: garage.erfurt
 garages[1]: garage.munchen
}

bank : _nameless.20e.3442.4168 {
 money_account: 4000
}

}
```

So sieht der Text aus `decrypt_truck` aus, und so sind `quelle_mit_lkw.sii`, `ziel_schnellauftrag.sii` und `erwartet_nach_einsetzen.sii` aufgebaut:

- Die Datei beginnt mit `SiiNunit`, dann `{`.
- Jede Unit ist `typ : name {`, dann Felder, dann `}` und eine Leerzeile. Vor der ersten Unit steht keine Leerzeile.
- Feldzeilen sind ` name: wert`, mit einem Leerzeichen davor und einem nach dem Doppelpunkt. Ein Feldname enthält nie `: `, ein Wert kann `:` enthalten (`text: "D:UDO"`).
- Zeilenenden sind LF, nach der letzten `}` kommt kein Zeilenumbruch.
- Listen stehen als Zählzeile `feld: N` und danach `feld[0]` bis `feld[N-1]`. In allen Testdateien steht die Zählzeile direkt vor den Einträgen, und die Einträge laufen lückenlos.
- Unit-Namen sind entweder `_nameless.` plus Hex-Gruppen oder sprechend wie `garage.erfurt`, `driver.0`, `company.volatile.x.y`.
- Ein Feld ist ein Verweis, wenn sein ganzer Wert dem Namen einer Unit in der Datei entspricht. Ein eigenes Kennzeichen für Verweise gibt es nicht.
- Kommazahlen stehen oft als Hex mit `&` davor, zum Beispiel `&3dd0332a`. Das sind die Bits eines 32-Bit-Floats. Ganze Werte stehen dezimal da (`0`, `1`), alle anderen als Hex, auch genau darstellbare wie `rheostat_factor: &3f400000` (0,75).

Ein echtes `info.sii`, wie das Spiel es schreibt, hat denselben Aufbau, aber CRLF als Zeilenende und einen Zeilenumbruch nach der letzten `}`. Das Ende sieht so aus: letzte Feldzeile, CRLF, `}`, CRLF, leere Zeile mit CRLF, `}`, CRLF. So ist `testdata/ziel_info.sii` angelegt.

### Wie spedhub-save den Text liest und schreibt

Weil sich die Herkunft der Texte unterscheidet, ist der Parser an drei Stellen tolerant:

- **Zeilenende:** LF und CRLF. Er merkt sich je Datei das Zeilenende und schreibt es genauso zurück. Gemischte Zeilenenden werden gelesen und beim Schreiben auf das häufigere vereinheitlicht, bei Gleichstand auf LF. Ein CR ohne LF dahinter ist ein Fehler.
- **Dateiende:** mit oder ohne Zeilenumbruch nach der letzten `}`. Auch das bleibt beim Schreiben, wie es war.
- **Einrückung:** ein oder mehrere Leerzeichen oder Tabs vor einem Feld. Jede Zeile behält ihre Einrückung. Neue Felder, zum Beispiel beim Einsetzen eines Lkw, bekommen die häufigste Einrückung der Datei.

Alles andere liest er streng: Kopf, Unit-Köpfe, genau eine Leerzeile nach jeder Unit. Was er annimmt, gibt er byte-genau so wieder aus, außer bei gemischten Zeilenenden. Weicht eine Datei ab, gibt es einen Fehler mit Zeilennummer statt einer stillen Korrektur.

Schreibt `spedhub-save` eine Kommazahl neu, nimmt es dezimal für ganze Werte und sonst Hex. Ein Wert, der sich nicht ändert, bleibt Zeichen für Zeichen stehen.

## Unit-Namen

In echten Spielständen aus 1.62 haben alle rund 15.000 `_nameless`-Namen die Form `_nameless.XXX.XXXX.XXXX`: drei Hex-Gruppen mit 3, 4 und 4 Stellen, die hinteren mit führenden Nullen, Kleinbuchstaben. Alle enden auf `8`. Das sieht nach ausgerichteten Speicheradressen aus. In den Testdaten ist es genauso.

Das Spiel vergibt beim Speichern alle `_nameless`-Namen neu (im Spiel bestätigt). Eigene Namen müssen also nur innerhalb einer Datei eindeutig sein und die richtige Form haben.

Neue Units bekommen von `spedhub-save` Namen derselben Form: ab `_nameless.2ff.7a11.0008` in Schritten von 0x10, also `.0008`, `.0018`, `.0028` und so weiter, immer auf `8` endend. Vergebene Namen werden übersprungen, nach `.fff8` geht es mit `_nameless.2ff.7a12.0008` weiter. `tools/transplant.py` nimmt für sein neues `profit_log` denselben ersten Namen, damit beide Ergebnisse byte-genau vergleichbar bleiben. Das Schema steht in `crates/spedhub-save/src/layout.rs`.

## Was immer gelten muss

In allen untersuchten Originalen gilt:

1. Jeder Unit-Name kommt genau einmal vor.
2. Jeder `_nameless`-Verweis zeigt auf eine vorhandene Unit.
3. Jede Unit ist von der ersten Unit (`economy`) aus über Verweise erreichbar.

Laut Community-Quellen bricht das Spiel das Laden ab, wenn es mehrere unverbundene Unit-Bäume findet, und fällt auf ein Backup zurück. Wer eine Unit entfernt, muss also alles mit entfernen, was nur an ihr hängt. Wer eine hinzufügt, muss sie irgendwo verweisen.

`spedhub-save` prüft das vor jedem Schreiben, dazu noch zwei eigene Punkte:

4. Jede Liste ist vollständig: Zählzeile direkt vor den Einträgen, Einträge lückenlos von 0 bis N-1.
5. Der erzeugte Text lässt sich wieder einlesen und ergibt denselben Spielstand.

Verwaiste Units werden gar nicht toleriert. `tools/transplant.py` hat nur geprüft, dass es nicht mehr werden als vorher.

## Wo der Lkw steht

Schema seit 1.60, in 1.62 so abgelesen:

| Stelle | Bedeutung |
|---|---|
| `player.trucks[]` | Alle eigenen Lkw als Verweise auf `vehicle`-Units |
| `player.truck_profit_logs[]` | Parallel zu `trucks[]`, gleiche Länge, je ein `profit_log` |
| `player.assigned_vehicles` | Verweis auf eine `player_vehicles`-Unit: was der Spieler gerade fährt |
| `player.my_vehicles[0..2]` | Drei weitere `player_vehicles`-Units. Index 0 ist der eigene Lkw. Vermutlich sind 1 und 2 für Bus und Auto |
| `player.my_vehicles_mode` | `none` ohne eigenen Lkw, sonst `truck` |
| `player.current_job` | Verweis auf `player_job` oder `null` |
| `player.hq_city` | Stadt der Heimatgarage, zum Beispiel `erfurt` |
| `garage.<stadt>.vehicles[]` und `drivers[]` | Parallele Slots. `null` ist ein freier Slot. Der Lkw des Spielers steht im selben Slot wie der Spieler-Fahrer |
| `game_progress.owned_trucks[]` | Modellnamen als Text, zum Beispiel `"vehicle.scania.s_2016"`. Keine Verweise |

Eine `player_vehicles`-Unit hat `vehicle`, `stored_vehicle_placement`, `stored_vehicle_city`, `trailer`, `stored_trailer_placements[]` und `stored_trailer_attached`.

`stored_vehicle_city` von `my_vehicles[0]` ist nicht unbedingt die Heimatstadt. In der Testquelle steht dort `city.a_coruna`, die Heimatgarage ist München. Was das Feld genau bedeutet, ist offen.

Garagen haben in den Testdaten 1 bis 5 Stellplätze. Ein freier Stellplatz hat `vehicles[k]: null` und `drivers[k]: null` (`garage.dresden` in der Testquelle). Angestellte Fahrer (`driver_ai`) stehen mit ihrem Lkw im selben Slot. Ihre Felder `assigned_truck` und `adopted_truck` sind trotzdem `null`, die Zuordnung ergibt sich also aus der Garage.

Jeder eigene Lkw hat ein `profit_log` in `player.truck_profit_logs[]`. Daran hängen über `stats_data[]` die `profit_log_entry`-Units, eine pro Zeitraum.

Die alten Felder `assigned_truck`, `my_truck`, `my_truck_placement` und `assigned_trailer` gibt es noch, sie sind in 1.62 aber leer.

Der Spieler-Fahrer heißt in einem frischen Profil `driver.0`. In einem älteren Profil kann es eine andere Nummer sein. Maßgeblich ist `player.drivers[0]`.

## Der Lkw selbst

Ein Lkw ist eine `vehicle`-Unit plus ihre Zubehör-Units in `accessories[]`. Der getestete Scania S kam auf 76 Units. Das Zubehör verweist nicht weiter.

Wichtige Felder der `vehicle`-Unit:

| Feld | Bedeutung |
|---|---|
| `odometer`, `odometer_float_part` | Kilometerstand |
| `integrity_odometer`, `integrity_odometer_float_part` | Kilometer für die Wartung |
| `engine_wear`, `transmission_wear`, `cabin_wear`, `chassis_wear`, `wheels_wear[]` | Verschleiß, 0 bis 1 |
| dieselben mit `_unfixable` | Dauerhafter Verschleiß, seit 1.49 |
| `fuel_relative` | Tankfüllung, 0 bis 1 |
| `license_plate` | Kennzeichen, siehe unten |
| `accessories[]` | Verweise auf Zubehör |

Zubehör-Typen: `vehicle_accessory`, `vehicle_addon_accessory`, `vehicle_paint_job_accessory`, `vehicle_wheel_accessory`, `vehicle_sound_accessory`, `vehicle_drv_plate_accessory`. Jede hat einen `data_path` auf die Definition im Spiel. Das Modell ergibt sich aus dem `data_path` der Form `/def/vehicle/truck/<modell>/data.sii`.

### Kennzeichen

Zwei Formen kommen vor:

- Vom Spiel vergeben, mit Markup: `"DD<offset hshift=4 vshift=-5><img src=/material/ui/lp/germany/b.mat color=FFFFFFFF><offset hshift=4 vshift=5>RI 772|germany"`
- Selbst vergeben, schlicht: `"SPED 001|_germany"`

Hinter dem `|` steht das Land. Bei selbst vergebenen Kennzeichen hat es einen Unterstrich davor.

Das Spiel kann ein selbst vergebenes Kennzeichen durch eins mit Markup ersetzen (im Spiel bestätigt, Grund offen). Ein Lkw ist über sein Kennzeichen also nicht dauerhaft zu finden.

Zum Suchen und Vergleichen nimmt `spedhub-save` den Text vor dem `|`, ersetzt jedes Markup-Tag durch ein Leerzeichen und vergleicht ohne Rücksicht auf Groß- und Kleinschreibung. Aus dem ersten Beispiel wird so `DD RI 772`. Neu gesetzte Kennzeichen sind immer selbst vergebene, mit Großbuchstaben, Ziffern, Leerzeichen, Punkt und Bindestrich, höchstens 16 Zeichen.

## Lkw wiedererkennen

Weil das Spiel Unit-Namen und Kennzeichen ersetzt, findet `spedhub_save::find_package_truck` den Lkw zu einem Paket ohne beides:

- Er gehört dem Spieler oder der Spieler fährt ihn gerade.
- Das Modell ist gleich.
- Sein Kilometerstand ist nicht kleiner als im Paket.
- Die `data_path`-Listen stimmen zu mindestens 80 Prozent überein (`DEFAULT_MIN_MATCH`). Gemessen wird der Anteil der gemeinsamen Einträge an allen Einträgen beider Listen, jeder Eintrag einmal. Getauschte, fehlende und zusätzliche Teile zählen so gleich. Der Test-Scania hat 74 verschiedene Teile. Mit drei getauschten sind es 92 Prozent, bei 80 Prozent verträgt er bis zu acht.

Passen mehrere, gewinnt die größte Übereinstimmung. Liegen die besten gleichauf, etwa bei zwei baugleichen Lkw, ist das ein Fehler (`AmbiguousTruck`) und kein Raten.

## Lkw einsetzen

So macht es `tools/transplant.py` für ein Ziel ohne eigenen Lkw, und `spedhub-save` genauso:

1. `vehicle`-Unit und alles Zubehör aus der Quelle übernehmen. Die Unit-Namen dürfen im Ziel noch nicht vorkommen.
2. Einen neuen, leeren `profit_log` anlegen (`stats_data: 0`, `acc_distance_free: 0`, `acc_distance_on_job: 0`, `history_age: nil`).
3. `player.trucks[]` und `player.truck_profit_logs[]` um je einen Eintrag erweitern.
4. `player.my_vehicles_mode: truck`, in `my_vehicles[0]` den Lkw eintragen, dazu Placement und `stored_vehicle_city: city.<hq_city>`.
5. In `player.assigned_vehicles` den Lkw eintragen. Das Placement bleibt, wie es war.
6. In `garage.<hq_city>` den Lkw in den Slot des Spielers eintragen.
7. `game_progress.owned_trucks[]` um den Modellnamen erweitern.

War das Ziel in einem Schnellauftrag, kommt vorher weg: `player.current_job` auf `null`, die `player_job`-Unit, der Firmen-Lkw mit Zubehör, der Anhänger mit Zubehör. In `assigned_vehicles` dann `trailer: null`, `stored_trailer_placements: 0`, `stored_trailer_attached: false`. Einen Schnellauftrag erkennt man an `player_job.company_truck`, das bei einem Auftrag mit eigenem Lkw `null` ist.

Die Units des Lkw werden direkt hinter der `player`-Unit eingefügt, zuerst die `vehicle`-Unit, dann das Zubehör in der Reihenfolge einer Tiefensuche (das ist umgekehrt zu `accessories[]`), zuletzt das neue `profit_log`. Das ist die Reihenfolge von `transplant.py`.

### Ziel mit eigenen Lkw

Noch nicht im Spiel geprüft. `spedhub-save` geht so vor:

- Der neue Lkw kommt in den Stellplatz des Spieler-Fahrers in der Heimatgarage.
- Stand dort schon ein Lkw, zieht er in einen freien Stellplatz derselben Garage, also ohne Lkw und ohne Fahrer. Gibt es keinen, bricht das Einsetzen ab.
- `assigned_vehicles` und `my_vehicles[0]` zeigen auf den neuen Lkw, Placement wie oben.
- Ein Anhänger in `assigned_vehicles` bleibt, wie er war.
- `owned_trucks[]` bekommt das Modell nur, wenn es noch nicht drinsteht.

Ein laufender Auftrag mit eigenem Lkw und ein Spieler ohne Lkw und ohne Auftrag werden abgelehnt.

## Lkw entfernen

Noch nicht im Spiel geprüft. `spedhub-save` entfernt einen eigenen Lkw so:

1. Weg kommen die `vehicle`-Unit mit Zubehör und das zugehörige `profit_log` mit allen `profit_log_entry`-Units.
2. Der Eintrag in `player.trucks[]` und `player.truck_profit_logs[]` fällt raus. Ist die Liste danach leer, wird `my_vehicles_mode` zu `none`.
3. In Garagen wird der Stellplatz `null`. `driver_ai.assigned_truck` und `adopted_truck` werden `null`, falls sie auf den Lkw zeigen.
4. Ist es der aktive Lkw (`assigned_vehicles` oder `my_vehicles[0]`), muss ein anderer eigener Lkw als Ersatz angegeben sein. Der Ersatz übernimmt den Stellplatz des entfernten Lkw, sein alter Stellplatz wird frei. Fuhr ihn ein angestellter Fahrer, steht der danach ohne Lkw da.
5. Verweist sonst noch etwas auf eine der Units, bricht das Entfernen ab.

`owned_trucks[]` bleibt unverändert, weil unklar ist, ob die Liste den aktuellen Besitz oder die Historie meint.
## Geld

Der Kontostand steht in `bank.money_account`, immer in Euro, als ganze Zahl. Derselbe Betrag steht in `info.sii` in `save_container.info_money_account` für den Ladebildschirm (siehe `testdata/ziel_info.sii`). `spedhub-save money --set` schreibt beide Dateien gemeinsam oder keine.

`info.sii` hat nur die eine Unit `save_container`. Sie enthält außerdem Name, Spielzeit, Save-Version (`version: 105`) und die DLC-Liste (`dependencies[]`).

## Neuer Speicherplatz

Noch nicht im Spiel geprüft. Der Launcher schreibt einen geliehenen Lkw nicht in einen vorhandenen Speicherplatz, sondern legt einen neuen an:

- Ordner ist die nächste freie Nummer unter `save\` (`1`, `2`, …), so wie das Spiel eigene Spielstände nummeriert.
- `game.sii` ist der neueste Spielstand mit eingesetztem Lkw, als Klartext.
- `info.sii` ist die des Ausgangs-Speicherplatzes mit zwei Änderungen: `name` wird `"SpedHub: <Kennzeichen>"`, `file_time` die aktuelle Zeit in Sekunden seit 1970. Zeilenende und Dateiende bleiben, wie das Spiel sie geschrieben hat. Beide Felder stehen in `crates/spedhub-save/src/layout.rs` (`info`).
- `preview.tga` fehlt, wie bei Autosaves.

Offen ist, ob das Spiel den Speicherplatz in der Liste zeigt, ob es nach `file_time` sortiert und ob es einen Klartext-`game.sii` in einem sonst verschlüsselt gespeicherten Profil lädt.
## Offen

- Warum das Spiel Kennzeichen ersetzt und ob es das bei jedem Laden tut, siehe [Im Spiel bestätigt](#im-spiel-bestätigt).
- Woher `Economy reset - company mismatch` beim Laden kommt.
- Was mit `truck_profit_logs` passiert, wenn das Log leer ist: Der Lkw-Manager rechnet laut Community-Quellen Kilometer aus dem Log und nicht aus dem Tacho.
- Wie ein Spielstand aussieht, in dem der Spieler gar keinen Lkw hat und auch keinen Auftrag fährt.
- Einsetzen in ein Profil, das schon eigene Lkw hat, und Entfernen des aktiven Lkw mit Ersatz: so gebaut wie oben beschrieben, im Spiel nicht geprüft.
- Wie das Spiel ein `game.sii` als Klartext schreibt (`g_save_format 2`): Zeilenende, Einrückung, Dateiende.
- Ob `owned_trucks[]` beim Entfernen eines Lkw angepasst werden muss.
- Ob ein vom Launcher angelegter Speicherplatz im Spiel erscheint und lädt, siehe oben.
- Lkw mit Mod-Teilen.
