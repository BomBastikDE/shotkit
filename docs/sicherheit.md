# Sicherheit und Datenschutz

[← Handbuch](README.md)

- ShotKit arbeitet vollständig lokal: kein Konto, keine Telemetrie, keine eigene Internetverbindung. Einzige
  Ausnahme ist die Website-Schaltfläche im Fenster „Über ShotKit“, die auf Klick den Browser öffnet.
- Aufnahmen werden nur gespeichert, wenn du speicherst oder automatisches Speichern einschaltest. Ausnahmen:
  der [Diagnoseordner](#diagnoseordner-der-scroll-aufnahme) und [Reste nach einem Absturz](#temporäre-dateien-nach-einem-absturz).

## Vertrauliche Inhalte unkenntlich machen

| Werkzeug | Sicher? |
|---|---|
| **Schwärzen** (<kbd>S</kbd>) | **Ja** – deckt den Bereich schwarz ab. Die Schwärzung ist Teil des gespeicherten und kopierten Bildes. |
| Verpixeln (<kbd>B</kbd>) | **Nein** – kleiner Text bleibt teils lesbar und lässt sich mit speziellen Werkzeugen teilweise zurückrechnen. |
| Weichzeichnen (<kbd>W</kbd>) | **Nein** – Inhalte können erkennbar oder rekonstruierbar bleiben. |
| Rechteck, Marker | **Nein** – das Rechteck zeichnet nur einen Rahmen, der Marker ist halbtransparent. |

> [!CAUTION]
> Für Passwörter, Kundendaten, Kontonummern o. Ä. nur **Schwärzen** verwenden – oder den Bereich mit **Zuschneiden**
> (<kbd>C</kbd>) entfernen bzw. gar nicht erst aufnehmen. Verpixeln und Weichzeichnen sind kein Schutz.

Im Editor bleiben das Originalbild und die Schwärzung getrennt bearbeitbar; erst beim Speichern, Kopieren und
Weitergeben wird sie fest in das Bild eingerechnet. Verpixeln, Weichzeichnen und Bildlupe sehen dabei nur das
bereits geschwärzte Bild. Prüfe das Ergebnis vor der Weitergabe.

### Automatisches Speichern und Kopieren

Die Optionen **Automatisch als PNG speichern** und **In Zwischenablage kopieren** wirken sofort nach der Aufnahme –
**vor** dem Editor und damit vor jeder Schwärzung. Bereits gespeicherte Dateien und Zwischenablagekopien ändert
eine spätere Schwärzung nicht. Soll nur die geschwärzte Fassung ausgegeben werden: beide Optionen vorher im Menü
ausschalten, im Editor schwärzen und erst dann speichern oder kopieren.

## Zwischenablage

Kopierte Aufnahmen können im Windows-Zwischenablageverlauf (<kbd>Win</kbd>+<kbd>V</kbd>) und im Cloud-Abgleich mit
anderen Geräten landen. Die Einstellung **Aufnahmen von Zwischenablageverlauf und Cloud ausschließen** ist
standardmäßig **an**: ShotKit bittet Windows dann, solche Bilder nicht aufzunehmen. Das ist keine Garantie, und
bereits vorhandene Einträge werden nicht entfernt – diese lassen sich in Windows einzeln löschen.

## Diagnoseordner der Scroll-Aufnahme

Lässt sich bei einer [Scroll-Aufnahme](aufnehmen.md#scroll-aufnahme) ein Bildabschnitt nicht zuordnen und ist ein
Speicherordner eingestellt, legt ShotKit unter `Diagnose` zwei Bilder des gewählten Bereichs und eine Textdatei ab.
Sie können vertrauliche Inhalte und Bildschirmkoordinaten enthalten und werden nicht automatisch gelöscht. Lösche sie
nach der Fehlersuche, bevor du den Ordner weitergibst.

## Temporäre Dateien nach einem Absturz

Beim Speichern schreibt ShotKit zuerst in eine temporäre Datei und ersetzt erst nach Erfolg das Ziel. Bricht
ShotKit oder Windows dabei abrupt ab, können im Zielordner Dateien der Form `.shotkit-….tmp` zurückbleiben. Sie
können Bilddaten enthalten. Prüfe sie und die vorhandene Zieldatei; lösche solche Dateien nur, wenn Herkunft und
Inhalt geklärt sind. Ab Version 2.0.0 weist ShotKit beim nächsten Start darauf hin. <!-- TODO #67: prüfen, sobald umgesetzt -->

## Dateien von Netzlaufwerken

Wird ShotKit mit einer Datei auf einem Netzwerkserver gestartet (UNC-Pfad oder zugeordnetes Netzlaufwerk), warnt
es vor dem Öffnen: ShotKit kontaktiert dann den Server, und Windows kann dabei deine Anmeldedaten automatisch zur
Authentifizierung verwenden. Lässt sich der Laufwerkstyp nicht sicher bestimmen, weist ein eigener Dialog darauf hin.
Die Prüfung erkennt nicht jeden Netzwerkzugriff, etwa über Verzeichnisverknüpfungen.

## Echtheit der EXE

Die EXE ist nicht digital signiert; Windows SmartScreen warnt deshalb. Eine Signierung ist derzeit nicht geplant.
Die `.sha256`-Datei schützt vor Übertragungsfehlern, aber nicht vor einer Manipulation des Releases, da sie am
selben Ort liegt. Lade ShotKit deshalb nur von der offiziellen
[Releases-Seite](https://github.com/BomBastikDE/shotkit/releases).

## Sicherheitslücke melden

Bitte **nicht** als öffentliches Issue, sondern vertraulich – siehe [SECURITY.md](../SECURITY.md).

---

[← Einstellungen](einstellungen.md) · Weiter: [Umstieg von C# →](umstieg.md)
