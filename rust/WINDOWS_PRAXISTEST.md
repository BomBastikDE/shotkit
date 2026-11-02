# ShotKit Rust: Windows-Praxistest

Diese Anleitung ergänzt die automatischen Tests. Sie ersetzt nicht die stabile C#-Version. Aktuelle Vorabversion: [v2.0.0-alpha.9](https://github.com/BomBastikDE/shotkit/releases/tag/v2.0.0-alpha.9); die Windows-x64-EXE ist nicht digital signiert. Bitte mit Testbildern und nicht mit vertraulichen Inhalten prüfen. Bei Auffälligkeiten Abbruch, ohne mehrfaches Überschreiben einer vorhandenen Datei.

## Ausgangslage notieren

- Windows-Version und -Build; Anzahl und Anordnung der Monitore sowie die jeweilige Skalierung (Windows-Einstellungen → System → Anzeige). Falls nur ein Monitor vorhanden ist, den Mehrmonitor-Teil als nicht geprüft markieren.
- ShotKit starten; Einstellungen der C#-Version bleiben getrennt. Im Tray-Menü einen Testordner wählen und die gewünschten Ausgabeoptionen (Editor, Zwischenablage, automatisches Speichern) bewusst einstellen.
- Im Tray „Über ShotKit“ öffnen: Versionsnummer, Oliver Muenk und die Schaltfläche für alleswasbewegt.de prüfen. Beim Anklicken soll die Website im Standardbrowser geöffnet werden.

## Aufnahme und Auswahl

1. Mit `Druck` oder `F11` einen Bereich frei aufziehen und prüfen, dass Bildinhalt und Ränder stimmen. Über einem Browserbereich den Vorschlag per Klick übernehmen; falls keine passende Umrandung erscheint, frei auswählen und Browser/Webseite notieren.
2. Beim Loslassen eines frei aufgezogenen Bereichs `Umschalt` halten. Mit Pfeiltasten verschieben, mit `Umschalt` + Pfeiltasten rechte/untere Kante ändern und mit `Enter` aufnehmen. Prüfen, ob die Auswahl jeweils um ein Bildschirmpixel verändert wird; `Esc` in einem neuen Versuch muss abbrechen.
3. Im Tray „Letzten Bereich erneut aufnehmen“ wählen und gleiche Grenzen und aktuellen Bildinhalt prüfen. Wenn ein zweiter Monitor vorhanden ist: Einen Bereich darauf aufnehmen, Monitor trennen oder Anzeigeanordnung ändern und Wiederholung versuchen. Liegt der Bereich nicht mehr vollständig auf aktiven Monitoren, muss ShotKit eine Meldung anzeigen statt eine andere Fläche aufzunehmen.
4. Falls mehrere Monitore mit unterschiedlichen Skalierungen vorhanden sind: Bereichsauswahl auf jedem Monitor und über deren Grenze testen. Grenzen und Bildinhalt visuell kontrollieren; Bildschirm-Anordnung und Skalierungen notieren.

## Scrollaufnahme

1. Mit `F10` eine längere, ruhige Testseite oder ein längeres Dokument aufnehmen. Anfang, Ende und mehrere Übergänge auf fehlende oder doppelte Zeilen prüfen.
2. Den Vorgang erneut starten und nach mindestens einem sichtbaren Scrollschritt mit `Esc` abbrechen. Das bereits aufgenommene Teilbild muss erhalten bleiben; Abbruchmeldung und Ergebnis prüfen. Eine weitere Aufnahme durch Mausbewegung abbrechen und ebenfalls prüfen.
3. Wenn eine geeignete Seite vorhanden ist, mit feststehender Kopf-/Fußleiste und einem Bereich, der horizontal gescrollt werden kann wiederholen. Webseite/Anwendung und beobachtete Fehler notieren.

## Editor und Ausgabe

1. Pfeil, Rechteck, Text, Marker und Verpixeln auf einem Testbild zeichnen. Mit gedrückter `Umschalt`-Taste Formen bzw. Linien ausrichten; ein Textobjekt mit aktivem Textwerkzeug erneut anklicken und ändern. `Strg+Z`/`Strg+Y` prüfen.
2. Zuschneiden und anschließend eine Form auswählen; danach als PNG speichern und erneut öffnen. Ein Bild in die Zwischenablage kopieren und mit `Strg+V` wieder öffnen; Größen und transparente Bildstellen prüfen, sofern das Testbild solche enthält.
3. Bei zwei Bildschirmen den Editor zwischen Monitoren verschieben und Darstellung, Werkzeugpositionen und gespeichertes Ergebnis kontrollieren.

## Ergebnis melden

Für jeden Schritt **bestanden**, **fehlgeschlagen** oder **nicht geprüft** notieren. Bei einem Fehler die Windows-Version, Monitor- und Skalierungskonfiguration, verwendete Anwendung/Webseite, Schritte und erwartetes gegenüber tatsächlichem Verhalten angeben. Screenshots nur dann teilen, wenn sie keine vertraulichen Informationen enthalten. Ein erfolgreicher Durchlauf belegt nur die konkret getestete Konfiguration.
