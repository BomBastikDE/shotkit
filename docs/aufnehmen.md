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
| <kbd>Enter</kbd> | den umrandeten Vorschlag übernehmen |
| <kbd>Strg</kbd>+Pfeiltaste | Mauszeiger um einen Pixel bewegen (mit <kbd>Umschalt</kbd> um zehn) |
| <kbd>Esc</kbd> oder Rechtsklick | abbrechen |

### Bereich pixelgenau korrigieren

1. Beim Loslassen eines aufgezogenen Bereichs <kbd>Umschalt</kbd> gedrückt halten.
2. Die Pfeiltasten verschieben den Bereich um je einen Pixel; <kbd>Umschalt</kbd>+Pfeiltaste ändert die rechte
   bzw. untere Kante.
3. <kbd>Enter</kbd> nimmt auf, <kbd>Esc</kbd> bricht ab.

Bei einem angezeigten Vorschlag startet schon ein Druck auf eine Pfeiltaste die Korrektur. Bewegst du die Maus,
kehrt ShotKit zur normalen Auswahl zurück.

## Weitere Aufnahmen im Menü

- **Bereich nach 3 / 5 / 10 Sekunden auswählen** – Zeit, um vorher ein Menü oder einen Tooltip zu öffnen.
- **Letzten Bereich erneut aufnehmen** – derselbe Ausschnitt wie zuletzt, mit aktuellem Inhalt.
  Gilt nur für die laufende Sitzung und ist bis zur ersten Bereichsaufnahme ausgegraut; liegt der Bereich nach
  einer Monitoränderung nicht mehr vollständig auf einem Bildschirm, meldet ShotKit das.
- **Alle Bildschirme aufnehmen** und **Aktives Fenster aufnehmen** – wie die entsprechenden Tastenkürzel.

## Scroll-Aufnahme

Für Webseiten und Dokumente, die länger oder breiter als der Bildschirm sind.

> [!IMPORTANT]
> Die Scroll-Aufnahme ist experimentell. Ergebnis bitte an den Übergängen prüfen.

1. <kbd>Strg</kbd>+<kbd>Alt</kbd>+<kbd>Druck</kbd> (oder <kbd>F10</kbd>) drücken.
2. Im Dialog **Scroll-Richtung** wählen: *Ja* = nach unten, *Nein* = nach rechts oder links (dann folgt eine zweite
   Frage), *Abbrechen* = schließen. ShotKit merkt sich die zuletzt gewählte Richtung.
3. Den Inhaltsbereich mit der Maus aufziehen. Er muss mindestens 100 × 100 Pixel groß sein und in Scrollrichtung
   mindestens 200 Pixel lang; sonst erscheint eine Meldung.
4. ShotKit scrollt selbst und setzt die Bilder zu einem langen Bild zusammen, bis das Ende erreicht ist.

Vorzeitig beenden: <kbd>Esc</kbd> drücken oder die Maus bewegen. Das bis dahin Aufgenommene bleibt erhalten; ein
Fenster „Scroll-Aufnahme beendet“ nennt den Grund.

ShotKit kann auch von selbst früher enden, etwa wenn sich ein Bildabschnitt nicht eindeutig zuordnen lässt oder
nicht sicher erkennbar ist, ob der Inhalt am Ende angekommen ist. Das bis dahin Aufgenommene bleibt dann ebenfalls
erhalten. Kann Windows den Mauszeiger nicht im Aufnahmebereich positionieren (etwa weil ein anderes Programm
seine Bewegung begrenzt), nennt die Meldung diesen Grund; es werden keine weiteren Scroll-Eingaben gesendet.

Bei Monitoren mit unterschiedlicher Skalierung kann die Scroll-Aufnahme weiterhin vorzeitig enden; das Teilbild
bleibt erhalten. Eine Prüfung auf echten Monitoren mit gemischter Skalierung steht noch aus. Auch kleine
Mausbewegungen beenden die Aufnahme weiterhin.

### Diagnoseordner

Lässt sich ein Abschnitt nicht zuordnen, legt ShotKit – sofern ein Speicherordner eingestellt ist – im Unterordner
`Diagnose` einen Ordner `Scroll_…` mit den Bildern `vorher.png` und `nachher.png` sowie `info.txt` an. Die Meldung
nennt den Pfad. Die Dateien können vertrauliche Inhalte und Bildschirmkoordinaten enthalten und werden nicht
automatisch gelöscht; nach der Fehlersuche bitte selbst entfernen.

> [!TIP]
> Cookie-Banner und Pop-ups vorher schließen und die Maus während der Aufnahme nicht berühren.

---

[← Installation](installation.md) · Weiter: [Editor →](editor.md)
