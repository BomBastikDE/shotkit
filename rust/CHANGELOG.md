# Änderungsprotokoll der Rust-Version

Die historischen CI-Nachweise werden unverändert aus der früheren ROADMAP übernommen.

## Vor Alpha 4

Der separate GitHub-Zweig `rust/development` führt Linux- und Windows-Tests sowie einen Windows-Release-Build aus. Die Läufe für Commits `fe34bc9`, `1a342e3`, `6534b8d` und `5fa325a` waren auf beiden Plattformen erfolgreich; sie umfassen Windows-GDI-Pixeltests. Die CI prüft interaktiv Bereichsaufnahme, vertikales und horizontales Scrollen anhand synthetischer Seiten, Einzelinstanz und Hover. Im Edge-Test wurde ein Element einer lokalen HTML-Seite mit normal gestarteter Edge-Instanz erkannt und pixelgeprüft. Diese Tests bestanden auf Windows. Zusätzlich wurde auf einer lokal erzeugten langen Edge-Seite eine 600 × 2164 Pixel große Scrollaufnahme bis zum Ende erstellt und über alle Zeilen an vier Stellen mit der Vorlage verglichen (CI-Commit `3e34f40`). Auch die unabhängige Bearbeitung mehrerer Dokumente wurde auf dem Windows-Runner geprüft (CI-Commit `8120573`). Andere Webseiten und Browser, mehrere Monitore, gemischte DPI und die komplette Editorbedienung sind dadurch nicht verifiziert.

## Alpha 4

Der veröffentlichte Stand `eec082b` (Alpha 4) bestand [GitHub Actions Lauf 41451797144](https://github.com/BomBastikDE/shotkit/actions/runs/41451797144): Linux und Windows erfolgreich; unter anderem wurden das Infofenster über das Tray-Menü, das transparente Kopieren und Einfügen sowie die Scroll- und Browserprüfungen ausgeführt. Der Link im Infofenster wurde dabei nicht geöffnet. Die oben genannten realen Windows-Praxistests bleiben offen.

## Zwischen Alpha 4 und Alpha 5

Der Windows-[Lauf 41453703225](https://github.com/BomBastikDE/shotkit/actions/runs/41453703225) hat die Elementauswahl auf derselben lokalen Testseite zusätzlich in Chrome geprüft: tatsächlicher Klick, ausgewählte Grenzen und vier Bildpunkte. Der Edge-Test bestand im selben Lauf ebenfalls. Andere Webseiten, Browser, Zoomstufen und DPI-Konfigurationen sind dadurch nicht allgemein nachgewiesen.

## Alpha 5

Die Vorabversion Alpha 5 basiert auf Commit `41e8bbd` und bestand [GitHub Actions Lauf 41457711642](https://github.com/BomBastikDE/shotkit/actions/runs/41457711642) unter Linux und Windows. Dabei wurden die pixelgenaue Tastaturkorrektur eines manuell gezogenen Bereichs, Abbruch und der normale Mausablauf sowie die Wiederholung des letzten Bereichs über das Tray-Menü geprüft. Tests auf realen Geräten und mit gemischten Skalierungen stehen weiterhin aus.

## Alpha 6

Die Vorabversion Alpha 6 basiert auf Commit `bc8efbe` und bestand [GitHub Actions Lauf 41459734057](https://github.com/BomBastikDE/shotkit/actions/runs/41459734057). Der Windows-Lauf prüfte insbesondere den Klick nach Tastaturkorrektur und das Wiederholen mit Bilddatei und Zwischenablage; Monitorwechsel und gemischte Skalierungen bleiben separate Praxistests.

## Alpha 9

Die Vorabversion Alpha 9 basiert auf Commit `7fba242` und bestand [GitHub Actions Lauf 41475395472](https://github.com/BomBastikDE/shotkit/actions/runs/41475395472) unter Linux und Windows. Der Windows-Lauf prüfte unter anderem das Anlegen, erneute Öffnen und Bearbeiten von Text sowie Rückgängig. Monitorwechsel und gemischte Skalierungen bleiben separate Praxistests.

## Alpha 13

Crate und ausführbare Datei heißen `shotkit`. Neue Release-Dateien verwenden das Schema `shotkit-v2.0.0-alpha.N-windows-x64.exe` mit zugehöriger SHA256-Datei; ältere Releases behalten ihren bisherigen Namen `ShotKit-Rust-…`. CI: [Lauf 41528774127](https://github.com/BomBastikDE/shotkit/actions/runs/41528774127).

## Alpha 14

Die Release-Erstellung für Alpha-Tags läuft nach erfolgreichen Linux- und Windows-Prüfungen automatisch; EXE und SHA256-Datei werden als Pre-Release veröffentlicht. [Tag-Lauf 41530450285](https://github.com/BomBastikDE/shotkit/actions/runs/41530450285).
