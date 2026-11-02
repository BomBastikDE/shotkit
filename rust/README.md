# ShotKit Rust – Entwicklungsstand

Rust-Portierung von [ShotKit](https://github.com/BomBastikDE/shotkit) v1.2.0 (Referenzcommit `cd5fff3`). **Noch nicht veröffentlichungsreif:** Funktionsumfang und Windows-Verhalten sind nicht vollständig geprüft. Das ursprüngliche C#-ShotKit bleibt die führende Version.

## Bisherige Funktionen

- Tray-Menü; primären Bildschirm mit `F9`, Bereich mit `F11` oder `Druck` aufnehmen. Beim Bereich lassen sich Vorschläge unter dem Mauszeiger anklicken oder Bereiche frei aufziehen. `Strg` wählt das größere Fenster.
- Experimentelle Scroll-Aufnahme mit `F10`: Bereich auswählen, nach unten/rechts/links scrollen, Abbruch mit Esc oder Mausbewegung. Der bisher gesicherte Teil bleibt erhalten.
- Editor-Werkzeuge über die Leiste: Auswahl, Pfeil, Linie, Rechteck, Ellipse, Text, Marker, Verpixeln, Nummer und Stift. Objekte können ausgewählt, verschoben, geändert und mit `Strg+Z`/`Strg+Y` zurückgenommen/wiederholt werden.
- `Strg+O` öffnet Bilder, `Strg+S` speichert, `Strg+Umschalt+S` speichert unter; PNG/JPEG/BMP/GIF stehen zur Verfügung. `Strg+C` kopiert ein Bitmap in die Zwischenablage. Vor dem Ersetzen ungesicherter Bilder wird nachgefragt.
- `Strg+Mausrad` ändert den Zoom, `Strg+0` passt das Bild ein, `Strg+9` zeigt es in Originalgröße. Mausrad verschiebt vertikal, Umschalt+Mausrad horizontal.

## Bekannte Grenzen

Windows-Laufzeittests fehlen noch. Die Fenster-/Browser-Erkennung hängt von den Windows-Bedienungshilfen und der jeweiligen Anwendung ab; nicht jedes Seitenelement lässt sich erkennen. Scrollen, DPI-Skalierung, mehrere Monitore, Clipboard und Editor sind auf realen Windows-Systemen zu prüfen. Bearbeitete Bilder mit Transparenz lassen sich momentan aus Sicherheitsgründen nicht speichern. Einstellungen, Farbwahl, einige Bild- und Editorfunktionen sowie weitere Originalfunktionen fehlen noch. Siehe [ROADMAP.md](ROADMAP.md).

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
