# Aufnehmen

[← Handbuch](README.md)

## Tastenkürzel

Die Kürzel funktionieren systemweit, auch wenn ShotKit nicht im Vordergrund ist.

| Aufnahme | Tastenkürzel | Ausweichkürzel |
|---|---|---|
| Bereich oder Fenster auswählen | <kbd>Druck</kbd> | <kbd>F11</kbd>, <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>1</kbd> |
| Bildschirm unter dem Mauszeiger | <kbd>Strg</kbd>+<kbd>Druck</kbd> | <kbd>F9</kbd>, <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>2</kbd> |
| Alle Bildschirme | <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>Druck</kbd> | – |
| Aktives Fenster | <kbd>Umschalt</kbd>+<kbd>Druck</kbd> | <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>3</kbd> |
| Scroll-Aufnahme (lange Seiten) | <kbd>Strg</kbd>+<kbd>Alt</kbd>+<kbd>Druck</kbd> | <kbd>F10</kbd>, <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>4</kbd> |

> [!TIP]
> Windows 11 belegt <kbd>Druck</kbd> oft mit dem Snipping Tool. Abschalten unter *Einstellungen →
> Barrierefreiheit → Tastatur → „Druck-Taste zum Öffnen der Bildschirmausschnitterstellung verwenden“* –
> oder einfach <kbd>F11</kbd> verwenden.

Alle Aufnahmearten gibt es auch im Menü (Rechtsklick auf das Symbol im Infobereich).

## Bereich oder Fenster auswählen

Nach <kbd>Druck</kbd> friert der Bildschirm ein und wird abgedunkelt. Eine Lupe zeigt Bildschirmkoordinaten
und Farbwert unter dem Mauszeiger.

| Aktion | Wirkung |
|---|---|
| Ziehen | freien Bereich aufziehen |
| Klicken | umrandeten Vorschlag übernehmen – ein Fenster oder ein Element darin, z. B. der Inhalt einer Webseite |
| <kbd>Strg</kbd> gedrückt halten | statt des Elements das größere, umschließende Fenster vorschlagen |
| <kbd>Esc</kbd> oder Rechtsklick | abbrechen |

### Bereich pixelgenau korrigieren

1. Beim Loslassen eines aufgezogenen Bereichs <kbd>Umschalt</kbd> gedrückt halten.
2. Die Pfeiltasten verschieben den Bereich um je einen Pixel; <kbd>Umschalt</kbd>+Pfeiltaste ändert die rechte
   bzw. untere Kante.
3. <kbd>Enter</kbd> nimmt auf, <kbd>Esc</kbd> bricht ab.

## Weitere Aufnahmen im Menü

- **Bereich nach 3 / 5 / 10 Sekunden auswählen** – Zeit, um vorher ein Menü oder einen Tooltip zu öffnen.
- **Letzten Bereich erneut aufnehmen** – derselbe Ausschnitt wie zuletzt, mit aktuellem Inhalt.
  Gilt nur für die laufende Sitzung; liegt der Bereich nach einer Monitoränderung nicht mehr vollständig auf
  einem Bildschirm, meldet ShotKit das.

## Scroll-Aufnahme

Für Webseiten und Dokumente, die länger oder breiter als der Bildschirm sind.

> [!IMPORTANT]
> Die Scroll-Aufnahme ist experimentell. Ergebnis bitte an den Übergängen prüfen.

1. <kbd>Strg</kbd>+<kbd>Alt</kbd>+<kbd>Druck</kbd> (oder <kbd>F10</kbd>) drücken und den Inhaltsbereich anklicken
   oder aufziehen.
2. Richtung wählen – **nach unten**, **nach rechts** oder **nach links** – und starten.
3. ShotKit scrollt selbst und setzt die Bilder zu einem langen Bild zusammen, bis das Ende erreicht ist.

Vorzeitig beenden: <kbd>Esc</kbd> drücken oder die Maus bewegen. Das bis dahin Aufgenommene bleibt erhalten.

> [!TIP]
> Cookie-Banner und Pop-ups vorher schließen und die Maus während der Aufnahme nicht berühren.

---

[← Installation](installation.md) · Weiter: [Editor →](editor.md)
