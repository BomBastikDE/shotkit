# ShotKit: Ausblick nach 2.0.0

ShotKit 2.0.0 ist die erste stabile Version in Rust. Sie ersetzt die C#-Version 1.2.0 (Branch `csharp/1.x`, dort nur noch Sicherheitskorrekturen). Der Funktionsvergleich mit 1.2.0 steht in [PARITY.md](PARITY.md), die Änderungen im [Änderungsprotokoll](CHANGELOG.md).

**Maßgeblich ist der Meilenstein [„nach 2.0.0“](https://github.com/BomBastikDE/shotkit/milestone/2)** im Issue-Tracker. Diese Datei fasst nur die Richtung zusammen; Details, Prioritäten und Abschlusskriterien stehen in den Issues.

## Qualität und Sicherheit

- Rauschen der Verpixelung kryptografisch ableiten ([#85](https://github.com/BomBastikDE/shotkit/issues/85)).
- `unsafe`-Fläche verkleinern und das Audit aktuell halten ([#69](https://github.com/BomBastikDE/shotkit/issues/69)).
- Den globalen `static mut STATE` des Auswahl-Overlays durch fenstergebundenen Zustand (`GWLP_USERDATA`) oder thread-lokalen Zustand ersetzen. Das soll Zuständigkeit und Lebensdauer des Zustands klarer machen und die Prüfung verschachtelter Windows-Nachrichten erleichtern.
- Horizontales Schwenken ([#42](https://github.com/BomBastikDE/shotkit/issues/42)) und Verträglichkeit älterer Einstellungsdateien ([#26](https://github.com/BomBastikDE/shotkit/issues/26)).

## Editor

- Bildgröße ändern, also skalieren statt nur die Leinwand ([#32](https://github.com/BomBastikDE/shotkit/issues/32)).
- Bereich ausschneiden und Rissrand-Effekt ([#33](https://github.com/BomBastikDE/shotkit/issues/33)).
- Schnellstile und Favoriten ([#34](https://github.com/BomBastikDE/shotkit/issues/34)).

## Aufnahme

- Mauszeiger optional mit aufnehmen ([#36](https://github.com/BomBastikDE/shotkit/issues/36)): Einbau in PR #103, vor 2.1.0 zurückgestellt, weil der Zeiger im Praxistest fehlte. Die Bausteine (Zeigerbild, Zeigerobjekt im Editor) sind im Code vorhanden; der Weg zurück steht in #36.

## Konzepte

- Web-Version mit gemeinsamem Rust-Kern ([#8](https://github.com/BomBastikDE/shotkit/issues/8)).

## Ausdrücklich nicht geplant

Videoaufnahme und GIF, Bildbibliothek, Cloud-Teilen, automatisches Schwärzen, Texterkennung (OCR), Schritt-Aufnahme und das Kombinieren mehrerer Bilder (Owner-Entscheidung vom 04.11.2026). Code-Signing ist derzeit ebenfalls nicht geplant.

Über eine mögliche spätere Ablage der C#-Quellen unter `legacy/` entscheidet der Owner; der Tag `v1.2.0` bleibt als historische Referenz erhalten.
