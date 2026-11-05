# Änderungsprotokoll der Rust-Version

## Beta 1 – Stand vor Veröffentlichung

Der Entwicklungszweig wird für `v2.0.0-beta.1` vorbereitet. Seit Alpha 20 wurden unter anderem Auswahlkorrektur und Scroll-Diagnose ergänzt sowie Fehler und Tests bearbeitet. Der [CI-Lauf 42056863647](https://github.com/BomBastikDE/shotkit/actions/runs/42056863647) bestand auf Linux und Windows mit 43 Windows-Szenarien für den Vorbereitungscommit `db5372e`. Die Freigabe des Release-Tags steht noch aus. Gemischte DPI, mehrere Monitore und visuelle Owner-Abnahmen bleiben offen; Verpixelung ist keine sichere Schwärzung.

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

## Alpha 16

Die Editorvorschau verwendet einen Cache für unveränderte Formen, begrenzt Neuzeichnungen auf betroffene Bereiche und berechnet das Schachbrett nur für transparente Pixel. Der [Windows-Release-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41537579633) maß bei vier Formen und drei Durchläufen für 1920 × 1080 Pixel 83,59 → 25,24 ms und für 1200 × 10.000 Pixel 435,56 → 127,93 ms je Vorschau-Berechnung. Diese Messung umfasst Rendering und Vorschau-Aufbereitung, nicht die abschließende Windows-Bildschirmausgabe. Der Owner meldete eine deutlich flüssigere Bedienung; schnelles Freihandzeichnen blieb auffällig ([Issue #2](https://github.com/BomBastikDE/shotkit/issues/2)).

## Alpha 17

Die vorhandenen ShotKit-Symbole werden höher aufgelöst gezeichnet und geglättet ausgegeben. Der [Windows-Release-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41539042247) bestand mit 23 von 23 Smoke-Szenarien, einschließlich eines Bildtests auf Zwischenstufen an Symbolkanten. Die EXE ist 1.135.616 Bytes groß, 1.536 Bytes mehr als Alpha 16; neue Abhängigkeiten wurden nicht hinzugefügt. Die visuelle Abnahme durch den Owner steht aus ([Issue #3](https://github.com/BomBastikDE/shotkit/issues/3)).

## Alpha 18

Freihandstriche werden auf einen begrenzten Bildbereich gerendert; eindeutig zuordenbare Zwischenpunkte aus der begrenzten Windows-Maushistorie werden übernommen. Der [Windows-Release-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41541041605) maß für einen Strich mit 600 Punkten und drei Durchläufen 28,15 → 18,05 ms bei 1920 × 1080 und 146,04 → 79,12 ms bei 1200 × 10.000 Pixeln. Die verglichenen Vorschaubilder waren pixelgleich; abschließende Windows-Bildschirmausgabe und realer Schnellzeichentest sind darin nicht enthalten. Alle 23 Smoke-Szenarien bestanden. EXE und SHA256-Datei sind im [Pre-Release](https://github.com/BomBastikDE/shotkit/releases/tag/v2.0.0-alpha.18) enthalten.

## Alpha 19

Die verkleinerte Editoransicht verwendet eine höherwertige Skalierung; während des Zeichnens bleibt der schnelle Darstellungsweg aktiv. Die 1:1-Ausgabe wird nicht geglättet ([Issue #4](https://github.com/BomBastikDE/shotkit/issues/4)). [Release-Lauf 41678872270](https://github.com/BomBastikDE/shotkit/actions/runs/41678872270) bestand unter Linux und Windows mit 23 von 23 Smoke-Szenarien. Der Owner meldete anschließend, dass nach einem Klick auf Rückgängig die Symbole der Werkzeugleiste verschwanden ([Issue #1](https://github.com/BomBastikDE/shotkit/issues/1)).

## Alpha 20

Die Korrektur für [Issue #1](https://github.com/BomBastikDE/shotkit/issues/1) schützt die Werkzeugleiste vor dem Übermalen durch den Editor. Der [CI-Lauf 41897317737](https://github.com/BomBastikDE/shotkit/actions/runs/41897317737) bestand unter Linux und Windows; der blockierende Editor-Smoke-Test prüft sichtbare Symbolpixel nach Undo, Redo und erneutem Undo. Der [Release-Lauf 41902749425, Versuch 2](https://github.com/BomBastikDE/shotkit/actions/runs/41902749425/attempts/2) bestand unter Linux und Windows; das [Pre-Release](https://github.com/BomBastikDE/shotkit/releases/tag/v2.0.0-alpha.20) enthält EXE und SHA256-Datei. Im ersten Versuch erreichte die Edge-UI-Automation das Zeitlimit; die Ursache ist nicht abschließend geklärt. Die Korrektur muss auf dem Rechner des Owners bestätigt werden.

## Übergabe 2. November 2026

- Erledigt: Alpha 16, 17 und 18 nach grüner Linux-/Windows-CI veröffentlicht; C# bleibt stabile Hauptversion.
- Erledigt: Die Textleisten-Meldung war eine noch laufende alte EXE; der Owner bestätigte die Symbolleiste nach Neustart.
- Offen: Owner testet Alpha 18 auf schnelles Freihandzeichnen und beurteilt die geglätteten Symbole.
- Offen: Die Maushistorie umfasst höchstens 64 Punkte; abgelaufene oder nicht eindeutige Verläufe werden nicht ergänzt.
- Nächster Schritt: Die fünf als „fehlt“ markierten Funktionen in [PARITY.md](PARITY.md) bearbeiten, einschließlich schmaler Aktionsleiste.
- Danach: Die 20 Zeilen „implementiert, nicht belegt“ priorisiert testen und den Symbolleisten-Test wieder verbindlich machen.
- Bei der nächsten ohnehin nötigen CI-Änderung checkout und upload-artifact auf aktuelle Hauptversionen prüfen und aktualisieren.
- Reale Tests mit gemischter DPI, mehreren Monitoren und weiteren Browsern sowie Entscheidungen zu main, legacy und Code-Signing bleiben beim Owner.

## Nach Alpha 20 – in Entwicklung

- Das Undo-Speicherbudget berücksichtigt nach Bildänderungen auch das bisherige Bild ([#47](https://github.com/BomBastikDE/shotkit/issues/47)); schmaler bewegter Inhalt wird bei der Scrollaufnahme nicht mehr als sicherer Stillstand gewertet ([#48](https://github.com/BomBastikDE/shotkit/issues/48)).
- Der Bildimport begrenzt die tatsächlich gelesenen Bytes; die Messung des Spitzenbedarfs bleibt offen ([#51](https://github.com/BomBastikDE/shotkit/issues/51)). Der interne Beenden-Ablauf nimmt Abschlussnachrichten nur nach bestätigtem Beenden an; Windows-Nachweis folgt ([#49](https://github.com/BomBastikDE/shotkit/issues/49)).
- Eine standardmäßig aktive Einstellung fordert Windows auf, kopierte Bilder vom Zwischenablageverlauf und der Cloud-Synchronisierung auszunehmen. Sie bietet keine Garantie; bereits vorhandene Einträge bleiben erhalten.
- Das neue Werkzeug „Schwärzen“ ist über `S` erreichbar. Es schwärzt den gerenderten Export, während Originalbild und editierbarer Editorzustand erhalten bleiben. Verpixeln ist weiterhin keine sichere Schwärzung.
- Das Werkzeug „Weichzeichnen“ (`W`) ergänzt Verpixeln; seine Stärke folgt der Linienbreite. Beide Filter sind keine sichere Schwärzung und beachten deckende Schwärzungen beim Rendern ([#30](https://github.com/BomBastikDE/shotkit/issues/30)).

- Eine waagerechte oder senkrechte Ziehbewegung ohne Fläche lässt die Bereichsauswahl offen; danach wird der Vorschlag am Endpunkt neu ermittelt ([#27](https://github.com/BomBastikDE/shotkit/issues/27), [#43](https://github.com/BomBastikDE/shotkit/issues/43)).
- Die Auswahllupe zeigt den vergrößerten Bildinhalt wieder statt einer weißen Fläche ([#41](https://github.com/BomBastikDE/shotkit/issues/41)).
- Textdialog und Infofenster passen Maße und Schrift bei Windows-DPI-Wechseln an; simulierte Wechsel sind getestet, reale gemischte DPI bleiben ein Praxistest ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Mausrad und Modifikatortasten werden aus derselben Windows-Ereignisnachricht ausgewertet; der sporadische Schwenkfehler bleibt zur Beobachtung offen ([#42](https://github.com/BomBastikDE/shotkit/issues/42)).
- Strg+Mausrad verwendet die Zoomschrittweite der C#-Referenz; Strg+0 und Strg+9 funktionieren auch mit dem Ziffernblock.

Bei Verpixeln beträgt die Mindestblockgröße 12 statt 6 Pixel; ein Regressionstest prüft die niedrigen Linienstärken. Dies verbessert die grobe Darstellung, garantiert aber keine Unkenntlichkeit oder sichere Schwärzung. [Issue #5](https://github.com/BomBastikDE/shotkit/issues/5) bleibt offen.

- Das Verpixeln-Werkzeug weist darauf hin, dass der Effekt keine sichere Schwärzung bietet ([#5](https://github.com/BomBastikDE/shotkit/issues/5)).
- Der Auswahlrahmen und seine Griffe werden beim Ziehen einer bestehenden Form auch bei aktivem Zeichenwerkzeug angezeigt; der Windows-Test prüft ein Rechteck während des Ziehens ([#23](https://github.com/BomBastikDE/shotkit/issues/23)).
- Windows-Smoke-Tests prüfen die verzögerte Aufnahme nach 3, 5 und 10 Sekunden, den konfigurierten Speicherordner, eine kombinierte Rahmen-/Schatten-Ausgabe mit Undo sowie die vier Speicherformate PNG, JPEG, BMP und GIF. Der [CI-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41973151902) bestand mit 32 Szenarien; Grenzen der einzelnen Nachweise stehen in [PARITY.md](PARITY.md).

Für Scroll-Aufnahmen wird `SendInput` verwendet; wenn Windows ein Scroll-Ereignis nicht übernimmt, meldet ShotKit den Fehler. Die konkrete Ursache einer Ablehnung lässt sich aus dem Rückgabewert nicht sicher bestimmen ([Issue #7](https://github.com/BomBastikDE/shotkit/issues/7)).

- Beim Zuschneiden werden beide gewählten Randpixel berücksichtigt ([#18](https://github.com/BomBastikDE/shotkit/issues/18)).
- Undo/Redo hebt die Auswahl auf, wenn Hinzufügen oder Entfernen die Anzahl der Formen ändert ([#19](https://github.com/BomBastikDE/shotkit/issues/19)).
- Beim ersten Speichern geöffneter JPEG-, GIF-, TIFF- und anders benannter Bilder mit diesen Inhalten wird ein neuer Dateiname verlangt; EXIF-Ausrichtung wird beim Laden angewendet. Metadaten und Animation bleiben beim Export nicht erhalten ([#20](https://github.com/BomBastikDE/shotkit/issues/20)).
- Scroll-Aufnahmen brechen nach wiederholt nicht eindeutig zugeordneten Bildern ab und weisen auf unsichere Übergänge hin ([#21](https://github.com/BomBastikDE/shotkit/issues/21)).
- Release-Tags müssen zur Cargo-Version passen ([#24](https://github.com/BomBastikDE/shotkit/issues/24)); der Edge-UIA-Test protokolliert aufeinanderfolgende Abfrageschritte bei einem Timeout ([#25](https://github.com/BomBastikDE/shotkit/issues/25)).
- GitHub Actions werden auf konkrete Commit-IDs festgelegt; checkout sowie Upload/Download verwenden aktuelle Hauptversionen. Dependabot prüft Actions und Cargo-Abhängigkeiten ([#10](https://github.com/BomBastikDE/shotkit/issues/10)).
- Schnelle Doppelklicks auf der Zeichenfläche behalten das aktive Werkzeug; ein Doppelklick auf ein Textobjekt öffnet weiterhin dessen Bearbeitung ([#22](https://github.com/BomBastikDE/shotkit/issues/22)).
- Der Release-Workflow akzeptiert vorbereitend Alpha- und Beta-Tags als Pre-Releases und prüft vor Veröffentlichung den Versionsabgleich. Eine Beta wurde noch nicht veröffentlicht.
- Beim Ziehen und Ändern vorhandener Formen ersetzt die Vorschau die alte Form und führt den Auswahlrahmen mit; neue Formen werden weiterhin zusätzlich angezeigt ([#23](https://github.com/BomBastikDE/shotkit/issues/23)).
- Automatisch gespeicherte PNG-Dateien werden erst nach vollständigem Schreiben unter ihrem endgültigen Namen veröffentlicht; bestehende Dateien werden dabei nicht ersetzt ([#25](https://github.com/BomBastikDE/shotkit/issues/25)).
- Das Verschieben der Bildansicht per Mausrad bleibt an den Bildrändern begrenzt und reagiert bei Richtungswechsel sofort ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Ist die Windows-Zwischenablage kurz belegt, wiederholt ShotKit nur das Öffnen mit begrenzter Wartezeit; Schreiboperationen werden nicht erneut ausgeführt ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Verliert eine laufende Bereichsauswahl die Maussteuerung, wird der unvollständige Vorgang abgebrochen, statt den Ziehzustand festzuhalten ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Ergebnisse der Scroll-Aufnahme bleiben bei fehlgeschlagener Abschlussnachricht erhalten und werden über einen Timer auf dem UI-Thread verarbeitet; Fenstergenerationen schützen vor veralteten Zielen ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Der Windows-Smoke-Test prüft die sichtbare Formvorschau während eines gehaltenen Ziehvorgangs; ein synthetischer Screenshot wird als CI-Artefakt gesichert ([#23](https://github.com/BomBastikDE/shotkit/issues/23)).
- Die Bestätigung für UNC-Dateien aus einer zweiten Instanz weist vor dem Öffnen auf den möglichen Serverkontakt und die automatische Authentifizierung hin; zugeordnete Netzlaufwerke mit Laufwerksbuchstaben werden nicht gesondert erkannt ([#12](https://github.com/BomBastikDE/shotkit/issues/12)).
- Der Abschluss einer Bereichsauswahl weckt die Nachrichtenschleife auch nach synchronen Abbruchnachrichten; ein Windows-Smoke-Test prüft anschließend eine erneute Auswahl ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Beim Speichern bleiben Punkte im Dateinamen erhalten; die gewählte Bildendung wird ergänzt und vor dem Ersetzen des endgültigen Zielpfads nachgefragt ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Das Tray-Menü sendet nach dem Schließen die von Windows vorgesehene Abschlussnachricht; ein Smoke-Test prüft Außenklick und erneutes Öffnen ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- „Speichern unter“ ersetzt keine zwischen Auswahl und Schreibabschluss neu angelegte Datei; die Anwendung meldet die Kollision und erhält das vorhandene Ziel ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Sehr kurze, dicke Pfeile werden ohne über die Spitze hinausragenden Schaft gerendert; die Form reicht bis zum gesetzten Startpunkt ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
- Ein Rechtsklick bricht die laufende Bereichsauswahl wie in ShotKit C# v1.2.0 ohne Aufnahme ab.
- Ein zusätzlicher Windows-Smoke-Test prüft die Übernahme eines unveränderten Browservorschlags mit `Enter`.
- Der Browser-Smoke-Test begrenzt den Start seines UI-Automation-Prüfprozesses und die anschließende UIA-Prüfung getrennt, damit ein langsamer Prozessstart nicht das UIA-Zeitbudget aufbraucht ([#25](https://github.com/BomBastikDE/shotkit/issues/25)).
- Die Editorleiste bleibt bei schmalem Fenster einzeilig; Aufnahme, Kopieren, Speichern und Datei bleiben rechts sichtbar, weitere Aktionen wandern stufenweise in ein Menü. `Strg+0` und `Strg+9` entsprechen nun den Beschreibungen Originalgröße und Einpassen.
- Vor einer Scroll-Aufnahme wird die asynchrone Aktivierung des Ziel-Fensters begrenzt synchronisiert; Eingaben bleiben ohne bestätigten Vordergrund gesperrt.
- Ein Windows-Test prüft die Wiederholung beim Öffnen der Zwischenablage mit einer echten, von einem zweiten Prozess gehaltenen Sperre ([#27](https://github.com/BomBastikDE/shotkit/issues/27)).
