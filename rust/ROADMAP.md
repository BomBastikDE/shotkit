# Rust-Portierung: Funktionsstand

Referenz: ShotKit C# v1.2.0, Commit `cd5fff3`. Der Rust-Zweig ist ein Arbeitsstand, kein Ersatz und kein neues Release.

## Bestehende ShotKit-Funktionen für die Portierung

- Tray-Menü, Einzelinstanz, globale und konfigurierbare Tastenkürzel, verzögerte Aufnahme.
- Bereich/Fenster/Unterfenster, Monitor unter Maus, alle Monitore, aktives Fenster; Auswahl-Overlay, Lupe und DPI.
- Scroll-Aufnahme: Bereichs- und Fensterwahl, Auswahl der Richtung (unten/rechts/links), Scrollen, Stop mit Esc/Maus, Teilbild erhalten, ruhige Bildfolge, Bildabgleich, Grenzwerte, Hinweise und Diagnose.
- Editor: Auswahl und Bearbeitung aller Formen, Pfeile, Text, Textmarker, Verpixeln, Schritt-Nummern, Freihand, Zuschneiden, Zoom, Rahmen/Schatten, Undo/Redo, ungespeicherte Änderungen.
- PNG/JPG/BMP/GIF, Öffnen/Speichern, Zwischenablage, optionales automatisches Speichern, Speicherordner und Autostart.

## Stand

- Experimenteller Windows-Editor: Werkzeugleiste, bearbeitbare Formen und Undo/Redo; native Dialoge und PNG/JPEG/BMP/GIF; Bildschirm- und Bereichsaufnahme, Scroll-Prototyp, Zoom/Pan und Zwischenablage sind implementiert. Windows-Laufzeittest ausstehend.
- Scroll-Algorithmus und Editorlogik sind synthetisch getestet. Die Anbindung an echte Browser, Fenster, DPI und Maus muss unter Windows überprüft werden.
- Transparente Bilder werden nach Bearbeitung vorsorglich nicht gespeichert; verlustfreie Bearbeitung und Export müssen ergänzt werden.
- Crop ist mit der gemeinsamen Bild- und Objekt-Historie integriert; Rahmen und Schatten haben getestete Bildoperationen, sind aber noch nicht in den Editor integriert. Einstellungen und Öffnen aus Clipboard sind experimentell integriert; weitere Originalfunktionen fehlen. Farb-/Breitenwahl und Textänderungen sind implementiert, aber unter Windows noch ungeprüft.
- Fenster- und UI-Automation-Vorschläge sind experimentell integriert; brauchbare Grenzen je Browser und Seitentyp sind noch nicht verifiziert.

## Ideen nach funktionsgleicher Portierung

Sprechblasen mit Pfeil, vergrößerter Bildausschnitt und vereinfachtes Hervorheben eines Details. Text aus Bildern/OCR ist ausdrücklich zurückgestellt. Zusätzliche Funktionen getrennt nach Nutzen, Paketgröße und Windows-Tests bewerten.

## Erforderliche Endprüfung

Auf Windows 10/11 mit normaler und gemischter DPI prüfen: Bereichs-/Fensterwahl, Scrollen vertikal/horizontal, fixe Kopf-/Fußleisten, dynamische Inhalte, vorzeitiger Abbruch, Bildqualität an Nähten, Zwischenablage/Dateiformate, alle Editor-Werkzeuge, mehrere Monitore, Speichern und Neustart. Automatische Logiktests und Cross-Build ersetzen diese Prüfungen nicht.

## Bereichsvorschläge unter dem Mauszeiger

Gewünscht: Beim Bewegen der Maus Fenster und sinnvolle Bereiche optisch umranden und per Klick übernehmen; manuelles Aufziehen bleibt möglich. In Browsern sind Seitenelemente häufig keine einzelnen Win32-Fenster. Deshalb Windows UI Automation und die zugänglichen Elementgrenzen untersuchen; bei verschachtelten Elementen Größen- und Auswahlregeln vorsehen. Webseiten, Canvas, fehlende Accessibility-Daten, Zoom und gemischte DPI können die Erkennung begrenzen. Die Funktion soll lokal arbeiten und bei ungeeigneten Treffern zur manuellen Auswahl zurückfallen. Experimentell integriert, noch nicht unter Windows geprüft.

## Windows-CI

Der separate GitHub-Zweig `rust/development` führt Linux- und Windows-Tests sowie einen Windows-Release-Build aus. Die Läufe für Commits `fe34bc9` und `1a342e3` waren auf beiden Plattformen erfolgreich; letzterer umfasst Windows-GDI-Pixeltests. Die CI verifiziert keine interaktive Windows-Oberfläche und keine realen Browser-Scrollaufnahmen.
