# ShotKit Rust – Alpha-Testversion

Rust-Portierung von [ShotKit](https://github.com/BomBastikDE/shotkit) v1.2.0 (Referenzcommit `cd5fff3`). **Vorabversion für Tests:** Funktionsumfang und Windows-Verhalten sind nicht vollständig geprüft. Die letzte als stabil freigegebene Version ist bis zur ersten Rust-Beta das C#-ShotKit 1.2.0 (Branch `csharp/1.x`).

## Umstieg von C#

Zum Testen die C#-Instanz und gegebenenfalls eine ältere Rust-Version vollständig über das Tray-Menü beenden; sonst kann ein erneuter Start die bereits laufende Version aktivieren. Danach das gewünschte Rust-Alpha-Pre-Release von der [Releases-Seite](https://github.com/BomBastikDE/shotkit/releases) laden und EXE sowie SHA256-Datei prüfen. Die Versionen verwenden getrennte Einstellungen; Rust übernimmt die C#-Einstellungen nicht. Ausgabeoptionen und Autostart im Rust-Tray-Menü neu prüfen. Für die Rückkehr Rust beenden und die bisherige C#-EXE starten. Die vollständige Migrationsbeschreibung steht in der [Haupt-README](../README.md#rust-version-testen-und-später-umsteigen).

Die Rust-EXE läuft ohne Installer und kann in einem selbst gewählten Ordner liegen. Zum Entfernen zuerst einen gegebenenfalls aktivierten Rust-Autostart im Tray-Menü ausschalten und ShotKit über das Tray-Menü beenden; danach kann die EXE gelöscht werden. Die separaten Rust-Einstellungen liegen unter `%APPDATA%\ShotKitRust\settings.json` (falls `APPDATA` fehlt, unter `%LOCALAPPDATA%\ShotKitRust\settings.json`). Sie können für eine spätere Nutzung erhalten oder bei Bedarf separat entfernt werden. Bereits gespeicherte Bilder liegen im jeweils gewählten Speicherordner und werden dadurch nicht entfernt.

## Bisherige Funktionen

- Tray-Menü mit „Über ShotKit“ (Oliver Muenk und Website); Bildschirm unter dem Mauszeiger mit `F9` oder `Strg+Druck`, alle Bildschirme mit `Strg+Umschalt+Druck`, aktives Fenster mit `Umschalt+Druck` und Bereich mit `F11` oder `Druck` aufnehmen. Beim Bereich lassen sich Vorschläge unter dem Mauszeiger anklicken oder Bereiche frei aufziehen; eine Lupe zeigt Bildschirmkoordinaten und den Pixelwert. `Strg` wählt das größere Fenster.
- Experimentelle Scroll-Aufnahme mit `F10`: Bereich auswählen, nach unten/rechts/links scrollen, Abbruch mit Esc oder Mausbewegung. Der bisher gesicherte Teil bleibt erhalten. Lässt sich ein Bildabschnitt nach erneutem Versuch nicht zuordnen und ist keine Scrollweite bekannt, legt ShotKit bei konfiguriertem und vorhandenem Speicherordner unter `Diagnose` zwei Bilder des ausgewählten Bereichs und eine Textdatei ab. Diese können vertrauliche Inhalte und Bildschirmkoordinaten enthalten und bleiben dort bis zur manuellen Löschung. Ohne konfigurierten Ordner oder bei Schreibfehlern wird keine erfolgreiche Ablage gemeldet; das Teilbild bleibt erhalten.
- Auswahl präzise korrigieren: Beim Loslassen eines selbst aufgezogenen Bereichs `Umschalt` gedrückt halten. Danach verschieben die Pfeiltasten den Bereich jeweils um ein Bildschirmpixel, `Umschalt` + Pfeiltaste verändert die rechte bzw. untere Kante, `Enter` bestätigt und `Esc` bricht ab. Bei einem angezeigten Bereichsvorschlag startet eine Pfeiltaste die Korrektur. Mausbewegung kehrt zur normalen Auswahl zurück. `Strg` + Pfeiltaste bewegt den Mauszeiger um ein Bildschirmkoordinaten-Pixel, mit zusätzlichem `Umschalt` um zehn; die normale Bereichskorrektur bleibt erhalten.
- „Letzten Bereich erneut aufnehmen“ im Tray-Menü wiederholt die letzte erfolgreiche Bereichsaufnahme dieser Sitzung mit den aktuellen Ausgabe-Einstellungen. Der Eintrag ist vor der ersten Bereichsaufnahme deaktiviert; nach einem Monitorwechsel wird geprüft, ob der Bereich noch vollständig auf angeschlossenen Bildschirmen liegt.
- Der Editor verwendet eine kompakte Symbolleiste. Beim Darüberfahren zeigt jedes Symbol seinen Namen und gegebenenfalls sein Tastenkürzel; Farbfelder erlauben eine direkte Farbauswahl. Das Tray verwendet das rote ShotKit-Symbol.
- Mehrere Bilder können in getrennten Editorfenstern bearbeitet werden. Editor-Werkzeuge über die Leiste: Auswahl, Pfeil, Linie, Rechteck, Ellipse, Text, Marker, Verpixeln, Nummer, Stift und Zuschneiden. Objekte können ausgewählt, verschoben, geändert und mit `Strg+Z`/`Strg+Y` zurückgenommen/wiederholt werden. Rahmen und Schlagschatten sind über die Werkzeugleiste verfügbar.
- Verpixeln ist keine sichere Schwärzung: Inhalte können trotz Verpixelung erkennbar oder rekonstruierbar bleiben. Vertrauliche Informationen vor der Weitergabe vollständig und deckend entfernen.
- Geöffnete JPEG-, GIF- und TIFF-Originale werden beim ersten `Strg+S` geschützt: Nach Bestätigung ist ein anderer Speichername erforderlich. JPEG- und GIF-Exporte erhalten Metadaten beziehungsweise Animationen nicht vollständig; EXIF-Ausrichtung wird beim Laden berücksichtigt.
- Übergibt eine zweite ShotKit-Instanz einen UNC-Netzwerkpfad, weist die Bestätigung auf den möglichen Serverkontakt und automatische Windows-Authentifizierung hin. Netzlaufwerke mit Laufwerksbuchstaben werden dabei nicht gesondert erkannt.
- Beim Zeichnen im Editor begrenzt `Umschalt` Rechtecke, Ellipsen, Marker und Verpixelung auf quadratische Formen sowie Linien und Pfeile auf 45-Grad-Schritte.
- `Strg+O` öffnet Bilder, `Strg+S` speichert, `Strg+Umschalt+S` speichert unter; PNG/JPEG/BMP/GIF stehen zum Speichern zur Verfügung; TIFF kann zusätzlich geöffnet werden. `Strg+C` kopiert ein Bitmap in die Zwischenablage; `Strg+V` öffnet ein Bild daraus. Vor dem Ersetzen ungesicherter Bilder wird nachgefragt.
- `Strg+Mausrad` ändert den Zoom, `Strg+0` zeigt das Bild in Originalgröße, `Strg+9` passt es ein. Mausrad verschiebt vertikal, Umschalt+Mausrad horizontal.

## Bekannte Grenzen

Auf einem interaktiven Windows-CI-Runner wurden Bereichsaufnahme, vertikale und horizontale Scrollbilder, lokale Testseiten in Edge und Chrome für Elementvorschläge, ein Editorablauf mit Transparenz, das Kopieren und Einfügen transparenter Bilder, das Infofenster und die unabhängige Bearbeitung mehrerer Bilder geprüft. Weitere Windows-Praxistests stehen aus. Die Fenster-/Browser-Erkennung hängt von den Windows-Bedienungshilfen und der jeweiligen Anwendung ab; nicht jedes Seitenelement lässt sich erkennen. Scrollen, DPI-Skalierung, mehrere Monitore, Clipboard und Editor sind auf realen Windows-Systemen zu prüfen. Transparente Bilder können bearbeitet und in passenden Formaten gespeichert werden; die neuen Bild- und Editorfunktionen benötigen weitere Windows-Praxistests. Die vollständige Übereinstimmung mit dem Original ist noch nicht bestätigt. Siehe [ROADMAP.md](ROADMAP.md).

Ein unerwarteter Programmabbruch beendet alle offenen Editorfenster; nicht gespeicherte Änderungen und nur im Arbeitsspeicher zurückgehaltene Aufnahmen gehen verloren. Die Wiederherstellungsfunktion im Tray hält solche Aufnahmen nur während der laufenden Sitzung vor. Wichtige Bearbeitungen daher regelmäßig speichern.

Beim ersten Start öffnet sich nach einer Aufnahme der Editor; automatisches Speichern und Kopieren in die Zwischenablage sind zunächst ausgeschaltet. Diese Ausgaben und der optionale Autostart können im Tray-Menü eingestellt werden. Die Rust-Version verwendet `%APPDATA%\ShotKitRust` und ändert die Einstellungen der C#-Version nicht.

Keine OCR oder Texterkennung aus Bildern. Die EXE ist unsigniert und ausschließlich für Testzwecke gedacht. Im Entwicklungsstand nur mit Kopien wichtiger Bilder arbeiten.

Für einen manuellen Test auf Windows-Geräten siehe [WINDOWS_PRAXISTEST.md](WINDOWS_PRAXISTEST.md).

## Bauen und testen

```sh
docker build -t shotkit-rust-build:local .
docker run --rm --user "$(id -u):$(id -g)" -e CARGO_HOME=/work/target/cargo-home -v "$PWD:/work" shotkit-rust-build:local cargo test --locked --lib
docker run --rm --user "$(id -u):$(id -g)" -e CARGO_HOME=/work/target/cargo-home -v "$PWD:/work" shotkit-rust-build:local cargo build --locked --release --target x86_64-pc-windows-gnu
```

Ergebnis: `target/x86_64-pc-windows-gnu/release/shotkit.exe`. Docker und MinGW sind nur für den Linux-Cross-Build erforderlich. Ein erfolgreicher Build und synthetische Tests ersetzen keine interaktiven Windows-Tests.

### Interaktive Windows-Smoke-Tests

Die vollständige Testreihe benötigt PowerShell 7, einen interaktiven Windows-Desktop sowie Edge und Chrome. Sie bedient Maus und Tastatur und ersetzt den Inhalt der Zwischenablage; deshalb nur in einer dafür vorgesehenen Testsitzung ausführen und alle laufenden ShotKit-Instanzen vorher schließen.

```powershell
cargo build --locked --release
pwsh -NoProfile -File .\tests\run-all.ps1 -ExePath .\target\release\shotkit.exe -AllowLocalInteractive
```

Das Skript führt derzeit 43 Szenarien getrennt und nacheinander aus, meldet jeden Fehler und beendet sich bei blockierenden Fehlern mit einem Fehlercode. Der Toolbar-Test und der zusätzliche Test der schmalen Leiste sind in der CI verbindlich. Die vorherige Ausnahme für den Toolbar-Test wurde nach wiederholten erfolgreichen CI-Läufen entfernt ([42004167628](https://github.com/BomBastikDE/shotkit/actions/runs/42004167628), [42006367024](https://github.com/BomBastikDE/shotkit/actions/runs/42006367024), [42007837556](https://github.com/BomBastikDE/shotkit/actions/runs/42007837556), [42015861642](https://github.com/BomBastikDE/shotkit/actions/runs/42015861642), [42017257158](https://github.com/BomBastikDE/shotkit/actions/runs/42017257158)).

## Lizenz

MIT, siehe [LICENSE](LICENSE). Der Lizenzhinweis des ursprünglichen ShotKit bleibt erhalten.
