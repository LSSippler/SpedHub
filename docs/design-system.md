# Design-System

Gilt für das Dashboard (`server/web/index.html`) und den Launcher (`crates/spedhub-launcher`). Beide sehen aus wie eine Dispositions-Software einer Spedition: Navigation links, oben die Kennzahlen, darunter Arbeitslisten mit klaren Status. Ruhig, dicht, gut lesbar, keine Spielerei.

Im Dashboard stehen alle Werte als CSS-Variablen an einer Stelle (`:root`), im Launcher in `src/theme.rs`. Beide Stellen tragen dieselben Namen und Werte. Feste Farben, Größen oder Abstände außerhalb dieser Stellen gibt es nicht.

## Grundsätze

- **Zuerst der Zustand, dann die Aktion.** Jede Zeile zeigt auf einen Blick, wie es dem Lkw, Fahrer oder Geld geht (Plakette, Balken, Zahl). Aktionen kommen danach.
- **Eine Hauptaktion je Element.** Was am häufigsten gebraucht wird, ist ein Knopf. Alles andere steckt im Menü "Mehr".
- **Zahlen sind Hauptsache.** Geld und Kilometer stehen rechtsbündig in Ziffern gleicher Breite, Beträge im Minus in der Farbe für Gefahr.
- **Farbe heißt Bedeutung.** Grün frei oder gut, Bernstein unterwegs oder Achtung, Rot Gefahr oder Minus, Blau Aktion. Farbe nie als einziges Merkmal, immer mit Text oder Symbol.
- **Ruhige Flächen.** Weiße Karten auf hellgrauem Grund, feine Linien, wenig Schatten. Keine Verläufe, keine Bilder fremder Marken.
- Texte bleiben wie in `CLAUDE.md`: deutsch, schlicht, keine Großschreibung ganzer Wörter (auch nicht per CSS `text-transform`), keine Gedankenstriche.

## Farben

| Token | Hell | Dunkel | Wofür |
|---|---|---|---|
| `--bg` | `#f2f4f7` | `#0b1220` | Seitengrund |
| `--surface` | `#ffffff` | `#121b2c` | Karten, Tabellen, Dialoge |
| `--surface-2` | `#f8fafc` | `#0f1727` | Kopf von Tabellen, eingelassene Flächen |
| `--border` | `#e3e8ef` | `#223049` | Linien, Ränder von Karten |
| `--border-strong` | `#cbd5e1` | `#33425e` | Ränder von Eingaben |
| `--text` | `#0f172a` | `#e5eaf2` | Text |
| `--text-2` | `#475569` | `#a3b1c6` | Nebentext, Beschriftungen |
| `--text-3` | `#64748b` | `#8191a8` | Hinweise, leere Zustände |
| `--primary` | `#1d4ed8` | `#5b8def` | Hauptaktion, Links, Auswahl |
| `--primary-hover` | `#1e40af` | `#7aa3f5` | |
| `--on-primary` | `#ffffff` | `#0b1220` | Schrift auf `--primary` und `--danger` (Knöpfe) |
| `--primary-soft` | `#e6eefc` | `#17284a` | Hintergrund ausgewählter Zeilen, aktive Navigation |
| `--accent` | `#ea7a1a` | `#f0953f` | Signalfarbe der Marke: Markierung der aktiven Navigation, Logo |
| `--success` | `#166534` | `#4ade80` | frei, bezahlt, online, Plus |
| `--success-soft` | `#e3f6ea` | `#123423` | |
| `--warning` | `#a34a06` | `#fbbf24` | unterwegs, verliehen, Achtung |
| `--warning-soft` | `#fdf1dc` | `#3a2a0c` | |
| `--danger` | `#b91c1c` | `#f87171` | Werkstatt, Minus, Fehler |
| `--danger-soft` | `#fde7e7` | `#3b1515` | |
| `--info` | `#0e7490` | `#38bdf8` | neutrale Hinweise |
| `--info-soft` | `#e0f3f8` | `#0d2c3a` | |
| `--nav-bg` | `#0f1e33` | `#0a1424` | Seitenleiste |
| `--nav-text` | `#c3cede` | `#aab7ca` | |
| `--nav-active` | `#ffffff` | `#ffffff` | |
| `--nav-active-bg` | `#1b3150` | `#162843` | |

Nur im Dashboard gibt es dazu:

| Token | Hell | Dunkel | Wofür |
|---|---|---|---|
| `--danger-hover` | `#991b1b` | `#fca5a5` | Hover auf Gefahr-Knöpfen |
| `--nav-hover-bg` | `#16283f` | `#111f35` | Hover in der Seitenleiste |
| `--nav-line` | `#22344f` | `#1b2a42` | Trennlinie unten in der Seitenleiste |
| `--plate-bg`, `--plate-text`, `--plate-border` | `#ffffff`, `#0f172a`, `#0f172a` | gleich | Kennzeichen, bleibt auch im Dunkelmodus ein weißes Schild |
| `--plate-eu`, `--plate-eu-text` | `#1d4ed8`, `#ffffff` | gleich | blauer Streifen mit "D" |
| `--overlay` | `rgb(15 23 42 / 45%)` | `rgb(0 0 0 / 60%)` | abgedunkelter Hintergrund hinter Dialogen und dem Menü auf dem Telefon |

Maße, die keine Abstände sind (Breite der Seitenleiste, Höhe von Knöpfen und Zeilen, Größe von Symbolen, Mindestbreite der Karten, Breite von Dialog und Meldung), stehen im Dashboard ebenfalls als Variablen in `:root`.

Text auf Flächen hat mindestens 4,5:1 Kontrast, auch in den Plaketten (kräftige Farbe auf weicher Farbe) und auf Knöpfen. Im Dunkelmodus ist die Schrift auf Knöpfen deshalb dunkel.

Der Dunkelmodus folgt der Systemeinstellung (`prefers-color-scheme`). Im Dashboard lässt er sich zusätzlich über einen Schalter in der Seitenleiste festlegen (`data-theme="light"` oder `"dark"` am Wurzelelement, im Browser gemerkt).

## Schrift

Systemschrift, keine Webfonts: `"Segoe UI", system-ui, -apple-system, Roboto, "Helvetica Neue", Arial, sans-serif`. Zahlen immer mit `font-variant-numeric: tabular-nums`.

| Token | Größe / Zeilenhöhe | Gewicht | Wofür |
|---|---|---|---|
| `--fs-xs` | 12 / 16 px | 500 | Plaketten, Achsen, Fußnoten |
| `--fs-sm` | 13 / 18 px | 400 | Nebentext, Tabellenköpfe |
| `--fs-base` | 14 / 20 px | 400 | Fließtext, Tabellen, Eingaben |
| `--fs-md` | 16 / 24 px | 600 | Titel von Karten |
| `--fs-lg` | 20 / 28 px | 600 | Seitentitel |
| `--fs-xl` | 28 / 34 px | 700 | Kennzahlen |

## Abstände, Radien, Schatten, Bewegung

- Abstände in Schritten von 4 px: `--sp-1` 4, `--sp-2` 8, `--sp-3` 12, `--sp-4` 16, `--sp-5` 20, `--sp-6` 24, `--sp-8` 32, `--sp-10` 40.
- Radien: `--r-sm` 6 px (Knöpfe, Eingaben), `--r-md` 10 px (Karten, Dialoge), `--r-pill` 999 px (Plaketten).
- Schatten: `--shadow-sm` `0 1px 2px rgb(15 23 42 / 6%)` für Karten, `--shadow-md` `0 4px 12px rgb(15 23 42 / 10%)` für Menüs, `--shadow-lg` `0 16px 40px rgb(15 23 42 / 18%)` für Dialoge. Im Dunkelmodus dunkler und mit Rand statt Schatten.
- Bewegung: `--dur-fast` 120 ms, `--dur` 200 ms, `ease-out`. Bei `prefers-reduced-motion` keine Bewegung.

## Aufbau

- **Seitenleiste** (240 px, `--nav-bg`, Fokus-Ring dort in `--nav-active`, weil `--primary` auf dem dunklen Grund kaum zu sehen ist):
  - Oben das Logo: ein kleines Lkw-Symbol in `--accent` und der Firmenname.
  - Darunter die Bereiche mit Symbol: Übersicht, Fuhrpark, Fahrer, Kassenbuch, Büro (nur Chef), Testbereich.
  - Der aktive Bereich hat `--nav-active-bg` und links eine 3 px breite Leiste in `--accent`.
  - Unten: angemeldeter Nutzer mit Initialen-Kreis, Rolle, Menü (Passwort ändern, Firma wechseln, Abmelden) und der Schalter hell/dunkel.
- **Kopfzeile des Bereichs:** Seitentitel (`--fs-lg`), eine Zeile Nebentext und rechts die Hauptaktion des Bereichs (etwa "Lkw anlegen").
- **Inhalt:** höchstens 1280 px breit, Abstand `--sp-6`, Karten im Raster mit `--sp-4` Lücke.
- **Schmal (unter 900 px):** Die Seitenleiste wird zur oberen Leiste mit Menüknopf, Inhalte einspaltig, Tabellen rollen in sich. Die Seite selbst rollt nie waagerecht.
- Die Bereiche sind Ansichten derselben Seite (Adresse `#/fuhrpark` usw.), zurück und vorwärts im Browser funktionieren.

## Bausteine

### Knopf
| Art | Aussehen | Wofür |
|---|---|---|
| primär | `--primary`, Schrift `--on-primary` | eine Hauptaktion je Bereich oder Karte |
| sekundär | `--surface`, Rand `--border-strong`, Schrift `--text` | weitere Aktionen |
| leise | ohne Rand, Schrift `--primary` | Aktionen in Listen, "Abbrechen" |
| Gefahr | `--danger`, Schrift `--on-primary` | Stornieren, Ausmustern, Verkaufen (immer mit Bestätigung) |
| Symbol | 32 × 32 px, nur Symbol mit `aria-label` | Menü "Mehr", Schließen |

Höhe 36 px (klein 30 px), Radius `--r-sm`, Schrift 14 px Gewicht 600. Zustände: Hover etwas dunkler, Fokus 2 px Ring in `--primary` mit 2 px Abstand, deaktiviert 50 % Deckkraft, beim Laden ein kleiner Kreisel statt Symbol und kein zweiter Klick.

### Menü "Mehr"
Symbolknopf mit drei Punkten. Darunter klappt eine Liste mit `--shadow-md` auf. Bedienbar mit Tastatur: Pfeile, Enter, Escape. Einträge für Gefahr stehen in `--danger` und unten, abgetrennt.

### Plakette (Status)
Pille mit `--fs-xs`, Hintergrund in der weichen Farbe, Schrift und ein Punkt von 6 px in der kräftigen Farbe.

| Status | Farbe |
|---|---|
| frei, online, bezahlt | success |
| unterwegs, verliehen, wird gefahren | warning |
| in der Werkstatt, abgelehnt, ungeklärt, offen | danger |
| ausgemustert, verkauft, offline | neutral (`--surface-2`, `--text-2`) |
| Chef, Hinweis | info |

### Kennzahl-Kachel
Eine Karte mit Symbol in einem weichen Farbfeld (32 px) und einer Beschriftung in `--fs-sm` `--text-2`. Darunter der Wert in `--fs-xl` mit `tabular-nums`, darunter eine Zeile Zusatz in `--fs-xs`, etwa "3 von 5 Lkw im Einsatz".

Kacheln der Übersicht:
- Firmenkasse
- Lkw im Einsatz
- Fahrer online
- Erlös der letzten 7 Tage
- Offene Punkte: Werkstatt, ungeklärt, abgelehnt

### Kennzeichen
Das bestehende Kennzeichen bleibt das Erkennungszeichen eines Lkw, sauberer gezeichnet:
- weißes Schild, 2 px Rand
- blauer Streifen mit "D"
- 600er Schrift mit Ziffern gleicher Breite
- Größen klein (Tabellen) und groß (Karten)

### Lkw-Karte (Fuhrpark)
- **Kopf:**
  - Oben das Kennzeichen (groß), rechts daneben die Plakette des Status.
  - Darunter über die volle Breite Marke und Modell lesbar ("Scania S 2016", `--fs-md`, nie umbrechend, sonst gekürzt mit Titel-Tooltip), darunter klein der Name aus dem Spiel und frühere Kennzeichen. Neben Kennzeichen und Plakette blieben bei 320 px Breite für das Modell nur gut 100 px.
- **Mitte, drei Kennwerte in einer Reihe:**
  - Kilometerstand
  - Schaden als Balken: grün unter 10 %, Bernstein bis zur Grenze, rot darüber, dazu die Zahl. Darunter "dauerhaft x %" in `--text-3`.
  - Tank, falls bekannt. Die Liste der Lkw liefert keinen Tank, das Dashboard zeigt an dieser Stelle "Letzte Meldung": Zeit und Kilometerstand des letzten Herzschlags, "jetzt" während der Fahrt.
- **Fahrer-Zeile, wenn verliehen:**
  - Initialen-Kreis, Name, "seit 10:02".
  - Fährt der Lkw gerade: "84 km/h, Bremen nach Hannover" mit Symbol in `--warning`.
- **Fuß:**
  - Genau eine Hauptaktion: frei "Ausleihen", eigener "Zurückgeben", Chef bei fremder Leihe "Zurücknehmen", Werkstatt beim Chef "Reparieren, 3.870 €".
  - Alles Weitere im Menü "Mehr": beim Chef "Reparieren lassen" für einen freien Lkw mit Schaden, Kennzeichen ändern (mit dem Vorschlag des Servers), Verkaufen, Ausmustern.
- **Raster:** Karten mit mindestens 320 px Breite, so viele Spalten wie passen. Darüber Filter-Pillen: Alle, Frei, Unterwegs, Werkstatt, mit Anzahl.
- **Ausgemusterte und verkaufte Lkw:** in einem eingeklappten Bereich darunter, gedämpft.

### Tabelle
- Weiße Karte, Kopfzeile `--surface-2` mit `--fs-sm` `--text-2` Gewicht 600.
- Zeilen 44 px, Trennlinien `--border`, Hover `--surface-2`.
- Zahlen rechtsbündig.
- Ausgewählte Zeilen `--primary-soft`.
- Lange Texte kürzen statt umbrechen, wo es geht, mit Tooltip.

### Kassenbuch
- **Kopf:**
  - Kennzahlen: Firmenkasse, eigenes Lohnkonto, Saldo der gezeigten Buchungen (Einnahmen, Ausgaben).
  - Filter: Zeitraum mit Schnellwahl (Heute, 7 Tage, 30 Tage, Alle) und Datumsfeldern, dazu Art.
  - Die Auswahlleiste erscheint nur, wenn etwas ausgewählt ist, schwebend unten: "3 ausgewählt", "Stornieren".
- **Zeilen:**
  - Datum und Uhrzeit.
  - Ein Symbol je Art: Auftrag, Maut, Fähre, Zug, Strafe, Lohn, Betriebskosten, Werkstatt, Überweisung, Storno.
  - Titel und Nebenzeile: Fahrer, Fracht, von nach, km.
  - Betrag Kasse und Betrag Lohnkonto, rechtsbündig, Plus grün mit "+", Minus rot.
- **Stornierte Zeilen:** gedämpft, durchgestrichen, mit Plakette "storniert" und Tooltip mit Grund.
- **Lohnabrechnungen:** als aufklappbare Gruppe.
- **Schmal (unter 700 px):** Jede Buchung wird ein Block: oben Zeit, darunter Titel und Nebenzeile, rechts die Beträge mit kleiner Beschriftung "Kasse" oder "Lohnkonto". So bleiben die Beträge sichtbar, statt aus der Tabelle zu rollen.

### Formular
- Beschriftung über dem Feld in `--fs-sm` Gewicht 600.
- Feld 36 px hoch mit Rand `--border-strong`, Fokus mit Ring in `--primary`.
- Hilfetext darunter in `--text-3`, Fehler darunter in `--danger` mit Symbol.
- Formulare im Büro als Karten mit Titel, kurzem Satz und Feldern im Raster. Die Speichern-Knöpfe stehen unten rechts.

### Dialog
- Mittig, höchstens 480 px breit, `--r-md`, `--shadow-lg`, Hintergrund abgedunkelt.
- Titel, Text, Felder, dann Knöpfe rechts: Abbrechen leise, Bestätigen primär oder Gefahr.
- Escape schließt, der Fokus bleibt im Dialog.

### Meldung (Toast)
- Unten rechts, Karte mit farbigem Rand links und Symbol: Erfolg, Hinweis, Fehler.
- Verschwindet nach 4 Sekunden, Fehler erst nach 8.
- `role="status"`, Fehler mit `role="alert"`.

### Leerer Zustand
Symbol in `--text-3`, ein Satz, was fehlt, und, wenn möglich, die Aktion dazu, etwa "Noch kein Lkw im Fuhrpark. Leg den ersten an." mit dem Knopf "Lkw anlegen".

### Symbole
Eigene, einfache Liniensymbole als Inline-SVG (24er Raster, 1,75 px Strich, `currentColor`), an einer Stelle im Code gesammelt:
- Lkw, Übersicht, Fahrer, Kassenbuch, Büro, Testbereich
- Geld, Werkzeug, Uhr, Ort, Pfeil, Plus, Minus
- Mehr, Schließen, Haken, Warnung, Fähre, Zug, Maut, Strafe, Storno, Mond, Sonne

Keine Logos von Lkw-Herstellern oder SCS.

## Launcher

Der Launcher nutzt dieselben Farben (`src/theme.rs`, Hell und Dunkel nach Systemeinstellung) und dieselben Bausteine, so weit egui sie hergibt:

- **Aufbau:**
  - Links eine schmale Seitenleiste in `--nav-bg` mit Logo, Bereichen (Fuhrpark, Geld und Spielstand, Übernehmen nur Chef, Einstellungen) und unten Nutzer, Version und Server.
  - Oben im Inhalt Kennzahl-Kacheln: Lohnkonto, Firmenkasse, Plugin-Status, Firmenprofil.
- **Fuhrpark:** Lkw als Karten nach demselben Muster wie im Dashboard, mit Kennzeichen, Plakette, Balken für den Schaden und einer Hauptaktion. Karten sind mindestens 380 px breit, weil die eigene Leihe zwei Knöpfe nebeneinander hat (Zurückgeben, In Spielstand schreiben) und egui die Reihe nicht umbricht. Modellnamen folgen denselben Regeln wie im Dashboard (`theme::model_name` und `modellName`).
- **Hinweise:** Warnungen (Steam Cloud, Version, Plugin) als farbige Hinweiskarten mit Symbol statt farbigem Text.
- **Text und Bedienung:** Systemschrift, Abstände nach demselben Raster, Knöpfe 32 px hoch mit `--r-sm`.
