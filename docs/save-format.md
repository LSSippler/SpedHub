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

## Formate

| Kennung | Bedeutung |
|---|---|
| `SiiN` | Klartext |
| `ScsC` | verschlüsselt und komprimiert, darin steckt binär oder Text |
| `BSII` | binär |

Das Spiel schreibt im Standard `ScsC`. Mit `uset g_save_format "2"` in der `config.cfg` schreibt es Klartext. Zum Lesen ist das nicht nötig: `decrypt_truck` 1.3.6 wandelt alle Formate in Text um (siehe `tools/siidec`). Laut Community-Quellen lädt das Spiel Klartext unabhängig von der Einstellung. Selbst geprüft ist das noch nicht.

## Aufbau des Textformats

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

- Die Datei beginnt mit `SiiNunit`, dann `{`.
- Jede Unit ist `typ : name {`, dann Felder mit einem Leerzeichen eingerückt, dann `}` und eine Leerzeile.
- Die Datei endet mit `}` ohne Zeilenumbruch danach. Zeilenenden sind LF.
- Listen stehen als Zählzeile `feld: N` und danach `feld[0]` bis `feld[N-1]`.
- Unit-Namen sind entweder `_nameless.` plus Hex-Gruppen oder sprechend wie `garage.erfurt`, `driver.0`, `company.volatile.x.y`.
- Ein Feld ist ein Verweis, wenn sein ganzer Wert dem Namen einer Unit in der Datei entspricht. Ein eigenes Kennzeichen für Verweise gibt es nicht.
- Kommazahlen stehen oft als Hex mit `&` davor, zum Beispiel `&3dd0332a`. Das sind die Bits eines 32-Bit-Floats.

## Was immer gelten muss

In allen untersuchten Originalen gilt:

1. Jeder Unit-Name kommt genau einmal vor.
2. Jeder `_nameless`-Verweis zeigt auf eine vorhandene Unit.
3. Jede Unit ist von der ersten Unit (`economy`) aus über Verweise erreichbar.

Laut Community-Quellen bricht das Spiel das Laden ab, wenn es mehrere unverbundene Unit-Bäume findet, und fällt auf ein Backup zurück. Wer eine Unit entfernt, muss also alles mit entfernen, was nur an ihr hängt. Wer eine hinzufügt, muss sie irgendwo verweisen.

`tools/transplant.py` prüft diese drei Punkte vor dem Schreiben (`Save.check`).

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

## Lkw einsetzen

So macht es `tools/transplant.py` für ein Ziel ohne eigenen Lkw:

1. `vehicle`-Unit und alles Zubehör aus der Quelle übernehmen. Die Unit-Namen dürfen im Ziel noch nicht vorkommen.
2. Einen neuen, leeren `profit_log` anlegen (`stats_data: 0`, `acc_distance_free: 0`, `acc_distance_on_job: 0`, `history_age: nil`).
3. `player.trucks[]` und `player.truck_profit_logs[]` um je einen Eintrag erweitern.
4. `player.my_vehicles_mode: truck`, in `my_vehicles[0]` den Lkw eintragen, dazu Placement und `stored_vehicle_city: city.<hq_city>`.
5. In `player.assigned_vehicles` den Lkw eintragen. Das Placement bleibt, wie es war.
6. In `garage.<hq_city>` den Lkw in den Slot des Spielers eintragen.
7. `game_progress.owned_trucks[]` um den Modellnamen erweitern.

War das Ziel in einem Schnellauftrag, kommt vorher weg: `player.current_job` auf `null`, die `player_job`-Unit, der Firmen-Lkw mit Zubehör, der Anhänger mit Zubehör. In `assigned_vehicles` dann `trailer: null`, `stored_trailer_placements: 0`, `stored_trailer_attached: false`.

## Geld

Der Kontostand steht in `bank.money_account`, immer in Euro. Laut Community-Quellen steht derselbe Betrag zusätzlich in `info.sii` für den Ladebildschirm.

## Offen

- Ob das Spiel das Ergebnis des Einsetzens lädt, ist noch nicht bestätigt.
- Was mit `truck_profit_logs` passiert, wenn das Log leer ist: Der Lkw-Manager rechnet laut Community-Quellen Kilometer aus dem Log und nicht aus dem Tacho.
- Wie ein Spielstand aussieht, in dem der Spieler gar keinen Lkw hat und auch keinen Auftrag fährt.
- Einsetzen in ein Profil, das schon eigene Lkw hat (freier Garagen-Slot, Wechsel des aktiven Lkw).
- Lkw mit Mod-Teilen.
