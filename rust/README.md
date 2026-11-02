# ShotKit Rust – Alpha-Testversion

Rust-Portierung von [ShotKit](https://github.com/BomBastikDE/shotkit) v1.2.0 (Referenzcommit `cd5fff3`). **Vorabversion für Tests:** Funktionsumfang und Windows-Verhalten sind nicht vollständig geprüft. Das ursprüngliche C#-ShotKit bleibt die stabile Version.

## Bisherige Funktionen

- Tray-Menü; Bildschirm unter dem Mauszeiger mit `F9` oder `Strg+Druck`, alle Bildschirme mit `Strg+Umschalt+Druck`, aktives Fenster mit `Umschalt+Druck` und Bereich mit `F11` oder `Druck` aufnehmen. Beim Bereich lassen sich Vorschläge unter dem Mauszeiger anklicken oder Bereiche frei aufziehen; eine Lupe zeigt Bildschirmkoordinaten und den Pixelwert. `Strg` wählt das größere Fenster.
- Experimentelle Scroll-Aufnahme mit `F10`: Bereich auswählen, nach unten/rechts/links scrollen, Abbruch mit Esc oder Mausbewegung. Der bisher gesicherte Teil bleibt erhalten.
- Mehrere Bilder können in getrennten Editorfenstern bearbeitet werden. Editor-Werkzeuge über die Leiste: Auswahl, Pfeil, Linie, Rechteck, Ellipse, Text, Marker, Verpixeln, Nummer, Stift und Zuschneiden. Objekte können ausgewählt, verschoben, geändert und mit `Strg+Z`/`Strg+Y` zurückgenommen/wiederholt werden. Rahmen und Schlagschatten sind über die Werkzeugleiste verfügbar.
- `Strg+O` öffnet Bilder, `Strg+S` speichert, `Strg+Umschalt+S` speichert unter; PNG/JPEG/BMP/GIF stehen zum Speichern zur Verfügung; TIFF kann zusätzlich geöffnet werden. `Strg+C` kopiert ein Bitmap in die Zwischenablage; `Strg+V` öffnet ein Bild daraus. Vor dem Ersetzen ungesicherter Bilder wird nachgefragt.
- `Strg+Mausrad` ändert den Zoom, `Strg+0` passt das Bild ein, `Strg+9` zeigt es in Originalgröße. Mausrad verschiebt vertikal, Umschalt+Mausrad horizontal.

## Bekannte Grenzen

Auf einem interaktiven Windows-CI-Runner wurden Bereichsaufnahme, vertikale und horizontale Scrollbilder, eine lokale Edge-Seite, Elementvorschläge und ein Editorablauf mit Transparenz geprüft. Weitere Windows-Praxistests stehen aus. Die Fenster-/Browser-Erkennung hängt von den Windows-Bedienungshilfen und der jeweiligen Anwendung ab; nicht jedes Seitenelement lässt sich erkennen. Scrollen, DPI-Skalierung, mehrere Monitore, Clipboard und Editor sind auf realen Windows-Systemen zu prüfen. Transparente Bilder können bearbeitet und in passenden Formaten gespeichert werden; die neuen Bild- und Editorfunktionen benötigen weitere Windows-Praxistests. Einige Originalfunktionen fehlen noch. Siehe [ROADMAP.md](ROADMAP.md).

Die Ausgaben nach einer Aufnahme (Editor, Zwischenablage, automatisches PNG) und der optionale Autostart können im Tray-Menü eingestellt werden. Die Rust-Version verwendet `%APPDATA%\ShotKitRust` und ändert die Einstellungen der C#-Version nicht.

Keine OCR oder Texterkennung aus Bildern. Die EXE ist unsigniert und ausschließlich für Testzwecke gedacht. Im Entwicklungsstand nur mit Kopien wichtiger Bilder arbeiten.

## Bauen und testen

```sh
docker build -t shotkit-rust-build:local .
docker run --rm --user "$(id -u):$(id -g)" -e CARGO_HOME=/work/target/cargo-home -v "$PWD:/work" shotkit-rust-build:local cargo test --locked --lib
docker run --rm --user "$(id -u):$(id -g)" -e CARGO_HOME=/work/target/cargo-home -v "$PWD:/work" shotkit-rust-build:local cargo build --locked --release --target x86_64-pc-windows-gnu
```

Ergebnis: `target/x86_64-pc-windows-gnu/release/shotkit-rust-prototype.exe`. Docker und MinGW sind nur für den Linux-Cross-Build erforderlich. Ein erfolgreicher Build und synthetische Tests ersetzen keine interaktiven Windows-Tests.

## Lizenz

MIT, siehe [LICENSE](LICENSE). Der Lizenzhinweis des ursprünglichen ShotKit bleibt erhalten.
