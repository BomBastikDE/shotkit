# Editor

[← Handbuch](README.md)

Nach einer Aufnahme öffnet sich der Editor (abschaltbar, siehe [Einstellungen](einstellungen.md)). Jedes Bild
bekommt ein eigenes Fenster; mehrere Bilder lassen sich parallel bearbeiten. Alle gezeichneten Objekte bleiben
bis zum Speichern einzeln auswählbar und änderbar. Fährst du mit der Maus über ein Symbol, nennt ShotKit seinen
Namen und gegebenenfalls das Tastenkürzel.

Der Fenstertitel zeigt den geöffneten Dateipfad und das aktive Werkzeug mit seinem Namen aus der Werkzeugleiste.
Ein Stern vor dem Dateipfad kennzeichnet ungespeicherte Änderungen; beides bleibt beim Werkzeugwechsel sichtbar.

## Werkzeuge

| Werkzeug | Taste | Bedienung |
|---|---|---|
| Auswahl | <kbd>V</kbd> | Objekt anklicken, verschieben, an Griffpunkten ändern; <kbd>Entf</kbd> oder <kbd>Rücktaste</kbd> löscht, Pfeiltasten verschieben (mit <kbd>Umschalt</kbd> um zehn Pixel) |
| Pfeil | <kbd>A</kbd> | ziehen; mit <kbd>Umschalt</kbd> in 45°-Schritten |
| Linie | <kbd>L</kbd> | ziehen; mit <kbd>Umschalt</kbd> in 45°-Schritten |
| Rechteck | <kbd>R</kbd> | Rahmen ziehen; mit <kbd>Umschalt</kbd> quadratisch |
| Ellipse | <kbd>E</kbd> | ziehen; mit <kbd>Umschalt</kbd> kreisrund |
| Text | <kbd>T</kbd> | klicken und Text eingeben; Doppelklick auf vorhandenen Text zum Ändern |
| Sprechblase | – | mehrzeiligen Text in einem abgerundeten Kasten platzieren; Doppelklick bearbeitet den Text, die Spitze lässt sich am Griff verschieben |
| Marker | <kbd>M</kbd> | Bereich halbtransparent farbig hervorheben; mit <kbd>Umschalt</kbd> quadratisch |
| Verpixeln | <kbd>B</kbd> | Bereich vergröbern – **keine sichere Schwärzung**; für vertrauliche Daten Schwärzen (<kbd>S</kbd>) verwenden, siehe [Sicherheit](sicherheit.md) |
| Weichzeichnen | <kbd>W</kbd> | Bereich unscharf machen; die Stärke folgt der Linienbreite – **keine sichere Schwärzung** |
| Schwärzen | <kbd>S</kbd> | Bereich deckend schwarz abdecken – die sichere Methode, siehe [Sicherheit](sicherheit.md) |
| Nummer | <kbd>N</kbd> | per Klick fortlaufende Schritt-Nummern setzen |
| Stempel | – | Symbol wählen (Haken, Kreuz, Warnung, Information, Mauszeiger) und per Klick platzieren; über die Eckpunkte oder die Schriftgröße skalieren |
| Stift | <kbd>P</kbd> | freihand zeichnen |
| Zuschneiden | <kbd>C</kbd> | Bereich aufziehen, auf den das Bild zugeschnitten wird |
| Spotlight | – | Bereich aufziehen, der hell bleibt; der Rest wird abgedunkelt (Stärke 25, 50, 75 oder 90 % unter **Formstil…**) |
| Bildlupe | – | Bildausschnitt aufziehen und vergrößert anzeigen (1,5× bis 4×, rund/oval oder rechteckig, unter **Formstil…**) |

Stempel, Spotlight, Sprechblase und Bildlupe lassen sich wie andere Objekte verschieben, an den Griffen ändern und
rückgängig machen. Die Bildlupe zeigt das Bild nach allen Schwärzungen; andere Anmerkungen werden nicht mit
vergrößert. Mit <kbd>Umschalt</kbd> zeichnen auch Verpixeln, Weichzeichnen und Schwärzen quadratisch.

## Werkzeugleiste

- **Farbe** – Schnellwahl Rot, Schwarz, Gelb, Blau oder frei wählbar
- **Formstil…** – für neue und das gerade ausgewählte Objekt:
  - Füllung: keine, deckend, halbtransparent
  - Linie: durchgezogen, gestrichelt, gepunktet
  - Rechteck-Ecken: eckig oder abgerundet
  - Pfeilenden: einfach, doppelt oder mit Punkt
  - Sprechblase: Füllfarbe, Textfarbe, Spitze ein/aus
  - Spotlight: Abdunklung; Bildlupe: Vergrößerung und Form
- **Linienbreite…** und **Schriftgröße…** – gelten für neue Objekte und für das gerade ausgewählte
- **Rahmen / Schatten / Leinwand…** – Rahmen und/oder Schlagschatten um das ganze Bild oder **Leinwandgröße ändern…**
- **Neue Aufnahme**, **Kopieren**, **Speichern**, **Zurück**, **Wiederholen**
- **Ansicht** – Vergrößern, Verkleinern, Originalgröße, Einpassen
- **Datei** – Speichern, Speichern unter …, Öffnen …, Bild aus Zwischenablage einfügen, Speicherordner öffnen

Ist das Fenster schmal, rücken weitere Symbole in das Menü **»** am rechten Rand.

### Leinwandgröße ändern

Unter **Rahmen / Schatten / Leinwand…** → **Leinwandgröße ändern…** stellst du Breite und Höhe in Pixel ein
(1 bis 30.000 je Achse), die Ausrichtung des Bildes (links/Mitte/rechts, oben/Mitte/unten) und die neue Fläche:
transparent oder einfarbig (Farbe als `#RRGGBB`). Das Bild wird nicht skaliert; eine kleinere Leinwand schneidet
Bildbereiche ab. Die Änderung lässt sich rückgängig machen. Sehr große Bilder können abgelehnt werden; das Bild bleibt dann unverändert.

## Tastenkürzel

| Kürzel | Funktion |
|---|---|
| <kbd>Strg</kbd>+<kbd>Z</kbd> | Rückgängig |
| <kbd>Strg</kbd>+<kbd>Y</kbd> oder <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>Z</kbd> | Wiederholen |
| <kbd>Strg</kbd>+<kbd>C</kbd> | Bild in die Zwischenablage kopieren |
| <kbd>Strg</kbd>+<kbd>S</kbd> | Speichern |
| <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>S</kbd> | Speichern unter (PNG, JPG, BMP, GIF) |
| <kbd>Strg</kbd>+<kbd>O</kbd> | Bild öffnen (zusätzlich TIFF) |
| <kbd>Strg</kbd>+<kbd>V</kbd> | Bild aus der Zwischenablage in einem neuen Editor öffnen |
| <kbd>Strg</kbd>+<kbd>N</kbd> | Neue Aufnahme |
| <kbd>Strg</kbd>+Mausrad, <kbd>Strg</kbd>+<kbd>+</kbd> / <kbd>Strg</kbd>+<kbd>-</kbd> | Zoomen (auch mit dem Ziffernblock) |
| <kbd>Strg</kbd>+<kbd>0</kbd> / <kbd>Strg</kbd>+<kbd>9</kbd> | Originalgröße / Einpassen (auch mit dem Ziffernblock) |
| Mausrad / <kbd>Umschalt</kbd>+Mausrad | senkrecht / waagerecht verschieben |
| <kbd>Entf</kbd> oder <kbd>Rücktaste</kbd> | ausgewähltes Objekt löschen |
| Pfeiltasten (mit <kbd>Umschalt</kbd>: zehn Pixel) | ausgewähltes Objekt verschieben |
| <kbd>Esc</kbd> | Auswahl aufheben; sonst laufende Aktion abbrechen; sonst zurück zum Auswahlwerkzeug |

## Rückgängig-Verlauf

Der Verlauf hat ein Speicherlimit pro Bild. Wird es bei einer Bearbeitung überschritten, fragt ShotKit vorher:

- ob ältere Rückgängig-/Wiederholen-Schritte dafür entfernt werden sollen, oder
- ob der gesamte Verlauf gelöscht werden soll, wenn selbst die einzelne Bearbeitung nicht hineinpasst – sie ist
  dann nicht mehr rückgängig zu machen.

Bei **Nein** bleiben Bild und Verlauf unverändert.

Passt ein vollständiger Verlauf bei Rückgängig oder Wiederholen nicht ins Limit, lehnt ShotKit den Schritt mit einer
Meldung ab; Bild und Verlauf bleiben erhalten.

## Speichern und Öffnen

- Zum Speichern stehen PNG, JPG, BMP und GIF zur Verfügung. Fehlt die Endung, ergänzt ShotKit die gewählte.
  Vor dem Ersetzen einer vorhandenen Datei fragt ShotKit nach.
- Geöffnete JPEG-, GIF- und TIFF-Bilder schützt ShotKit beim ersten <kbd>Strg</kbd>+<kbd>S</kbd>: Nach Bestätigung
  ist ein anderer Dateiname nötig. JPG und GIF behalten Metadaten bzw. Animationen nicht vollständig; die
  EXIF-Ausrichtung wird beim Öffnen berücksichtigt.
- Sehr große Bilder können beim Öffnen, Zeichnen, Kopieren oder Speichern abgelehnt werden. Das Bild im Editor bleibt
  erhalten, eine vorhandene Zieldatei wird nicht verändert.

> [!NOTE]
> Vor dem Schließen oder Ersetzen eines ungespeicherten Bildes fragt ShotKit nach. Transparenz bleibt beim
> Speichern als PNG und beim Kopieren erhalten.

---

[← Aufnehmen](aufnehmen.md) · Weiter: [Einstellungen →](einstellungen.md)
