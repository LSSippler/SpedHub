# Architektur

Stand November 2026, ETS2 1.62.

## Ziel

Eine virtuelle Spedition, die sich wie eine echte anfühlt:

- Die Fahrer fahren gemeinsam im Convoy und sehen sich auf der Straße.
- Einnahmen aus Aufträgen gehen in die Firmenkasse, die Fahrer bekommen einen Lohnanteil.
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
| Server (`server/`) | Accounts, Firma, Kasse, Fuhrpark, Leihen | läuft |
| Dashboard (`server/web/`) | Weboberfläche für Chef und Fahrer | läuft |
| Save-Bibliothek (`crates/spedhub-save`) | Spielstand lesen und schreiben, Lkw exportieren, einsetzen, entfernen, Kontostand setzen | erste Fassung, im Spiel noch nicht geprüft |
| Launcher | Anmelden, Abgleich, Spielstand vorbereiten, Spiel starten, Abgleich beim Beenden | offen |
| Spiel-Plugin | Telemetrie-Ereignisse melden, Quicksave und Quickload auslösen (Telemetry SDK und Input SDK) | offen |
| Overlay | Kasse und Fuhrpark im Spiel | offen |
| Convoy Dedicated Server | Die gemeinsame Session, offizieller Server von SCS | offen |

```
Spiel (ETS2)  <->  Plugin  <->  Launcher  <->  Server
     |                             |
  Spielstand  <-- Save-Bibliothek -+
     |
Convoy Dedicated Server (nur gemeinsames Fahren, kennt weder Geld noch Fuhrpark)
```

## Abläufe

### Start über den Launcher

1. Fahrer meldet sich an und wählt die Firma.
2. Der Launcher holt Kontostand, geliehene Lkw und Fuhrpark vom Server.
3. Der Launcher räumt das Firmenprofil auf: Firmen-Lkw, die laut Server nicht an diesen Fahrer verliehen sind, kommen raus. Verliehene werden aus dem Serverstand neu eingesetzt. Der Kontostand wird auf den Serverwert gesetzt.
4. Der Launcher startet das Spiel. Den Convoy betritt der Fahrer selbst über den Session-Browser, ein automatischer Beitritt ist nicht möglich.

### Lkw ausleihen

- **Vor dem Start:** Fahrer wählt im Launcher einen freien Lkw, der Server vergibt die Leihe, der Launcher setzt den Lkw in den letzten Spielstand ein. Das ist der sichere Weg und der Kern der ersten Version.
- **Über das Hauptmenü:** Fahrer verlässt die Session, wechselt den Lkw, lädt das Profil neu und tritt wieder bei. Das Spiel bleibt offen.
- **Im laufenden Spiel (später):** Quicksave, Lkw einsetzen, Quickload. Im Convoy ist Laden laut SCS instabil und braucht `g_convoy_allow_load 1`.

### Rückgabe

Beim Wechsel und beim Beenden liest der Launcher Kilometerstand, Verschleiß und Tank aus dem Spielstand und meldet sie dem Server. Der zählt die Revision des Lkw hoch und gibt ihn frei. Nach einem Absturz nimmt der Server den letzten Zustand aus der Telemetrie.

### Kasse

Das Plugin meldet Ereignisse aus dem Spiel, der Server bucht sie:

| Ereignis | Fahrer | Firma |
|---|---|---|
| Auftrag abgeliefert | plus Lohnanteil | plus Rest |
| Auftrag abgebrochen, Strafe | minus Betrag | |
| Maut, Fähre, Zug | | minus Betrag |

Ein Ereignis fürs Tanken gibt es im SDK nicht. Tanken und Reparaturen ergeben sich aus dem Geldabgleich.

### Geldabgleich (noch nicht gebaut)

Das Konto im Spiel ist nur eine Anzeige. Bei jedem Abgleich:

1. Erwarteter Stand = Stand beim letzten Abgleich plus gemeldete Einnahmen minus gemeldete Ausgaben.
2. Tatsächlicher Stand = `bank.money_account` im Spielstand.
3. Fehlt Geld, sind das sonstige Ausgaben (Tanken, Reparatur). Sie gehen im Normalfall auf die Firma.
4. Ist mehr Geld da als erklärbar, wird das nicht gutgeschrieben, sondern markiert.
5. Der Launcher schreibt den neuen Kontostand in `bank.money_account` und in `info.sii`.

Betriebsvorschuss: Damit ein Fahrer ohne eigenes Geld tanken kann, zeigt sein Konto im Spiel sein eigenes Geld plus einen festen Vorschuss aus der Firmenkasse.

## Das Lkw-Paket

So liegt ein Firmen-Lkw auf dem Server (`trucks.package` und `trucks.state`):

- **Bauplan:** die `vehicle`-Unit und alle Zubehör-Units als Text. Beim Einsetzen bekommen sie neue, eindeutige Unit-Namen.
- **Zustand:** Kilometerstand, Wartungs-Kilometer, Verschleiß je Bauteil, Tank.
- **Kennzeichen:** eindeutig je Lkw. Darüber erkennt die Telemetrie den Lkw, weil das SDK keine eindeutige Lkw-ID liefert.
- **Abhängigkeiten:** alle `data_path`-Einträge. Daraus ergibt sich, welche DLCs und Mods ein Fahrer braucht.
- **Revision:** steigt bei jeder Rückgabe.

## Reihenfolge

1. Tests von Hand im Spiel (siehe unten)
2. Save-Bibliothek in Rust
3. Launcher mit Leihe vor dem Start, Plugin für Telemetrie, Geldabgleich
4. Lkw-Wechsel im laufenden Spiel, Overlay
5. Convoy-Server, Lohnregeln, Lkw-Kauf für die Firma
6. Härten: Versionssperre, Backups, Mod-Lkw

## Offene Tests im Spiel

| Test | Frage |
|---|---|
| Lkw verpflanzen | Lädt 1.62 einen Spielstand mit eingesetztem fremdem Lkw fehlerfrei? Stimmen Kilometer, Schaden und Tuning? Wo steht der Lkw? |
| Save-Cache | Übernimmt das laufende Spiel einen geänderten Spielstand zuverlässig, auch beim zweiten Mal? Seit 1.50 gibt es einen Datei-Cache, der alte Versionen liefern kann |
| Lkw entfernen | Lädt ein Spielstand, dessen aktiver Lkw ersatzlos entfernt wurde? |
| Laden im Convoy | Bleibt die Verbindung beim Laden stehen? |
| Sicht der anderen | Sehen Mitspieler nach einem Lkw-Wechsel sofort den neuen Lkw? |
| Session-Ende | Welcher Spielstand liegt nach "behalten" oder "zurücksetzen" auf der Platte? |

Solange der erste Test offen ist, sind alle Angaben zum Einsetzen vorläufig.

## Fremde Bausteine

| Projekt | Wofür | Lizenz |
|---|---|---|
| `decrypt_truck` | Spielstände entschlüsseln | MIT, direkt nutzbar |
| `scs-sdk-crates` | Telemetry und Input SDK in Rust | MIT/Apache-2.0, Basis fürs Plugin |
| TS SE Tool | Aufbau der Save-Units nachlesen | Apache-2.0 |
| truck-tools | Schema seit 1.60 nachlesen | GPL-3.0, kein Code übernehmen |
| NMC-SCS-LAUNCHER | Logik für den Lkw-Wechsel nachlesen | ohne Lizenz, kein Code übernehmen |
