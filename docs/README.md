# ShotKit – Benutzerhandbuch

ShotKit ist ein Screenshot- und Bildbearbeitungswerkzeug für Windows 10 und 11. Es läuft im Infobereich der
Taskleiste, nimmt Bereiche, Fenster, Bildschirme und lange Seiten auf und bietet einen Editor für Pfeile,
Markierungen, Text, Sprechblasen, Stempel, Spotlight, Lupe und sicheres Schwärzen.

> [!NOTE]
> Dieses Handbuch beschreibt **ShotKit 2.1.1**. Fehler bitte als
> [Issue](https://github.com/BomBastikDE/shotkit/issues) melden (siehe [Häufige Fragen](faq.md#fehler-melden)).

## Inhalt

1. [Installation und erster Start](installation.md)
2. [Aufnehmen](aufnehmen.md) – Tastenkürzel, Bereich wählen, Scroll-Aufnahme
3. [Editor](editor.md) – Werkzeuge, Menüs und Tastenkürzel
4. [Einstellungen](einstellungen.md) – Menü im Infobereich und was nach einer Aufnahme passiert
5. [Sicherheit und Datenschutz](sicherheit.md) – sicher schwärzen, Zwischenablage, Diagnosedateien
6. [Umstieg von der C#-Version 1.2.0](umstieg.md)
7. [Häufige Fragen und Fehlerbehebung](faq.md)

## Schnellstart

1. Die neueste Version (`shotkit-v2.1.1-windows-x64.exe`) von den [Releases](https://github.com/BomBastikDE/shotkit/releases) laden und starten.
2. <kbd>Druck</kbd> drücken, Bereich aufziehen oder ein Fenster anklicken.
3. Im Editor markieren, vertrauliche Stellen mit **Schwärzen** (<kbd>S</kbd>) abdecken und mit <kbd>Strg</kbd>+<kbd>C</kbd> kopieren oder <kbd>Strg</kbd>+<kbd>S</kbd> speichern.

## Für Entwickler

Bauen, Tests und Projektstand stehen in der [technischen Übersicht](../rust/README.md), im
[Testprotokoll](../rust/TESTPROTOKOLL.md) und im [Änderungsprotokoll](../rust/CHANGELOG.md).

> [!TIP]
> **Pflege:** Ändern sich Tastenkürzel, Menüeinträge oder Werkzeuge, wird das Handbuch im selben Pull Request
> angepasst. Bilder für das Handbuch gehören nach `docs/images/`.
