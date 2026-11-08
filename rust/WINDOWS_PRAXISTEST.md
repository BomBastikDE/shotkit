# ShotKit: Windows-Praxistest

Diese Anleitung ergänzt die automatischen Tests (siehe [Testprotokoll](TESTPROTOKOLL.md)). Sie deckt ab, was die CI auf einem einzelnen virtuellen Bildschirm nicht sehen kann: echte Monitore und Skalierungen, echte Webseiten, das sichtbare Erscheinungsbild und das Gefühl bei der Bedienung.

Bitte mit Testbildern und nicht mit vertraulichen Inhalten prüfen. Die EXE kommt von der [Releases-Seite](https://github.com/BomBastikDE/shotkit/releases) und ist nicht signiert.

## Ausgangslage notieren

- Windows-Version und -Build, Anzahl und Anordnung der Monitore und ihre Skalierung (Einstellungen → System → Anzeige). Bei nur einem Monitor die Mehrmonitor-Punkte als „nicht geprüft“ markieren.
- Eine eventuell noch laufende ältere ShotKit-Version (auch C#) über das Tray-Menü vollständig beenden. Sonst aktiviert der Start die alte Version.

## Kerntest (etwa 15 Minuten)

| # | Schritt | Erwartung |
|---|---|---|
| 1 | Neue EXE starten, **ohne** vorhandene Einstellungen (`%APPDATA%\ShotKitRust` vorher umbenennen) | Einmaliger Hinweis „Willkommen bei ShotKit“, danach Symbol im Infobereich. Zweiter Start: **kein** Hinweis. |
| 2 | Tray → „Über ShotKit“ | Versionsnummer 2.1.0, Name, Website-Schaltfläche öffnet den Browser. Kein „Rust“ oder „Prototyp“ in Titeln. |
| 3 | `Druck` oder `F11`: Bereich aufziehen, dann im Editor ein Fenster per Vorschlag übernehmen | Bildinhalt und Ränder stimmen; Editor öffnet sich. |
| 4 | Im Editor: Pfeil, Rechteck, Text, Sprechblase, Stempel zeichnen; Werkzeug mehrmals wechseln | Symbole scharf; Fenstertitel zeigt weiter den Dateinamen und **keinen** englischen Werkzeugnamen (#79). |
| 5 | Verpixeln (`B`) über Text ziehen | Grobe Blöcke (mindestens 48 px), Text nicht lesbar; Hinweis „keine sichere Schwärzung“ im Tooltip. |
| 6 | Schwärzen (`S`) über Text ziehen, als PNG speichern, PNG in einem anderen Programm öffnen | Bereich vollständig schwarz. |
| 7 | `Strg+Z` / `Strg+Y` mehrmals | Symbole der Werkzeugleiste bleiben sichtbar (#1). |
| 8 | `F10`: Scrollaufnahme auf einer langen Webseite bis zum Ende | Keine fehlenden oder doppelten Zeilen an den Übergängen; Ende erreicht. |
| 9 | `F10` erneut, nach einigen Schritten **Maus bewegen** | Aufnahme endet, Teilbild bleibt erhalten, Meldung nennt den Grund. |
| 10 | Bei mehreren Monitoren mit **unterschiedlicher Skalierung**: Schritt 8 auf jedem Monitor | Aufnahme läuft ohne Mausberührung bis zum Ende (#6). Bricht sie mit „Maus bewegt“ ab, Monitoranordnung und Skalierungen notieren. |
| 11 | Editor auf einen anderen Monitor ziehen, zeichnen, speichern | Darstellung und Ergebnis stimmen. |
| 12 | Tray → Einstellungen → „Mit Windows starten“ einschalten, abmelden und wieder anmelden | ShotKit startet ohne Hinweisfenster. |
| 13 | Tray → Einstellungen → **„Mauszeiger mit aufnehmen“** einschalten, mit `F9` aufnehmen; im Editor den Zeiger verschieben und löschen; mit automatischem Speichern wiederholen | Zeiger an der richtigen Stelle, als Objekt verschieb- und löschbar; „Rückgängig“ entfernt ihn nicht; die gespeicherte PNG enthält ihn. Ohne Option: kein Zeiger. *(Der CI-Runner hat keine Maus und kann das nicht prüfen, #36.)* |

## Ausführliche Prüfungen (optional)

**Bereichsauswahl:** Beim Loslassen `Umschalt` halten. Pfeiltasten verschieben um 1 px, `Umschalt`+Pfeil ändert die rechte bzw. untere Kante, `Enter` nimmt auf, `Esc` bricht ab. „Letzten Bereich erneut aufnehmen“ liefert dieselben Grenzen. Nach Trennen eines Monitors muss eine Meldung kommen statt einer anderen Fläche.

**Scrollaufnahme:** Seite mit fester Kopf- und Fußleiste; horizontal scrollbarer Bereich (Richtung „rechts“/„links“). Abbruch mit `Esc` behält das Teilbild.

**Editor:** Formstil (Füllung, gestrichelte Linie, abgerundete Ecken, Pfeilenden), Spotlight, Bildlupe, Weichzeichnen, Leinwandgröße, Rahmen und Schatten. Zuschneiden, speichern, wieder öffnen. Kopieren und mit `Strg+V` einfügen, auch mit transparenten Stellen.

**Große Bilder:** Ein sehr großes Bild öffnen (z. B. ein Foto mit über 80 Megapixeln). Erwartung: verständliche Meldung, kein Absturz, andere offene Bilder bleiben erhalten.

**Absturz beim Speichern:** Nicht absichtlich herbeiführen. Falls ShotKit beim Speichern abstürzt: Beim nächsten Start sollte ein Hinweis auf zurückgebliebene `.shotkit-….tmp`-Dateien erscheinen (#67).

## Ergebnis melden

Für jeden Schritt **bestanden**, **fehlgeschlagen** oder **nicht geprüft** notieren, am einfachsten als Kommentar im Plan-Issue [#72](https://github.com/BomBastikDE/shotkit/issues/72) oder als neues Issue. Bei einem Fehler: Windows-Version, Monitore und Skalierung, Anwendung oder Webseite, Schritte, erwartetes und tatsächliches Verhalten. Screenshots nur ohne vertrauliche Inhalte. Ein erfolgreicher Durchlauf belegt nur die getestete Konfiguration.

## Frühere Rückmeldungen aus Praxistests

| Datum | Version | Meldung | Status |
|---|---|---|---|
| 02.11. | Alpha 15 | Werkzeugleiste mit Text statt Symbolen | Geklärt: Es lief noch eine ältere EXE; die Einzelinstanz-Sperre aktivierte sie. Nach dem Beenden zeigte die neue Version die Symbole. |
| 02.11. | Alpha 15 | Ruckeln beim Zeichnen und Verschieben | **Offen.** Nicht gemessen; mögliche Ursache: Neuberechnung bildgroßer Ebenen je Mausbewegung. Bitte im Kerntest (Schritt 4) mit beurteilen. |
| 02.11. | Alpha 15 | Symbolleiste unscharf ([#3](https://github.com/BomBastikDE/shotkit/issues/3)) | Vom Owner am 05.11. als behoben bestätigt. |
| 03.11. | Alpha 16 | Schnelles Freihandzeichnen ruckelt, Bewegungen fehlen ([#2](https://github.com/BomBastikDE/shotkit/issues/2)) | Vom Owner am 05.11. als behoben bestätigt. |
| 04.11. | Alpha 19 | Symbole verschwinden nach Rückgängig ([#1](https://github.com/BomBastikDE/shotkit/issues/1)) | Korrektur seit Alpha 20, in der CI nach Undo/Redo getestet; Issue geschlossen. Bitte im Kerntest (Schritt 7) bestätigen. |
