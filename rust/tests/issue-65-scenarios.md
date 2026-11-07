# Windows-Szenarien für #65

Stand: 2026-11-07, Abschlusskriterium
[#65](https://github.com/BomBastikDE/shotkit/issues/65#issuecomment-6704228300),
Releaseplan [#72](https://github.com/BomBastikDE/shotkit/issues/72).

Der Workflow `Windows large-image scenario checks` führt die nachfolgenden
Prüfungen nacheinander auf einem interaktiven Windows-2022-Runner aus. Er lässt
sich manuell mit `All` oder einer Szenarionummer starten. Die zusätzlichen
PR-Läufe betreffen nur Änderungen an diesem Workflow und seinen Testskripten.
Vorhandene normale CI-Prüfungen werden nicht ersetzt.

```powershell
./tests/run-resource-scenarios.ps1 -ExePath target/release/shotkit.exe
```

| # | Automatischer Nachweis bei erfolgreichem Lauf | Verbleibende Grenze |
|---|---|---|
| 1 | GUI-Speichern eines synthetischen 256 × 30 000 großen Ergebnisses als PNG und JPEG; vollständige Dekodierung und Pixelprüfung beider Ausgaben, Quelldatei unverändert, dasselbe Dokument bleibt offen. | **Teilabdeckung:** Die tatsächliche Scrollaufnahme bis 30 000 Pixel wird damit nicht geprüft. Ein erfolgreicher Export ist kein Nachweis der Aufnahme. |
| 2 | Vollständiges RGBA8-PNG mit 16 384 × 8 192 Pixeln, exakt 536 870 912 dekodierte BGRA-Bytes. Zweimaliger Öffnungsversuch zeigt den erwarteten deutschen Erhaltungs-Hinweis beim Budgetfehler. Dasselbe vorher bereits geöffnete, ungespeicherte Dokument, seine Pixel und Undo/Redo bleiben erhalten; beide Quelldateien bleiben unverändert. | **Teilabdeckung:** Dieser konkrete PNG-Typ wird schon vor der Übernahme abgelehnt. Schwärzen, Weichzeichnen und Speichern des Grenzbildes werden nicht erreicht. Andere Farbschemata/Codecs können einen anderen Bedarf haben; hieraus folgt keine allgemeine Unmöglichkeit des Szenarios. |
| 3 | RGBA8-PNG mit 16 384 × 8 193 Pixeln (64 KiB über 512 MiB) wird mit „Das dekodierte Bild überschreitet 512 MiB.“ abgewiesen. Derselbe Dokument-/Pixel-/Verlaufs- und Quelldateierhalt wie bei #2. | Kein allgemeiner Nachweis aller Codec-Grenzfälle. |
| 4 | Fünf gleichzeitig offene, unabhängige 6 000 × 4 000 große Dokumente, jeweils 96 000 000 BGRA-Bytes. Eines wird über GUI-Speichern unter exportiert und vollständig auf identische Pixel geprüft. Alle fünf HWNDs bleiben erhalten, die anderen Titel und alle fünf Quelldatei-Hashes unverändert. | Die Bilder sind synthetische, identische und noch nicht annotierte Inhalte in verschiedenen Dateien. Unterschiedliche Bilder, gleichzeitige Exporte und weitere Bearbeitungsfolgen sind nicht Teil dieses festen Tests. |
| 5 | Vorhandener `windows-crop-history-confirmation-smoke.ps1`: tatsächliche Überschreitung des 128-MiB-Verlaufsbudgets, Ablehnen/Bestätigen, vollständige Pixel-Fingerprints, Undo/Redo und Erhalt des Dokuments. Zweiter Lauf mit `-WithoutUndo` prüft einen einzelnen Snapshot oberhalb des Budgets. | Keine vollständige Prüfung aller Bearbeitungswerkzeuge; die vorhandenen Kriterien/Assertions bleiben unverändert. |

**#65 bleibt offen**, solange die vollständigen Szenarien 1 und 2 nicht
nachgewiesen oder vom Owner ausdrücklich anders abgegrenzt sind. Der Runner
meldet deshalb auch bei fehlerfreien automatisierten Prüfungen
`ISSUE_65_ACCEPTANCE_INCOMPLETE`. Ein grüner Workflow allein erfüllt nicht
alle fünf Abschlusskriterien. Bis zum echten Windows-Lauf sind diese neuen
Prüfungen lediglich implementiert, nicht als bestanden nachgewiesen.

## Grenzen und Ressourcen

`image_io.rs::import_peak_reservation` bucht zwei Decoder-Raster, das finale
BGRA-Raster und 16 MiB Arbeitsreserve. Für das konkrete RGBA8-PNG in #2 sind
das `3 × 536870912 + 16777216 = 1627389952` Bytes, zusätzlich zur kleinen
Datei-/Decoderbuchung. `app/import_budget.rs` erlaubt regulär 896 MiB
(`939524096` Bytes). Der Test verlangt diese konkrete Ablehnung und die
verständliche deutsche Ergänzung; er ersetzt das Grenzbild nicht durch ein
kleineres. Decoder mit weniger Ausgabekanälen können andere Buchungen haben.

Die PNG-Grenzfixtures werden zeilenweise mit vollständigem zlib-Datenstrom,
PNG-CRC und unabhängig nachgezählter Dekompressionslänge erzeugt. Der Test
legt dafür kein 512-MiB-Bitmap im PowerShell-Prozess an. Die bestehenden
Generierungs-, Zeit- und Sicherheitsgrenzen bleiben bestehen. Die fünf großen
Dokumente werden ausschließlich in einem eigenen, temporären Benutzerkontext
geöffnet; der Test verweigert die Interaktion mit vorgefundenen Instanzen.

Für PNG bleibt die Pixelprüfung exakt. Nur das zusätzliche JPEG verwendet ein
Graustufenmuster aus konstanten 16 × 16-Blöcken und prüft jedes dekodierte
RGB-Byte mit einer maximalen Abweichung von 3; Alpha muss 255 sein. Die
ursprüngliche zufällige PNG-Fixture und ihre exakte Prüfung bleiben erhalten.

Der Runner protokolliert pro gestarteter ShotKit-Instanz den vom Betriebssystem
gelieferten Working-Set-Spitzenwert sowie das Maximum der beobachteten privaten
Bytes (Polling etwa alle 50 ms). Der Exporttest misst zusätzlich phasenweise
private Bytes im vorhandenen Hintergrundsampler. Verpasste Zwischenpeaks sind
möglich; dies ist keine harte RAM-Garantie und kein Pass/Fail-Schwellenwert.
Vorhandene Sicherheits-Tripwires des Importtests werden nicht entfernt.

Jeder Teiltest läuft in einem frischen PowerShell-Prozess, längstens 300
Sekunden; die Anwendung und Fixtures werden durch dessen bestehende
Eigentums-/Cleanup-Prüfungen verwaltet. Bei einem harten Timeout wird nur der
vom Runner gestartete Prozessbaum beendet. Nicht mehr sicher bereinigbare
Fixtures bleiben auf dem kurzlebigen Runner. Keine neuen Abhängigkeiten,
keine Systembereinigung und keine Produktänderungen.

## Lokale Vorprüfung dieses Zwischenstands

`git diff --check`, YAML-Laden des neuen Workflows und `cargo fmt --check`
im vorhandenen ARM64-Rust-Dockerimage sind fehlerfrei. Ein lokaler PowerShell-Parser-/C#-
Kompilierungsversuch mit dem bereits vorhandenen PowerShell-Dockerimage
scheiterte vor der Ausführung: amd64-Image auf aarch64-Host,
`exec /usr/bin/pwsh: exec format error`. Somit liegen für diesen Zwischenstand
noch keine ausgeführte PowerShell-Syntaxprüfung und keine Windows-GUI-
Testergebnisse vor. Der neue Windows-Workflow muss diese Nachweise liefern;
keine Werkzeuge wurden nachinstalliert.
