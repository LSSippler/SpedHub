# Testdaten

Kleine Spielstände im Textformat (SiiN), abgeleitet aus echten Spielständen von ETS2 1.62 ohne Mods.

| Datei | Inhalt |
|---|---|
| `quelle_mit_lkw.sii` | Profil mit drei eigenen Lkw. Der aktive ist ein getunter Scania S mit 76 Units |
| `ziel_schnellauftrag.sii` | Frisches Profil ohne eigenen Lkw, mitten in einem Schnellauftrag mit Firmen-Lkw und Anhänger |
| `erwartet_nach_einsetzen.sii` | Das Ergebnis von `tools/transplant.py` mit den beiden Dateien oben |
| `nach_spiel_neues_kennzeichen.sii` | `erwartet_nach_einsetzen.sii`, so abgewandelt, wie es das Spiel beim Speichern getan hat: alle `_nameless`-Namen neu, statt `SPED 001` ein Kennzeichen der Heimatstadt mit Markup (`EF XU 577`), 425 km mehr, halber Tank |
| `ziel_info.sii` | Die `info.sii` zu `ziel_schnellauftrag.sii`, mit demselben Kontostand |

Die Dateien sind verkleinert: Firmen, Auftragsangebote, fremde Garagen und lange Listen fehlen. Der Aufbau der übrigen Units ist unverändert. Jede Unit ist von `economy` aus erreichbar, und kein Verweis zeigt ins Leere. Selbst vergebene Kennzeichen sind ersetzt.

Das Spiel kann diese Dateien nicht laden. Sie sind nur für automatische Tests gedacht.
