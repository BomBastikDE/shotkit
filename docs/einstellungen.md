# Einstellungen

[← Handbuch](README.md)

## Menü im Infobereich

Ein **Linksklick** auf das ShotKit-Symbol startet die Bereichsaufnahme, ein **Rechtsklick** öffnet das Menü:

| Eintrag | Wirkung |
|---|---|
| Bildschirm aufnehmen (F9) | Bildschirm unter dem Mauszeiger |
| Bereich nach 3 / 5 / 10 Sekunden auswählen | verzögerte Bereichsaufnahme |
| Bereich aufnehmen (F11) | Bereich oder Fenster auswählen |
| Letzten Bereich erneut aufnehmen | wiederholt den letzten Bereich dieser Sitzung (zuerst ausgegraut) |
| Scroll-Aufnahme (F10) | lange Seiten, siehe [Aufnehmen](aufnehmen.md#scroll-aufnahme) |
| Alle Bildschirme aufnehmen | alle Monitore in einem Bild |
| Aktives Fenster aufnehmen | das Fenster im Vordergrund |
| Editor anzeigen | zeigt die geöffneten Editorfenster |
| Bild öffnen … | Bilddatei im Editor öffnen |
| Bild aus Zwischenablage öffnen | Bild aus der Zwischenablage im Editor öffnen |
| Speicherordner öffnen | öffnet den gewählten Ordner im Explorer |
| Ungesicherte Aufnahme speichern … | sichert eine Aufnahme nachträglich, die weder im Editor geöffnet noch gespeichert wurde (auch eine horizontale Scroll-Aufnahme, deren Rückdrehung scheiterte) |
| Einstellungen | Untermenü, siehe unten |
| Über ShotKit | Infofenster mit Version und Website-Schaltfläche |
| Beenden | beendet ShotKit |

## Untermenü „Einstellungen“

| Einstellung | Wirkung | Beim ersten Start |
|---|---|---|
| Nach Aufnahme Editor öffnen | jede Aufnahme öffnet ein Editorfenster | an |
| In Zwischenablage kopieren | jede Aufnahme wird sofort kopiert | aus |
| Automatisch als PNG speichern | jede Aufnahme wird im Speicherordner abgelegt | aus |
| Aufnahmen von Zwischenablageverlauf und Cloud ausschließen | ShotKit bittet Windows, kopierte Bilder nicht im Verlauf (<kbd>Win</kbd>+<kbd>V</kbd>) und nicht im Cloud-Abgleich abzulegen | an |
| Speicherordner wählen … | Ziel für automatisches Speichern | – (wird beim Einschalten abgefragt) |
| Mit Windows starten | ShotKit startet mit der Anmeldung | aus |

Automatisch gespeicherte Dateien heißen `ShotKit-<Zeitstempel>-<Nummer>.png`. Vorhandene Dateien ersetzt ShotKit nie.

> [!IMPORTANT]
> Automatisches Speichern und Kopieren geschehen sofort nach der Aufnahme – **vor** einer späteren Schwärzung im
> Editor. Soll nur die geschwärzte Fassung ausgegeben werden, schalte beide Optionen vorher aus und speichere
> oder kopiere erst im Editor. Siehe [Sicherheit](sicherheit.md).

ShotKit merkt sich außerdem die zuletzt gewählte Scroll-Richtung.

## Einstellungsdatei

Die Einstellungen liegen in `%APPDATA%\ShotKitRust\settings.json` (fehlt `APPDATA`, unter
`%LOCALAPPDATA%\ShotKitRust\settings.json`). Der Autostart wird nicht in dieser Datei, sondern bei Windows
eingetragen.

Ist die Datei beschädigt, startet ShotKit mit Standardwerten und deaktiviert das Speichern der Einstellungen, damit
nichts überschrieben wird. Ist sie eindeutig als Version 1 erkennbar, fragt ShotKit **„Einstellungen
zurücksetzen?“** (siehe [Häufige Fragen](faq.md)).

---

[← Editor](editor.md) · Weiter: [Sicherheit und Datenschutz →](sicherheit.md)
