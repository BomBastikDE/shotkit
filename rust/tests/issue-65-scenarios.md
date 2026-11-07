# Windows-Szenarien für #65

Stand: 2026-11-07. Maßgeblich sind das Abschlusskriterium und die anschließende
Owner-Präzisierung in [#65](https://github.com/BomBastikDE/shotkit/issues/65),
Releaseplan [#72](https://github.com/BomBastikDE/shotkit/issues/72).

Der Workflow `Windows large-image scenario checks` führt die Prüfungen
nacheinander auf einem interaktiven Windows-2022-Runner aus. Er lässt sich
manuell mit `All` oder einer Szenarionummer starten; `2` startet 2a und 2b.
Die zusätzlichen PR-Läufe betreffen Änderungen am Workflow und seinen
Testskripten. Vorhandene normale CI-Prüfungen werden nicht ersetzt.

```powershell
./tests/run-resource-scenarios.ps1 -ExePath target/release/shotkit.exe
```

| # | Automatischer Nachweis bei erfolgreichem Lauf | Grenze |
|---|---|---|
| 1 | GUI-Speichern eines synthetischen 256 × 30 000 großen Ergebnisses als PNG und JPEG; vollständige Dekodierung und Pixelprüfung beider Ausgaben, Quelldatei unverändert, dasselbe Dokument bleibt offen. | Der Owner hat dieses Szenario ausdrücklich als ausreichend für #65 angenommen. Es prüft keine Scroll-Mechanik; diese bleibt bei den Scroll-Tests (#58). |
| 2a | RGBA8-PNG mit 8 192 × 8 192 Pixeln (256 MiB BGRA) wirklich öffnen, mit S schwärzen und exportieren, mit W überlappend weichzeichnen und erneut speichern. Die Ausgabe muss Schwärzung und gemischte Blur-Pixel zeigen; ausschließlich eine konkrete Budgetfehlermeldung darf den kombinierten Export ersetzen. Bei Budgetablehnung muss ein zusätzlicher Blur-only-Export aus dem aktuellen Dokument das erhaltene W-Objekt nachweisen. Danach prüfen UndoBlur und UndoRedact durch weitere Exporte den vollständigen Erhalt der vorherigen Schwärzung bzw. aller Originalpixel. | Dies ist die vom Owner genannte große, tatsächlich importierbare RGBA-Fixture, keine Suche nach dem mathematisch größten importierbaren Bild. Ein abgewiesener kombinierter Export wird ausdrücklich als solcher protokolliert. |
| 2b | Vollständiges RGBA8-PNG mit 16 384 × 8 192 Pixeln, exakt 536 870 912 dekodierte BGRA-Bytes. Zweimaliger Öffnungsversuch zeigt den erwarteten deutschen Erhaltungs-Hinweis beim Budgetfehler. Dasselbe vorher bereits geöffnete, ungespeicherte Dokument, seine Pixel und Undo/Redo bleiben erhalten; beide Quelldateien bleiben unverändert. | Dieses PNG wird vor Übernahme abgewiesen. Das ist der separat geforderte Grenzfall, kein erfolgreicher 512-MiB-Import. |
| 3 | RGBA8-PNG mit 16 384 × 8 193 Pixeln (64 KiB über 512 MiB) wird mit „Das dekodierte Bild überschreitet 512 MiB.“ abgewiesen. Derselbe Dokument-/Pixel-/Verlaufs- und Quelldateierhalt wie bei 2b. | Kein allgemeiner Nachweis aller Codec-Grenzfälle. |
| 4 | Fünf gleichzeitig offene, unabhängige 6 000 × 4 000 große Dokumente, jeweils 96 000 000 BGRA-Bytes. Eines wird über GUI-Speichern unter exportiert und vollständig auf identische Pixel geprüft. Alle fünf HWNDs bleiben erhalten, die anderen Titel und alle fünf Quelldatei-Hashes unverändert. | Synthetische, identische und noch nicht annotierte Inhalte in verschiedenen Dateien; keine gleichzeitigen Exporte. |
| 5 | Bestehender `windows-crop-history-confirmation-smoke.ps1`: tatsächliche Überschreitung des 128-MiB-Verlaufsbudgets, Ablehnen/Bestätigen, vollständige Pixel-Fingerprints, Undo/Redo und Dokumenterhalt. Zweiter Lauf mit `-WithoutUndo` prüft einen einzelnen Snapshot oberhalb des Budgets. | Keine vollständige Prüfung aller Bearbeitungswerkzeuge; bestehende Assertions unverändert. |

Bis zum echten Windows-Lauf sind die neuen Prüfungen **implementiert, nicht
als bestanden nachgewiesen**. Nur ein erfolgreicher `All`-Lauf meldet
`ISSUE_65_SCENARIO_CHECKS_PASSED`; ein einzelnes ausgewähltes Szenario meldet
weiter `ISSUE_65_ACCEPTANCE_INCOMPLETE`. Das Schließen des Issues benötigt
weiter den überprüften Commit und grüne CI für alle geforderten Fälle.

## Pixel- und Erhaltungsprüfungen

PNG ohne Annotationen wird bytegenau nach vollständiger Dekodierung geprüft.
Das zusätzliche JPEG in Szenario 1 verwendet ein Graustufenmuster aus konstanten
16 × 16-Blöcken und prüft jedes RGB-Byte mit maximaler Abweichung 3; Alpha muss
255 sein. Die ursprüngliche zufällige PNG-Fixture im normalen Exporttest bleibt
unverändert.

2a nutzt ein deterministisches, nicht schwarzes 16 × 16-Schachmuster. Der Test
prüft PNG-Farbschema RGBA8 am Header, öffnet das große Bild und zeichnet die
Annotationen über die echten S-/W-Tasten und Mausgesten im maximierten Editor.
Die Schwärzung umfasst ungefähr (1024,1024) bis (2048,2048), Blur überlappt von
(1536,1536) bis (2560,2560). Eine feste 64-Pixel-Randzone berücksichtigt die
Bildschirm-/Bildkoordinatenrundung; der Viewport muss mindestens 256 Pixel breit
sein. Innen muss jedes Schwärzungspixel deckend schwarz sein; außerhalb beider
Randzonen muss jedes Pixel exakt dem Original entsprechen. Bei erfolgreichem
kombiniertem Export müssen mindestens 1000 Pixel außerhalb der Schwärzung echte
gemischte Grauwerte aufweisen. UndoBlur muss zusätzlich denselben SHA-256 über
**alle dekodierten Pixel** wie der erste Schwärzungsexport liefern; UndoRedact
muss wieder jedes Originalbyte ergeben.

Bei kombinierter Speicherablehnung müssen ein deutscher Speicherfehler mit
Budgetbezug, das unveränderte Quellbild und der erste gültige Schwärzungsexport,
ein weiterhin schmutziges Editorfenster mit identischem HWND und das Fehlen
einer veröffentlichten Fehlerausgabe nachgewiesen werden. Zusätzlich wird **aus genau diesem aktuellen Dokument** die Redaktionsform
vorübergehend mit V-Auswahl und Entf entfernt. Der erhaltene Gaussian-Blur muss
als vollständiges 8192²-PNG exportierbar sein: mindestens 1000 gemischte Pixel
im Blur-Inneren, deckendes Alpha und jedes Pixel außerhalb seiner Randzone
exakt original. Damit genügt weder ein alter Export noch ein verlorenes oder
leeres Blur-Objekt. Die Löschung wird anschließend rückgängig gemacht; erst
danach folgen UndoBlur mit identischem vollständigem Pixel-Hash der ersten
Schwärzung und UndoRedact mit allen Originalpixeln. Diese Exporte bleiben
Pflicht; ein Dialog oder ein noch vorhandenes Fenster allein beweist keinen
Dokumenterhalt. Der kombinierte Rasterexport selbst bleibt bei dieser
Budgetablehnung unbewiesen und wird auch nicht als erfolgreich gemeldet. Andere Speicherfehler und Timeouts
schlagen fehl.

## Grenzen und Ressourcen

`image_io.rs::import_peak_reservation` bucht zwei Decoder-Raster, das finale
BGRA-Raster und 16 MiB Arbeitsreserve. Für RGBA8 in 2b sind das
`3 × 536870912 + 16777216 = 1627389952` Bytes; reguläre Importe erlauben
896 MiB (`939524096` Bytes). 2a benötigt für seine Rasterbuchung 784 MiB.
Weitere Formate können andere Buchungen haben. Dies ist keine Prozess-RAM-Grenze.

Die Ablehnungsfixtures werden zeilenweise mit vollständigem zlib-Strom, PNG-CRC
und unabhängig nachgezählter Dekompressionslänge erzeugt. Die vorhandenen
Generierungs-, Zeit- und Sicherheitsgrenzen bleiben erhalten. 2a erzeugt seine
256-MiB-Fixture über ein eigenes GDI-Bitmap; dessen Speicher gehört zum Testhost,
nicht zur separat gemessenen ShotKit-Instanz. Alle Tests verweigern vorgefundene
Instanzen und verwenden eigene temporäre Benutzerkontexte.

Der Runner protokolliert pro gestarteter ShotKit-Instanz den nativen
Working-Set-Spitzenwert sowie das Maximum beobachteter privater Bytes (Polling
etwa 50 ms). Der Exporttest misst private Bytes zusätzlich phasenweise mit
Hintergrundsampler. Zwischenpeaks können fehlen; es gibt keinen Speicher-
Pass/Fail-Schwellenwert. Bestehende Import-Sicherheits-Tripwires bleiben erhalten.

Jeder Teiltest läuft in einem frischen PowerShell-Prozess, höchstens 300
Sekunden; 2a erhält wegen Fixture und mehreren vollständigen 256-MiB-Exporten
600 Sekunden und bis zu 150 Sekunden pro UI-Wartepunkt. Ein Timeout beendet
nur den vom Runner gestarteten Prozessbaum. Unsicher bereinigbare Fixtures
bleiben auf dem kurzlebigen Runner. Keine neuen Abhängigkeiten, keine
Systembereinigung und keine Produktänderungen.

## Lokale Vorprüfung

`git diff --check`, YAML-Laden des Workflows und `cargo fmt --check` im
vorhandenen ARM64-Rust-Dockerimage prüfen diesen Testskriptstand. Eine
PowerShell-Syntax-/C#-Kompilierung ist lokal weiterhin nicht verfügbar: Das
vorhandene PowerShell-Dockerimage ist amd64 auf einem aarch64-Host und startet
mit `exec format error`. Der Windows-Workflow muss die ausführbare Syntax,
die GUI-Abläufe und die Ressourcenmessungen noch belegen. Es wurden keine
Werkzeuge nachinstalliert.
