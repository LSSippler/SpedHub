# SpedHub-Launcher

Desktop-App für Windows. Hier meldest du dich an, siehst den Fuhrpark der Firma, leihst Lkw aus und gibst sie zurück. Geliehene Lkw schreibt der Launcher in deinen Spielstand, danach startet er das Spiel.

Wichtig: Dass ETS2 einen so bearbeiteten Spielstand lädt, ist noch nicht im Spiel bestätigt. Der Launcher zeigt das als Hinweis an. Er legt immer einen neuen Speicherplatz an und sichert den alten vorher. Leg trotzdem selbst ein Backup deines Profils an, bevor du es zum ersten Mal ausprobierst.

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
4. **Ausleihen:** Der Server vergibt den Lkw. Der Launcher setzt ihn sofort in den neuesten Spielstand des Profils ein und legt das Ergebnis als neuen Speicherplatz `SpedHub: <Kennzeichen>` an. Im Spiel diesen Speicherplatz laden.
5. **In Spielstand schreiben:** dasselbe noch einmal, zum Beispiel wenn beim Ausleihen das Spiel lief.
6. **Zurückgeben:** Der Launcher liest Kilometerstand und Zustand des Lkw aus dem neuesten Spielstand und meldet sie dem Server. Er sucht den Lkw zuerst über das Kennzeichen. Das Spiel kann das Kennzeichen aber ersetzen. Findet er es nicht, erkennt er den Lkw über Modell, Teile und Kilometerstand wieder und meldet das neue Kennzeichen mit. Dafür braucht er den Bauplan aus `leases\`, also den Rechner, auf dem ausgeliehen wurde. Passen zwei Lkw gleich gut, sagt er das und rät nicht. Erst wenn beides nichts findet, gibst du den Kilometerstand von Hand ein.
7. **Lkw in den Fuhrpark übernehmen** (nur Chef): Spielstand wählen, Lkw wählen, übernehmen. Der Lkw kommt mit Bauplan, Kilometerstand und Zustand auf den Server.
8. **Spiel starten:** öffnet `steam://rungameid/227300`.

Läuft das Spiel, schreibt der Launcher nichts in den Spielstand und sagt das auch.

## Was beim Schreiben in den Spielstand passiert

1. Prüfen, ob `eurotrucks2.exe` läuft. Wenn ja, oder wenn sich das nicht prüfen lässt: abbrechen.
2. Den neuesten Speicherplatz des Profils lesen, gemessen an der Änderungszeit von `game.sii`.
3. Prüfen, ob der Lkw schon im Spielstand steht, auch unter einem Kennzeichen, das das Spiel vergeben hat (über Modell und Teile). Wenn ja: abbrechen, sonst stünde er doppelt da.
4. Den Lkw mit `spedhub-save` einsetzen. Kilometerstand und Zustand kommen vom Server.
5. In `info.sii` den Namen `SpedHub: <Kennzeichen>` und die aktuelle Zeit setzen. Zeilenende und Aufbau bleiben, wie das Spiel sie geschrieben hat.
6. Den Ausgangs-Speicherplatz nach `%LOCALAPPDATA%\SpedHub\backups\<Profil>\<Zeit>-<Speicherplatz>\` kopieren.
7. Kurz vor dem Schreiben noch einmal prüfen, ob das Spiel läuft.
8. Einen neuen Speicherplatz anlegen, die nächste freie Nummer unter `save\` (`1`, `2`, …), und `game.sii` und `info.sii` hineinschreiben. Vorhandenes wird nie überschrieben. Scheitert das Schreiben, wird der neue Ordner wieder entfernt.

## Dateien

Alles liegt in `%LOCALAPPDATA%\SpedHub`:

| Datei | Inhalt |
|---|---|
| `launcher.json` | Server, Name, Firma, Profil und der Token. Der Token ist mit DPAPI für deinen Windows-Benutzer verschlüsselt |
| `plugin.json` | Server, Token und Firma für das Plugin. Wird geschrieben, sobald eine Firma gewählt ist, und beim Abmelden entfernt. Das Plugin liest den Token im Klartext |
| `leases\<id>.json` | Bauplan geliehener Lkw. Den gibt der Server nur beim Ausleihen heraus |
| `backups\` | Kopien der Speicherplätze, aus denen der Launcher einen neuen gemacht hat |

## Aufbau

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Oberfläche (egui/eframe). Alles, was dauert, läuft in einem eigenen Thread |
| `src/client.rs` | Client für den Server |
| `src/profiles.rs` | Profile, Hex-Namen, Speicherplätze |
| `src/savegame.rs` | Lkw übernehmen, einsetzen, für die Rückgabe lesen, über `spedhub-save` |
| `src/settings.rs` | `launcher.json`, `plugin.json`, Ablage der Baupläne |
| `src/system.rs` | Windows-Anbindung: Dokumente-Ordner, laufendes Spiel, Steam, DPAPI |

Außer `main.rs` lässt sich alles ohne Fenster testen:

```powershell
cd C:\dev\SpedHub
cargo test -p spedhub-launcher
```

Die Tests bauen einen Dokumente-Ordner mit der echten Profilstruktur aus `testdata` und starten einen echten `spedhub-server`.
