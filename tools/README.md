# Werkzeuge für den Spielstand

Zwei Hilfsprogramme aus dem ersten Lkw-Test. Sie sind der Keim für die spätere Save-Bibliothek, noch kein fertiges Werkzeug.

## siidec: Spielstand in Text umwandeln

Liest `game.sii` oder `info.sii` in jedem Format (verschlüsselt, binär, Text) und schreibt Klartext. Nutzt die Bibliothek `decrypt_truck` (MIT).

```powershell
cd C:\dev\SpedHub
cargo build --release -p siidec
.\target\release\siidec.exe "C:\Pfad\zu\game.sii" quelle.sii
```

## transplant.py: Lkw von einem Spielstand in einen anderen setzen

Nimmt den aktiven Lkw aus dem Quell-Spielstand und setzt ihn als eigenen, aktiven Lkw ins Ziel. Ein laufender Schnellauftrag im Ziel wird dabei entfernt. Beide Eingaben müssen Klartext sein (siehe siidec).

```powershell
cd C:\dev\SpedHub
python tools\transplant.py quelle.sii ziel.sii ergebnis.sii
```

Das Skript prüft vor dem Schreiben, dass alle Verweise auflösen und keine Unit verwaist ist. Getestet ist es mit Spielständen aus ETS2 1.62 ohne Mods, mit einem frischen Zielprofil ohne eigenen Lkw. Ob das Spiel das Ergebnis lädt, ist noch nicht bestätigt.

Das Ergebnis kommt als `game.sii` in einen neuen Ordner unter `profiles\<Profil>\save\`, zusammen mit einer `info.sii` aus dem Zielprofil.
