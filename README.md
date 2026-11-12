# ShotKit

Screenshots aufnehmen, markieren und sicher schwärzen – für Windows 10 und 11, mit deutscher Oberfläche.
ShotKit ist eine einzige EXE ohne Installation, in Rust geschrieben und ohne Telemetrie.

> **Aktuelle Version: 2.1.1** · [Download](https://github.com/BomBastikDE/shotkit/releases/latest) ·
> [Benutzerhandbuch](docs/README.md) · [Was ist neu?](rust/CHANGELOG.md)

## Was ShotKit kann

**Aufnehmen**

- Bereich, Fenster, einen oder alle Bildschirme – per Tastenkürzel von überall
- Fenster und Bedienelemente werden beim Auswählen vorgeschlagen; eine Lupe zeigt Koordinaten und Farbwert
- Scroll-Aufnahme für lange Seiten, nach unten, rechts oder links
- Verzögerte Aufnahme nach 3, 5 oder 10 Sekunden und „Letzten Bereich erneut aufnehmen“

**Bearbeiten**

- Pfeil, Linie, Rechteck, Ellipse, Text, Sprechblase, Marker, Stift, Schritt-Nummern und Stempel
- Spotlight und Bildlupe, um Stellen hervorzuheben
- Zuschneiden, Rahmen, Schatten, Leinwandgröße und Rissrand
- Alle Objekte bleiben verschiebbar und änderbar; Rückgängig und Wiederholen
- Mehrere Bilder in getrennten Fenstern; ein Bild lässt sich als schwebendes Fenster anheften

**Weitergeben**

- Kopieren in die Zwischenablage, Speichern als PNG, JPG, BMP oder GIF
- Auf Wunsch automatisch kopieren und speichern, direkt nach der Aufnahme
- **Schwärzen** ersetzt die Bildpunkte deckend durch Schwarz. Verpixeln und Weichzeichnen gibt es auch, sie gelten
  aber ausdrücklich nicht als sicher – siehe [Sicherheit und Datenschutz](docs/sicherheit.md)

## Schnellstart

1. Die neueste EXE (z. B. `shotkit-v2.1.1-windows-x64.exe`) von der
   [Releases-Seite](https://github.com/BomBastikDE/shotkit/releases/latest) laden und starten.
   ShotKit erscheint als Symbol im Infobereich der Taskleiste.
2. <kbd>Druck</kbd> drücken und einen Bereich aufziehen oder ein Fenster anklicken.
3. Im Editor markieren, Vertrauliches mit **Schwärzen** (<kbd>S</kbd>) abdecken, dann mit
   <kbd>Strg</kbd>+<kbd>C</kbd> kopieren oder mit <kbd>Strg</kbd>+<kbd>S</kbd> speichern.

| Aufnahme | Tastenkürzel | Ausweichkürzel |
|---|---|---|
| Bereich oder Fenster | <kbd>Druck</kbd> | <kbd>F11</kbd> |
| Bildschirm unter dem Mauszeiger | <kbd>Strg</kbd>+<kbd>Druck</kbd> | <kbd>F9</kbd> |
| Alle Bildschirme | <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>Druck</kbd> | – |
| Aktives Fenster | <kbd>Umschalt</kbd>+<kbd>Druck</kbd> | – |
| Scroll-Aufnahme | <kbd>Strg</kbd>+<kbd>Alt</kbd>+<kbd>Druck</kbd> | <kbd>F10</kbd> |

Alle Kürzel, auch die im Editor, stehen im Handbuch unter [Aufnehmen](docs/aufnehmen.md) und [Editor](docs/editor.md).

## Download prüfen

Zu jeder EXE gehört eine `.sha256`-Datei. Prüfsumme vergleichen:

```powershell
Get-FileHash .\shotkit-*.exe -Algorithm SHA256
```

Die EXE ist nicht signiert; Windows SmartScreen kann warnen („Weitere Informationen → Trotzdem ausführen“).
Eine Signierung ist derzeit nicht geplant.

## Dokumentation

- **[Benutzerhandbuch](docs/README.md)** – Installation, Aufnehmen, Editor, Einstellungen, Sicherheit, häufige Fragen
- [Änderungsprotokoll](rust/CHANGELOG.md) und [Ausblick](rust/ROADMAP.md)
- Für Entwickler: [technische Übersicht, Bauen und Testen](rust/README.md) · [Testprotokoll](rust/TESTPROTOKOLL.md) ·
  [Entwicklungsrichtlinien](CONTRIBUTING.md)

## Mitmachen und Fehler melden

Fehler und Wünsche bitte als [Issue](https://github.com/BomBastikDE/shotkit/issues) melden.
Sicherheitslücken bitte vertraulich, siehe [SECURITY.md](SECURITY.md).

## Lizenz

MIT, siehe [LICENSE](LICENSE).
