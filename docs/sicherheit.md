# Sicherheit und Datenschutz

[← Handbuch](README.md)

- ShotKit arbeitet vollständig lokal: kein Konto, keine Telemetrie, keine eigene Internetverbindung. Einzige
  Ausnahme ist die Website-Schaltfläche im Fenster „Über ShotKit“, die auf Klick den Browser öffnet.
- Aufnahmen werden nur gespeichert, wenn du speicherst oder automatisches Speichern einschaltest.

## Vertrauliche Inhalte unkenntlich machen

> [!CAUTION]
> **Verpixeln ist kein sicherer Schutz für kleine Schrift.** Bei geringer Linienbreite bleibt kleiner Text teils
> lesbar, und verpixelter Text lässt sich mit speziellen Werkzeugen teilweise zurückrechnen. Rechteck und Marker
> decken ebenfalls nichts ab – das Rechteck zeichnet nur einen Rahmen, der Marker ist halbtransparent.

Für Passwörter, Kundendaten, Kontonummern o. Ä.:

1. den Bereich mit **Zuschneiden** (<kbd>C</kbd>) aus dem Bild entfernen, oder
2. ihn gar nicht erst mit aufnehmen.

Eine sichere Schwärzung ist geplant ([#5](https://github.com/BomBastikDE/shotkit/issues/5)).

## Zwischenablage

Kopierte Aufnahmen landen auch im Windows-Zwischenablageverlauf (<kbd>Win</kbd>+<kbd>V</kbd>) und – falls
eingeschaltet – im Cloud-Abgleich mit anderen Geräten. Einträge lassen sich dort einzeln löschen. Eine Option zum
Ausnehmen ist geplant ([#11](https://github.com/BomBastikDE/shotkit/issues/11)).

## Echtheit der EXE

Die `.sha256`-Datei schützt vor Übertragungsfehlern, aber nicht vor einer Manipulation des Releases, da sie am
selben Ort liegt. Signierung und Herkunftsnachweis sind geplant ([#9](https://github.com/BomBastikDE/shotkit/issues/9)).

## Sicherheitslücke melden

Bitte **nicht** als öffentliches Issue, sondern vertraulich – siehe [SECURITY.md](../SECURITY.md).

---

[← Einstellungen](einstellungen.md) · Weiter: [Umstieg von C# →](umstieg.md)
