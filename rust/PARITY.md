# Parität: ShotKit Rust gegenüber C# v1.2.0

Referenz: C#-Commit `cd5fff3a6ec32cb4521edae003dc3a85dd269cd5`, insbesondere `README.md` und `src/Program.cs`, `CaptureOverlay.cs`, `ScreenCapture.cs`, `ScrollCapture.cs`, `EditorForm.cs`, `EditorCanvas.cs`, `Shapes.cs`, `Settings.cs` und `TextInputDialog.cs` **dieses Commits**. Der Rust-Stand liegt im Verzeichnis dieser Datei.

## Bedeutung der Statuswerte

- **fertig:** Der genannte Funktionsumfang ist implementiert und für die ausdrücklich genannte Testkonstellation nachgewiesen. Das belegt keine vollständige Gleichheit außerhalb dieser Konstellation.
- **implementiert, nicht belegt:** Der Quellcode enthält das Verhalten; der angegebene Test belegt den vollständigen beschriebenen Umfang noch nicht.
- **fehlt:** Das beschriebene Verhalten ist nicht oder nicht vollständig implementiert. Unklarer Umfang wird im Text konkret benannt.
- **bewusst anders:** Eine dokumentierte Abweichung ist bereits im Rust-Stand vorhanden. Ob sie für Beta-Parität akzeptiert wird, wird hier nicht vorausgesetzt; bestehende Nutzeranforderungen und die Referenz sind weiter maßgeblich.

Testdateien beziehen sich auf `tests/`, Rust-Unit-Tests auf `src/`. Ein angegebener Lauf belegt nur den im Test geprüften Fall. Der erfolgreiche [Windows-/Linux-Lauf für Alpha 9](https://github.com/BomBastikDE/shotkit/actions/runs/41475395472) prüfte den Commit `7fba242`; spätere Änderungen müssen erneut auf dem jeweiligen Commit grün sein. Der [vollständige Windows-/Linux-Lauf für Commit `1877362`](https://github.com/BomBastikDE/shotkit/actions/runs/41516470659) bestand einschließlich des damals verbindlichen Toolbar-Tests. Im Folgecommit ist der Toolbar-Test vorübergehend nicht blockierend und muss vor Beta wieder verbindlich grün sein.

## Start und Aufnahme

| Funktion der C#-Referenz | Status | Rust-Test / Beleg | Stand / Grenze |
|---|---|---|---|
| Eine EXE starten, Tray-Menü und Einzelinstanz | fertig | `windows-instance-smoke.ps1`; Alpha-9-Lauf | Einzelinstanz und Weiterleitung im Test; echte Geräte separat. |
| Globale Tastenkürzel einschließlich Druck-Modifikatoren und Ausweichkürzeln | implementiert, nicht belegt | `hotkey_action::tests::print_screen_matrix_matches_csharp_reference_and_dispatch_actions` prüft Tasten, Modifier und Aktionen; `windows-smoke.ps1` prüft einen Aufnahmeweg | Registrierung und Konfliktbehandlung für jedes Kürzel sind unter Windows noch nicht vollständig nachgewiesen. |
| Bildschirm unter Mauszeiger aufnehmen | implementiert, nicht belegt | `capture_targets.rs`-Grenztests; Windows-Test für alle Monitoranordnungen fehlt | Implementierung vorhanden; mehrere echte Monitore sind Owner-Praxistest. |
| Alle Bildschirme aufnehmen | implementiert, nicht belegt | `capture_targets.rs`-Grenztests; Mehrmonitor-Windows-Test fehlt | Implementierung vorhanden; Mehrmonitor-Nachweis fehlt. |
| Aktives Fenster aufnehmen | implementiert, nicht belegt | Windows-Test für tatsächlich aktives Fremdfenster fehlt | Implementierung vorhanden; die Übernahme des korrekten Fensters braucht direkten Nachweis. |
| Bereich mit Maus aufziehen | fertig | `windows-smoke.ps1`; Alpha-9-Lauf | Interaktiver Einzelbildschirm-Test mit gespeicherter Aufnahme. |
| Fenster / Unterelement per Maus auswählen | implementiert, nicht belegt | `windows-hover-smoke.ps1`, `windows-browser-hover-smoke.ps1`; [Edge-/Chrome-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41461190364) | Vorschläge für verschachtelte Bedienelemente und Browser sind getestet. Das vollständige Referenzverhalten für `Strg` ist nicht belegt; Rust verwendet `Strg` für das größere Fenster. |
| Lupe mit Koordinaten und Farbwert | implementiert, nicht belegt | Direkter Windows-Test für dargestellte Werte fehlt | Implementierung vorhanden; Anzeige und Werte sind noch nicht direkt belegt. |
| Overlay: Pfeiltasten bewegen Mauszeiger um 1 / mit Umschalt um 10 Pixel | fehlt | `windows-keyboard-selection-smoke.ps1` testet stattdessen Rust-Bereichskorrektur | Andere Bedienung ist vorhanden; Referenz-Mauszeigerbewegung ist nicht nachgewiesen. |
| Overlay: Enter übernimmt Ziel; Esc / Rechtsklick bricht ab | fertig | `windows-keyboard-selection-smoke.ps1`; `windows-browser-hover-smoke.ps1 -EnterAccept`; [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41937859662) | Enter für einen unveränderten Vorschlag ist auf einer lokalen Edge-Testseite geprüft; Esc und Rechtsklick sind im Auswahl-Smoke-Test geprüft. |
| Verzögerte Aufnahme 3/5/10 Sekunden | implementiert, nicht belegt | `delayed_capture.rs`-Unit-Tests; direkter Windows-Test aller drei Verzögerungen fehlt | Timerlogik getestet; Tray-Ablauf aller Zeitwerte offen. |

## Scrollaufnahme

| Funktion der C#-Referenz | Status | Rust-Test / Beleg | Stand / Grenze |
|---|---|---|---|
| Auswahl von Bereich / Fenster für Scrollaufnahme | implementiert, nicht belegt | `windows-scroll-smoke.ps1`, `windows-browser-scroll-smoke.ps1` | Manuell gewählter Bereich getestet; Fenster-/Unterelementauswahl und Referenz-Modifier noch nicht vollständig belegt. |
| Nach unten scrollen und zusammensetzen | fertig | `windows-scroll-smoke.ps1`, `windows-browser-scroll-smoke.ps1`; [Abbruch-/Browserlauf](https://github.com/BomBastikDE/shotkit/actions/runs/41464599131) | Synthetisches Fenster und lokale Edge-Testseite; keine Aussage über jede Webseite. |
| Nach rechts und links scrollen | fertig | `windows-horizontal-scroll-smoke.ps1`; `scroll_capture.rs`-Tests `rightward_capture_rotates_back_to_output_orientation`, `leftward_capture_starts_at_right_edge_and_stitches_back_in_page_order` | Testet beide Richtungen anhand kontrollierter Fenster-/Bilddaten. |
| Esc: Teilbild nach Beginn erhalten | fertig | `windows-scroll-abort-smoke.ps1`; [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41464599131) | Interaktiver Abbruch nach verarbeitetem Schritt; Testbild und Bildzeilen geprüft. |
| Mausbewegung: Teilbild nach Beginn erhalten | implementiert, nicht belegt | `scroll_capture.rs::mouse_movement_after_a_successful_stitch_keeps_the_pixel_exact_prefix` | Algorithmustest vorhanden; interaktiver Windows-Abbruch durch Mausbewegung fehlt. |
| Feste Kopf-/Fußleisten und Nahtstellen | fertig | `scroll_capture.rs::sticky_header_and_footer_are_not_duplicated_across_stitched_seams`; `windows-browser-scroll-smoke.ps1 -FixedHeaderFooter` | Kontrollierte Bilddaten und lokale Edge-Testseite; dynamische Webseiten sind nicht allgemein nachgewiesen. |
| Referenzgrenze 30.000 Pixel und Abgleich mit Mehrheitsbändern | implementiert, nicht belegt | `scroll_match.rs`- und `stitch.rs`-Tests; `stitch.rs::MAX_LENGTH` | Die 30.000-Pixel-Grenze ist implementiert; Übereinstimmung der Fehlermeldungen und des gesamten Referenzverhaltens bleibt zu prüfen. |
| Diagnosebilder und Kennwerte bei nicht zuordenbarem Schritt | fehlt | Direkter Windows-Test für Diagnoseablage fehlt | Referenzablage und Rust-Fehlerbehandlung vergleichen; derzeit kein Gleichheitsnachweis. |

## Editor

| Funktion der C#-Referenz | Status | Rust-Test / Beleg | Stand / Grenze |
|---|---|---|---|
| Auswahl, Pfeil, Linie, Rechteck, Ellipse, Freihand | fertig | `editor.rs::creates_with_preview_then_commits_once_and_undo_redoes`; `windows-editor-smoke.ps1`; [Alpha-7-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41466383115) | Exemplarische Objekte und Windows-Zeichengesten; weitere Kombinationen prüfen. |
| Text anlegen und vorhandenen Text bearbeiten | fertig | `editor.rs::text_edit_is_undoable_and_no_change_adds_no_history`; `windows-editor-text-smoke.ps1`; Alpha-9-Lauf | Testet Anlegen, Öffnen, Ändern und Rückgängig. |
| Textmarker | implementiert, nicht belegt | `alpha_render.rs::windows_pixel_tests::highlight_layer_preserves_transparency_and_marker_opacity` | Render-Unit-Test vorhanden; interaktiver Windows-Ausgabetest für das Werkzeug fehlt. |
| Verpixeln | fertig | `windows-editor-pixelate-smoke.ps1`; [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41478213036) | Testet gespeicherte Pixel innerhalb/außerhalb und Undo/Redo. Kein Nachweis, dass Verpixelung sensible Inhalte sicher unkenntlich macht. |
| Fortlaufende Schritt-Nummern | fertig | `editor.rs::text_and_step_are_creatable`; `windows-step-visibility-smoke.ps1`; [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41515669398) | Der Nummerntest bestand für 1–12 auf hellem und dunklem Grund, PNG, Clipboard sowie Undo/Redo. Der Gesamtlauf war wegen des Toolbar-Tests rot; die Owner-Bestätigung steht aus. |
| Objekte wählen, verschieben, an Griffen ändern und löschen | fertig | `editor.rs::moves_as_one_step_and_delete_undo_redo`, `resizes_a_selected_handle_as_one_step`; `windows-editor-handles-smoke.ps1`; [Alpha-8-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41467952056) | Direkter Griff-Test für Rechteck; nicht jede Form einzeln auf Windows durchgetestet. |
| Farbe, Strichbreite und Schriftgröße nachträglich ändern | implementiert, nicht belegt | `editor.rs::selected_style_change_is_undoable_and_redoable` | Modelländerung geprüft; direkter Windows-Test aller Dialoge und Formtypen fehlt. |
| Umschalt: Quadrat/Kreis und 45-Grad-Linien/Pfeile | fertig | `editor.rs::shift_modifier_is_applied_to_preview_and_final_mouse_up`; `windows-editor-smoke.ps1`; Alpha-7-Lauf | Windows-Test prüft Rechteck, Linie und Pfeil; weitere Formen zusätzlich per Unit-Test. |
| Pfeiltasten verschieben ausgewähltes Objekt | implementiert, nicht belegt | `editor.rs::nudge_is_one_undoable_edit_and_requires_a_selection` | Modelltest vorhanden; direkter Windows-Tastaturtest für Objektverschiebung fehlt. |
| Werkzeugleiste: feste Aktionen und Überlauf bei schmalem Fenster | bewusst anders | C#: `src/EditorForm.cs::ApplyResponsive` und `ApplyStage`; Rust: `toolbar_layout.rs`, `windows-toolbar-visual-smoke.ps1 -ResponsiveOnly`; [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41945268715) | Stufenweises Überlaufmenü und feste Aktionen sind bei mehreren Fensterbreiten geprüft; einzelne echte Menüaktionen und Zoom-Ausgabe ebenfalls. Anders als C# bleibt auf ausdrücklichen Wunsch auch Speichern in jeder Stufe sichtbar. |
| Strg+N: neue Aufnahme aus dem Editor | fertig | `windows-toolbar-visual-smoke.ps1`; [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41516470659) | Der Test löst Strg+N aus, prüft das eigene Auswahl-Overlay und bricht es anschließend ab. |
| Zuschneiden | fertig | `document.rs::shared_history_orders_shape_and_image_changes`; `windows-editor-smoke.ps1`; Alpha-8-Lauf | Erfolgreicher Crop und Werkzeugwechsel unter Windows geprüft. |
| Rahmen / Schlagschatten mit transparentem PNG | implementiert, nicht belegt | `document.rs::frame_keeps_transparent_source_pixel_and_offsets_shapes` | Bildlogik geprüft; Windows-Ausgabe für beide Effekte fehlt. |
| Rückgängig / Wiederholen | fertig | `editor.rs::creates_with_preview_then_commits_once_and_undo_redoes`; `windows-editor-smoke.ps1`; Alpha-9-Lauf | Modell und konkrete Editorabläufe geprüft; kein pauschaler Nachweis jeder Aktion. |
| Zoom und Schwenken | implementiert, nicht belegt | `view.rs`-Unit-Tests | Direkter Windows-Test für alle Referenzkürzel und Radgesten fehlt. |
| Mehrere Bilder / ungesicherte Änderungen | implementiert, nicht belegt | `windows-multi-editor-smoke.ps1` | Mehrfenster-Lebenszyklus geprüft; alle Speicher-/Abbruchdialogzweige sind nicht nachgewiesen. |

## Ausgabe und Einstellungen

| Funktion der C#-Referenz | Status | Rust-Test / Beleg | Stand / Grenze |
|---|---|---|---|
| PNG, JPEG, BMP, GIF speichern | implementiert, nicht belegt | `image_io.rs::png_and_bmp_round_trip_color`, `gif_and_jpeg_are_readable` | Codec-Tests vorhanden; nicht alle vier Dateidialog-/Speicherpfade auf Windows nachgewiesen. |
| Bild öffnen und Speichern unter | implementiert, nicht belegt | `windows-editor-smoke.ps1`, `image_io.rs`-Tests | Öffnen und konkreter Speicherablauf geprüft; vollständige Dialog-/Fehlermatrix fehlt. |
| Bild in die Zwischenablage kopieren | fertig | `windows-clipboard-smoke.ps1`; Alpha-9-Lauf | Interaktiver Export inklusive Alpha-Daten. |
| Bild aus Zwischenablage einfügen | fertig | `windows-clipboard-smoke.ps1`; Alpha-9-Lauf | Der Test fügt den kopierten Inhalt als weiteren Editor ein und prüft den erneuten Export. |
| Optional automatisches Speichern, Kopieren und Editor nach Aufnahme | implementiert, nicht belegt | `output_windows.rs::autosave_creates_distinct_png_files_in_configured_folder`; `windows-last-region-clipboard-smoke.ps1` | Einzelpfade getestet; alle Kombinationen und Einstellungswechsel über das Tray sind nicht vollständig belegt. |
| Speicherordner öffnen | implementiert, nicht belegt | `src/main.rs::open_storage_folder`; direkter Windows-Smoke-Test fehlt | Menüaktion und Ordneröffnung sind implementiert; kein durchgängiger Nachweis des tatsächlichen Windows-Ablaufs. |
| Standardwerte und bestehende Einstellungen | bewusst anders | `settings.rs`-Default-/Pfadtests; `README.md` | Rust nutzt getrennte Einstellungen und startet ohne automatisches Speichern/Kopieren. Die C#-Einstellungen werden nicht übernommen; Konsequenzen sind für die Migration zu dokumentieren. |
| Autostart | implementiert, nicht belegt | `autostart.rs`-Tests für Kommandozeilen-Quoting | Direkter sicher isolierter Windows-Test für Aktivieren/Deaktivieren fehlt. |
| Infofenster / Tray-Symbol | fertig | `windows-about-smoke.ps1`, `windows-toolbar-visual-smoke.ps1`; [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41516470659) | Infofenster, Fenster-Icon und Registrierung des Tray-Symbols sind im Test belegt. Die Beurteilung der Optik durch den Owner bleibt offen. |

## Zusätzlich freigegebene Rust-Funktionen

| Funktion | Status | Rust-Test / Beleg | Stand / Grenze |
|---|---|---|---|
| Tastaturkorrektur der Bereichsauswahl | fertig | `windows-keyboard-selection-smoke.ps1`; [Alpha-6-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41459734057) | Mausauswahl mit Umschalt einfrieren; Pfeile, Größenänderung, Enter, Esc und anschließender Klick im Test. |
| Letzten Bereich derselben Sitzung wiederholen | fertig | `windows-last-region-smoke.ps1`, `windows-last-region-clipboard-smoke.ps1`; Alpha-6-Lauf | Datei-/Editor- und Clipboard-Weg getestet; echte Monitoränderung bleibt Owner-Praxistest. |
| Browserbereich unter Maus vorschlagen | fertig | `windows-browser-hover-smoke.ps1`; [Edge-/Chrome-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41461190364) | Kontrollierte lokale Testseite; Accessibility-/Browsergrenzen gelten. |

OCR, Bildausschnitt-Lupe im Editor und Bild anheften gehören ausdrücklich nicht zum Beta-Ziel. Die Auswahl-Lupe im Aufnahmeoverlay ist eine vorhandene Referenzfunktion und davon zu unterscheiden.

**Bestand:** 22 fertig, 21 implementiert, nicht belegt, 2 fehlt, 2 bewusst anders (47 Zeilen). Die Zählung beschreibt nur die Zeilen dieser Tabelle, keine vollständige Abnahme.

## Offene Owner-Meldung

Der Owner meldete für Alpha 9 eine nicht erwartungsgemäße Text-Symbolleiste, ein unpassendes Tray-Symbol und auf seinem Screenshot unleserliche Nummernmarkierungen. Nach Beenden einer noch laufenden alten EXE zeigte sein neuer Screenshot mit Alpha 15 eine Symbolleiste und lesbare Nummern 1 bis 8; der konkrete Textleisten-Vorfall ist damit geklärt. Eine gesonderte Beurteilung des Tray-Symbols, der allgemeinen Bedienung und weiterer Windows-Konfigurationen steht aus. Siehe [Windows-Praxistest](WINDOWS_PRAXISTEST.md).

## Owner-Praxistests

Ergebnisse ausschließlich nach tatsächlicher Owner-Rückmeldung in `WINDOWS_PRAXISTEST.md` erfassen. Ein CI-Runner oder diese Tabelle ersetzt keine Prüfung mit gemischter DPI, mehreren Monitoren oder weiteren Browsern. Es liegen hierfür noch keine vom Owner bestätigten Ergebnisse vor.
