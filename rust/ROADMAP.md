# Rust-Portierung: Funktionsstand

Referenz: ShotKit C# v1.2.0, Commit `cd5fff3`. Rust ist als Vorabversion `v2.0.0-alpha.9` veröffentlicht; die C#-Version bleibt die stabile Hauptversion.

## Nach der Beta

- Den globalen `static mut STATE` des Auswahl-Overlays durch fenstergebundenen Zustand (`GWLP_USERDATA`) oder thread-lokalen Zustand ersetzen. Das soll Zuständigkeit und Lebensdauer des Zustands klarer machen und die Prüfung verschachtelter Windows-Nachrichten erleichtern; die konkrete Lösung wird nach der Beta festgelegt.

## Bestehende ShotKit-Funktionen für die Portierung

- Tray-Menü, Einzelinstanz, globale Tastenkürzel und verzögerte Aufnahme.
- Bereich/Fenster/Unterfenster, Monitor unter Maus, alle Monitore, aktives Fenster; Auswahl-Overlay, Lupe und DPI.
- Scroll-Aufnahme: Bereichs- und Fensterwahl, Auswahl der Richtung (unten/rechts/links), Scrollen, Stop mit Esc/Maus, Teilbild erhalten, ruhige Bildfolge, Bildabgleich, Grenzwerte, Hinweise und Diagnose.
- Editor: Auswahl und Bearbeitung aller Formen, Pfeile, Text, Textmarker, Verpixeln, Schritt-Nummern, Freihand, Zuschneiden, Zoom, Rahmen/Schatten, Undo/Redo, ungespeicherte Änderungen.
- PNG/JPG/BMP/GIF, Öffnen/Speichern, Zwischenablage, optionales automatisches Speichern, Speicherordner und Autostart.

## Stand

- Experimenteller Windows-Editor: Werkzeugleiste, bearbeitbare Formen und Undo/Redo; native Dialoge und PNG/JPEG/BMP/GIF; Bildschirm- und Bereichsaufnahme, Scroll-Prototyp, Zoom/Pan und Zwischenablage sind implementiert. Start und Bereichsaufnahme wurden auf einem Windows-CI-Runner geprüft; umfangreiche Praxistests stehen aus.
- Scroll-Algorithmus und Editorlogik sind synthetisch getestet. Eine lokale Edge-Seite wurde auf Windows mit Maus und tatsächlichem Scrollbild geprüft; weitere Anwendungen, DPI-Konfigurationen und Webseiten bleiben offen.
- Transparente Bilder werden mit Alpha-Pipeline bearbeitet und gespeichert. Interaktive Windows-Tests prüfen Export, Undo und das Kopieren und Einfügen transparenter Bilder pixelgenau; weitere Editorabläufe benötigen zusätzliche Prüfung.
- Crop, Rahmen und Schatten sind in den Editor und die gemeinsame Bild- und Objekt-Historie integriert. Einstellungen, Öffnen aus Clipboard, verzögerte Aufnahme und Dateiargumente sind implementiert. Die Vollständigkeit und diese Abläufe benötigen weitere Windows-Praxistests.
- Fenster- und UI-Automation-Vorschläge sind integriert und auf einer lokalen Edge-Testseite unter Windows geprüft; Grenzen je Browser und Seitentyp bleiben offen.

## Ideen nach funktionsgleicher Portierung

Mögliche spätere Erweiterungen sind ein **vergrößerter Bildausschnitt im Editor** (Lupe für kleine Details) und **Bild anheften** (ein Screenshot bleibt als Referenz über anderen Fenstern sichtbar). Beide sind Ideen ohne festgelegten Umsetzungstermin; vor der Umsetzung werden Nutzen, Paketgröße, Bedienung und Windows-Tests bewertet und die Zustimmung eingeholt. Weitere Ideen sind Sprechblasen mit Pfeil und vereinfachtes Hervorheben eines Details. Text aus Bildern/OCR ist ausdrücklich zurückgestellt.

## Erforderliche Endprüfung

Auf Windows 10/11 mit normaler und gemischter DPI prüfen: Bereichs-/Fensterwahl, Scrollen vertikal/horizontal, fixe Kopf-/Fußleisten, dynamische Inhalte, vorzeitiger Abbruch, Bildqualität an Nähten, Zwischenablage/Dateiformate, alle Editor-Werkzeuge, mehrere Monitore, Speichern und Neustart. Automatische Logiktests und Cross-Build ersetzen diese Prüfungen nicht.

## Bereichsvorschläge unter dem Mauszeiger

Gewünscht: Beim Bewegen der Maus Fenster und sinnvolle Bereiche optisch umranden und per Klick übernehmen; manuelles Aufziehen bleibt möglich. In Browsern sind Seitenelemente häufig keine einzelnen Win32-Fenster. Deshalb Windows UI Automation und die zugänglichen Elementgrenzen untersuchen; bei verschachtelten Elementen Größen- und Auswahlregeln vorsehen. Webseiten, Canvas, fehlende Accessibility-Daten, Zoom und gemischte DPI können die Erkennung begrenzen. Die Funktion soll lokal arbeiten und bei ungeeigneten Treffern zur manuellen Auswahl zurückfallen. Auf einer lokalen Testseite unter Windows in Edge und Chrome geprüft; weitere Webseiten und Browser bleiben offen.

## Windows-CI

Der separate GitHub-Zweig `rust/development` führt Linux- und Windows-Tests sowie einen Windows-Release-Build aus. Die Läufe für Commits `fe34bc9`, `1a342e3`, `6534b8d` und `5fa325a` waren auf beiden Plattformen erfolgreich; sie umfassen Windows-GDI-Pixeltests. Die CI prüft interaktiv Bereichsaufnahme, vertikales und horizontales Scrollen anhand synthetischer Seiten, Einzelinstanz und Hover. Im Edge-Test wurde ein Element einer lokalen HTML-Seite mit normal gestarteter Edge-Instanz erkannt und pixelgeprüft. Diese Tests bestanden auf Windows. Zusätzlich wurde auf einer lokal erzeugten langen Edge-Seite eine 600 × 2164 Pixel große Scrollaufnahme bis zum Ende erstellt und über alle Zeilen an vier Stellen mit der Vorlage verglichen (CI-Commit `3e34f40`). Auch die unabhängige Bearbeitung mehrerer Dokumente wurde auf dem Windows-Runner geprüft (CI-Commit `8120573`). Andere Webseiten und Browser, mehrere Monitore, gemischte DPI und die komplette Editorbedienung sind dadurch nicht verifiziert.

Der veröffentlichte Stand `eec082b` (Alpha 4) bestand [GitHub Actions Lauf 41451797144](https://github.com/BomBastikDE/shotkit/actions/runs/41451797144): Linux und Windows erfolgreich; unter anderem wurden das Infofenster über das Tray-Menü, das transparente Kopieren und Einfügen sowie die Scroll- und Browserprüfungen ausgeführt. Der Link im Infofenster wurde dabei nicht geöffnet. Die oben genannten realen Windows-Praxistests bleiben offen.

Der Windows-[Lauf 41453703225](https://github.com/BomBastikDE/shotkit/actions/runs/41453703225) hat die Elementauswahl auf derselben lokalen Testseite zusätzlich in Chrome geprüft: tatsächlicher Klick, ausgewählte Grenzen und vier Bildpunkte. Der Edge-Test bestand im selben Lauf ebenfalls. Andere Webseiten, Browser, Zoomstufen und DPI-Konfigurationen sind dadurch nicht allgemein nachgewiesen.

Die Vorabversion Alpha 5 basiert auf Commit `41e8bbd` und bestand [GitHub Actions Lauf 41457711642](https://github.com/BomBastikDE/shotkit/actions/runs/41457711642) unter Linux und Windows. Dabei wurden die pixelgenaue Tastaturkorrektur eines manuell gezogenen Bereichs, Abbruch und der normale Mausablauf sowie die Wiederholung des letzten Bereichs über das Tray-Menü geprüft. Tests auf realen Geräten und mit gemischten Skalierungen stehen weiterhin aus.

Die Vorabversion Alpha 6 basiert auf Commit `bc8efbe` und bestand [GitHub Actions Lauf 41459734057](https://github.com/BomBastikDE/shotkit/actions/runs/41459734057). Der Windows-Lauf prüfte insbesondere den Klick nach Tastaturkorrektur und das Wiederholen mit Bilddatei und Zwischenablage; Monitorwechsel und gemischte Skalierungen bleiben separate Praxistests.

Die Vorabversion Alpha 9 basiert auf Commit `7fba242` und bestand [GitHub Actions Lauf 41475395472](https://github.com/BomBastikDE/shotkit/actions/runs/41475395472) unter Linux und Windows. Der Windows-Lauf prüfte unter anderem das Anlegen, erneute Öffnen und Bearbeiten von Text sowie Rückgängig. Monitorwechsel und gemischte Skalierungen bleiben separate Praxistests.
