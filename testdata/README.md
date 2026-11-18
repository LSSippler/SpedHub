# Testdaten

Kleine Spielstände im Textformat (SiiN), abgeleitet aus echten Spielständen von ETS2 1.62 ohne Mods, und eine selbst geschriebene Datei für die Wirtschafts-Mod.

| Datei | Inhalt |
|---|---|
| `quelle_mit_lkw.sii` | Profil mit drei eigenen Lkw. Der aktive ist ein getunter Scania S mit 76 Units |
| `ziel_schnellauftrag.sii` | Frisches Profil ohne eigenen Lkw, mitten in einem Schnellauftrag mit Firmen-Lkw und Anhänger |
| `erwartet_nach_einsetzen.sii` | Das Ergebnis von `tools/transplant.py` mit den beiden Dateien oben |
| `nach_spiel_neues_kennzeichen.sii` | `erwartet_nach_einsetzen.sii`, so abgewandelt, wie es das Spiel beim Speichern getan hat: alle `_nameless`-Namen neu, statt `SPED 001` ein Kennzeichen der Heimatstadt mit Markup (`EF XU 577`), 425 km mehr, halber Tank |
| `ziel_info.sii` | Die `info.sii` zu `ziel_schnellauftrag.sii`, mit demselben Kontostand |
| `economy_beispiel.sii` | Selbst geschrieben, keine Datei von SCS: eine Datei im Format von `def/economy_data.sii` für `spedhub-save economy-mod`. Nur die Schlüsselnamen der Erlös-Werte sind übernommen, Aufbau und Zahlen sind ausgedacht. Mit Kommentaren, `@include`, Liste, Hex-Zahl, Vektor und Kandidaten |
| `economy_beispiel_erwartet.sii` | `economy_beispiel.sii` nach `economy-mod` mit Faktor 0.01, von Hand geprüft: nur sechs Zahlen sind anders |

Die Spielstände sind verkleinert: Firmen, Auftragsangebote, fremde Garagen und lange Listen fehlen. Der Aufbau der übrigen Units ist unverändert. Jede Unit ist von `economy` aus erreichbar, und kein Verweis zeigt ins Leere. Selbst vergebene Kennzeichen sind ersetzt.

Das Spiel kann diese Dateien nicht laden. Sie sind nur für automatische Tests gedacht.
