# Unsichere Stellen: offene Verträge und Nachweise

Diese Bestandsaufnahme erfasst jede `unsafe fn`-Definition im Rust-Quelltext einschließlich Tests; Funktionszeiger-Typen und einzelne `unsafe`-Blöcke sind nicht mitgezählt. Die Zeilennummern beziehen sich auf den Stand dieser Bestandsaufnahme und können sich durch spätere Formatierung verschieben.

**Kriterium für „belegt“:** Die konkret benannte Voraussetzung ist für die relevanten Aufrufpfade und Reentranzgrenzen anhand zugeordneter Code-Stellen geprüft, und ein benannter Test oder CI-Lauf führt den geprüften Funktionsweg aus. Der Status gilt nur für diesen beschriebenen Umfang und beweist keine Eigenschaften fremder Handles, aller Windows-Umgebungen oder nicht getesteter Pfade.

**Weitere Statuswerte:** `offen` kennzeichnet eine noch nicht entschiedene Voraussetzung; `Beleg fehlt` kennzeichnet einen Vertrag ohne gesonderten vollständigen Nachweis. `Bewusst zurückgestellt` nennt einen konkreten Grund und den ausstehenden Nachweis; daraus folgt keine Behauptung, die betreffende Voraussetzung sei erfüllt.

**Zählung:** 155 Funktionen; 0 offen, 4 für den jeweils ausdrücklich eingegrenzten Vertrag belegt, 39 bewusst zurückgestellt und 112 mit fehlendem gesondertem Beleg.

Die Prüfung der Zustandsreferenzen über ausgewählte synchrone Windows-Aufrufe einschließlich unauffälliger Stellen steht zusätzlich in [REENTRANCY_AUDIT.md](REENTRANCY_AUDIT.md). Die Ablösung des globalen Overlay-Zustands ist in [ROADMAP.md](ROADMAP.md) für nach der Beta notiert.

Ergänzung 2026-11-07 ([#79](https://github.com/BomBastikDE/shotkit/issues/79)): `app::actions::activate_tool`
delegiert die Titelaktualisierung an `app::refresh_title`. Beide benötigen weiterhin ein lebendes eigenes
Editorfenster auf dem UI-Thread. Der Titel wird einschließlich Werkzeugname als besitzender String aufgebaut,
bevor `SetWindowTextW` synchron `WM_SETTEXT` auslösen kann; kein State-Borrow wird danach weiterverwendet.
Der erweiterte Test `tests/windows-save-undo-redo-smoke.ps1` prüft den wirklichen Fenstertitel vor und nach
Werkzeugwechseln im sauberen und geänderten Dokumentzustand. Dieser Windows-Lauf ist lokal auf Linux nicht
ausgeführt; der vollständige Handlelebensdauer-/Reentranznachweis bleibt ausstehend. Die bisherigen
Audit-Statuswerte werden durch die Titelkorrektur nicht angehoben.

| Datei | Funktion | Status | Beleg oder konkreter Grund |
|---|---|---|---|
| `src/about_dialog.rs:92` | `child` | Beleg fehlt | Nachzuweisen: parent und instance sind gültige Handles auf dem UI-Thread. |
| `src/about_dialog.rs:124` | `about_proc` | bewusst zurückgestellt | Die Callback-Registrierung ist sichtbar, aber Serialisierung der globalen Handles und Panikfreiheit über die ABI sind durch den normalen Infofenster-Test nicht vollständig belegt. |
| `src/about_dialog.rs:233` | `register_class` | Beleg fehlt | Nachzuweisen: instance ist ein gültiges Modulhandle und Klassenregistrierung wird serialisiert. |
| `src/alpha_render.rs:116` | `new` | Beleg fehlt | Nachzuweisen: raw ist null oder eine exklusiv übertragene lebende GDI+-Ressource des zu dispose passenden Typs. |
| `src/alpha_render.rs:293` | `render_started` | Beleg fehlt | Nachzuweisen: GDI+ bleibt aktiv und pixels umfasst exklusiv width*height*4 gültige Bytes bis zum Bitmap-Drop. |
| `src/alpha_render.rs:327` | `draw_shape` | Beleg fehlt | Nachzuweisen: g ist ein gültiges exklusiv verwendetes GDI+-Graphics einer aktiven Session. |
| `src/alpha_render.rs:450` | `create_brush` | Beleg fehlt | Nachzuweisen: Die aktive GDI+-Session überlebt den zurückgegebenen Brush und seinen Drop. |
| `src/alpha_render.rs:495` | `create_pen` | Beleg fehlt | Nachzuweisen: Die aktive GDI+-Session überlebt den Pen; width ist für GDI+ gültig. |
| `src/alpha_render.rs:506` | `draw_centered_text` | Beleg fehlt | Nachzuweisen: g ist gültig, die GDI+-Session lebt und die UTF-16-Länge passt in i32. |
| `src/alpha_render.rs:560` | `draw_text` | Beleg fehlt | Nachzuweisen: g ist gültig und die GDI+-Session lebt während der Zeichnung. |
| `src/app/actions.rs:7` | `report_editor_error` | Beleg fehlt | Nachzuweisen: h ist ein lebendes eigenes Editorfenster auf dem UI-Thread. |
| `src/app/actions.rs:24` | `copy_current` | Beleg fehlt | Nachzuweisen: h und die temporäre Bitmap bleiben während der Clipboard-Ausgabe gültig. |
| `src/app/actions.rs:57` | `choose_color` | Beleg fehlt | Nachzuweisen: h bleibt als Dialogbesitzer gültig. |
| `src/app/actions.rs:85` | `choose_number` | Beleg fehlt | Nachzuweisen: h bleibt gültig und values lebt bis zur Rückkehr. |
| `src/app/actions.rs:109` | `choose_frame_action` | Beleg fehlt | Nachzuweisen: h bleibt als Menübesitzer gültig. |
| `src/app/actions.rs:136` | `apply_frame` | Beleg fehlt | Nachzuweisen: h und der Editor erfüllen den Zustandsvertrag auch nach Dialogen. |
| `src/app/actions.rs:207` | `edit_style` | Beleg fehlt | Nachzuweisen: h und der Editor erfüllen den Zustandsvertrag über die Auswahl hinweg. |
| `src/app/actions.rs:258` | `toolbar_action` | bewusst zurückgestellt | Der ModalGuard schützt Eingabewege, doch die Vielzahl delegierter Aktionen und der erneute Zugriff nach Dialogen benötigen einen vollständigen Reentranz- und Fensterlebensdauernachweis. |
| `src/app/capture.rs:7` | `handle_capture` | bewusst zurückgestellt | Bild und Einstellungen sind eigene Werte; für Fensterlebensdauer und Fehlermeldungen während verschachtelter Ausgabepfade fehlt ein gezielter Nachweis. |
| `src/app/capture.rs:108` | `capture_rect` | bewusst zurückgestellt | BusyGuard serialisiert Aufnahmen, ersetzt aber keinen Test für Schließen und Wiederverwendung des Besitzerfensters während Aufnahme oder Fehlerdialog. |
| `src/app/capture.rs:153` | `capture` | Beleg fehlt | Nachzuweisen: h ist ein lebendes eigenes Fenster auf dem UI-Thread. |
| `src/app/capture.rs:160` | `schedule_capture` | Beleg fehlt | Nachzuweisen: h ist ein gültiger Dialogbesitzer auf dem UI-Thread. |
| `src/app/capture.rs:205` | `capture_active` | Beleg fehlt | Nachzuweisen: h ist ein eigenes Fenster; window ist ein Windows-Fensterhandle. |
| `src/app/capture.rs:222` | `active_window_target` | Beleg fehlt | Nachzuweisen: h ist ein eigenes Fenster; window ist nur ein von Windows geliefertes Handle. |
| `src/app/capture.rs:270` | `resolve_active_window_target` | Beleg fehlt | Nachzuweisen: saved stammt aus active_window_target und h ist das eigene Fenster. |
| `src/app/config.rs:14` | `load_settings` | Beleg fehlt | Nachzuweisen: h ist ein gültiger Besitzer für mögliche Fehlerdialoge. |
| `src/app/config.rs:32` | `save_settings` | Beleg fehlt | Nachzuweisen: h ist ein lebendes eigenes Dialog-Besitzerfenster. |
| `src/app/config.rs:61` | `cancel_gesture` | Beleg fehlt | Nachzuweisen: h hat einen initialisierten WindowState auf dem UI-Thread. |
| `src/app/config.rs:77` | `open_storage_folder` | Beleg fehlt | Nachzuweisen: h ist ein gültiger Besitzer; UTF-16-Zeiger leben bis zum API-Rücksprung. |
| `src/app/document.rs:44` | `save` | bewusst zurückgestellt | Pfadkopie und ModalGuard sind vorhanden; ein gezielter Schließen-/Quit-Test während des aktiven Dateidialogs fehlt. |
| `src/app/document.rs:91` | `resolve_recovery` | bewusst zurückgestellt | Die Locks enden vor Dialogen, aber ein Test mit tatsächlich gefülltem Wiederherstellungsspeicher und verschachteltem Schließen fehlt. |
| `src/app/document.rs:148` | `confirm_replace` | Beleg fehlt | Nachzuweisen: h und sein Editorzustand leben während der synchronen Bestätigung. |
| `src/app/document.rs:169` | `paste_image` | Beleg fehlt | Nachzuweisen: h ist ein lebendes eigenes Fenster auf dem UI-Thread. |
| `src/app/document.rs:193` | `open_image` | Beleg fehlt | Nachzuweisen: h ist ein lebender Besitzer für den Dateidialog. |
| `src/app/document.rs:210` | `open_image_path` | Beleg fehlt | Nachzuweisen: h ist ein lebendes eigenes Fenster auf dem UI-Thread. |
| `src/app/document.rs:236` | `create_editor_window` | Beleg fehlt | Nachzuweisen: Die Fensterklasse ist auf dem aktuellen UI-Thread registriert. |
| `src/app/document.rs:271` | `display_image` | Beleg fehlt | Nachzuweisen: Aufruf erfolgt auf dem UI-Thread nach Klassenregistrierung. |
| `src/app/document.rs:307` | `initialize_image` | Beleg fehlt | Nachzuweisen: h ist ein neu erzeugtes eigenes Editorfenster. |
| `src/app/document.rs:351` | `apply_crop` | Beleg fehlt | Nachzuweisen: h besitzt einen lebenden Editorzustand auf dem UI-Thread. |
| `src/app/document.rs:410` | `history` | Beleg fehlt | Nachzuweisen: h besitzt einen lebenden Editorzustand auf dem UI-Thread. |
| `src/app/hotkeys.rs:7` | `register` | Beleg fehlt | Nachzuweisen: h ist ein lebendes Fenster auf seinem Besitzerthread. |
| `src/app/hotkeys.rs:70` | `unregister` | Beleg fehlt | Nachzuweisen: Die IDs wurden für h auf diesem Thread registriert. |
| `src/app/hotkeys.rs:80` | `dispatch` | Beleg fehlt | Nachzuweisen: h erfüllt die Verträge der gewählten Aufnahmeaktion. |
| `src/app/instance.rs:42` | `create_instance_mutex` | Beleg fehlt | Nachzuweisen: Das zurückgegebene Handle wird genau einmal geschlossen. |
| `src/app/instance.rs:68` | `find_primary_window` | belegt | Enger Snapshot-Vertrag: lokaler Klassenpuffer und opakes HWND; `send_to_primary` prüft Null/Sendefehler, `windows-instance-smoke.ps1` bestand im [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41523039080); Handle-Identität nach Rückkehr wird nicht zugesichert. |
| `src/app/instance.rs:86` | `send_to_primary` | Beleg fehlt | Nachzuweisen: Ohne Pfad ist message für Nullparameter gültig; mit Pfad gilt WM_COPYDATA. |
| `src/app/instance.rs:147` | `forward_second_instance` | Beleg fehlt | Nachzuweisen: Die Verträge von send_to_primary gelten. |
| `src/app/instance.rs:164` | `handle_instance_file` | bewusst zurückgestellt | Länge, Payload-Ausrichtung und Terminierung werden geprüft; Lesbarkeit und Ausrichtung der von Windows gelieferten Eingangsstruktur sowie negative Nachrichtenfälle bleiben nachzuweisen. |
| `src/app/mod.rs:361` | `window_state` | bewusst zurückgestellt | Der Typ `&static mut` erzwingt weder Lebensdauer noch Exklusivität; ein vollständiger Nachweis aller Aufrufer und reentranten Zerstörungspfade fehlt. |
| `src/app/mod.rs:369` | `editor` | bewusst zurückgestellt | Der zurückgegebene Editor übernimmt die nicht vom Typ erzwungenen Lebensdauer- und Exklusivitätsvoraussetzungen von `window_state`. |
| `src/app/mod.rs:378` | `toolbar_height` | Beleg fehlt | Nachzuweisen: h besitzt einen lebenden WindowState auf dem UI-Thread. |
| `src/app/mod.rs:391` | `new` | Beleg fehlt | Nachzuweisen: h bleibt bis zum Ende des Guards auf dem UI-Thread am Leben. |
| `src/app/mod.rs:407` | `editor_windows` | Beleg fehlt | Nachzuweisen: EDITOR_WINDOWS wird nur auf dem UI-Thread verändert. |
| `src/app/mod.rs:414` | `refresh_title` | Beleg fehlt | Nachzuweisen: h und sein WindowState sind auf dem UI-Thread gültig. |
| `src/app/mod.rs:444` | `show_editors` | Beleg fehlt | Nachzuweisen: Fensterliste und PRIMARY gehören zum aktiven UI-Thread. |
| `src/app/mod.rs:478` | `quit_application` | Beleg fehlt | Nachzuweisen: h und alle gelisteten Fenster gehören zum UI-Thread. |
| `src/app/mod.rs:519` | `run_application` | bewusst zurückgestellt | Eintrittsthread und einmaliger Aufruf sind im Code sichtbar; Panikfreiheit aller aufgerufenen Windows-Callbacks über die ABI ist damit nicht nachgewiesen. |
| `src/app/region.rs:7` | `capture_region` | bewusst zurückgestellt | Koordinaten und BusyGuard sind eigenständig; ein gezielter Test für Besitzerabbau während des modalen Overlays fehlt. |
| `src/app/region.rs:42` | `capture_region_rect` | bewusst zurückgestellt | Monitorgrenzen und Rechteckwerte werden geprüft; die Besitzerlebensdauer bei verschachtelten Nachrichten und Fehlerdialogen benötigt einen eigenen Nachweis. |
| `src/app/region.rs:116` | `collect_monitor` | Beleg fehlt | Nachzuweisen: data zeigt exklusiv auf eine lebende MonitorList. |
| `src/app/region.rs:142` | `current_monitor_rects` | Beleg fehlt | Nachzuweisen: Der Callback-Pointer ist während EnumDisplayMonitors gültig und exklusiv. |
| `src/app/region.rs:165` | `repeat_last_region` | Beleg fehlt | Nachzuweisen: h ist ein lebendes eigenes Fenster; gespeicherte Koordinaten sind gültig. |
| `src/app/region.rs:194` | `start_scroll` | bewusst zurückgestellt | Der Worker nutzt Werte und einen BusyGuard; ein Test für Fensterabbau oder Handle-Wiederverwendung zwischen Workerstart und `PostMessageW` fehlt. |
| `src/app/rendering.rs:7` | `image_view` | bewusst zurückgestellt | Die Berechnung verwendet kurzlebige kopierte Werte; der vollständige HWND-, Thread- und `window_state`-Vertrag aller Aufrufer ist nicht belegt. |
| `src/app/rendering.rs:26` | `point` | Beleg fehlt | Nachzuweisen: h erfüllt den image_view-Vertrag; l enthält gepackte Clientkoordinaten. |
| `src/app/rendering.rs:50` | `composited_bitmap` | Beleg fehlt | Nachzuweisen: h erfüllt den Fensterzustandsvertrag; die Bitmap wird vom Aufrufer freigegeben. |
| `src/app/rendering.rs:185` | `bitmap_image` | Beleg fehlt | Nachzuweisen: h erfüllt den Fensterzustandsvertrag und enthält gültige Bilddaten. |
| `src/app/rendering.rs:203` | `create_bitmap` | Beleg fehlt | Nachzuweisen: Bildpufferlänge entspricht der 32-Bit-DIB-Größe und der DC ist gültig. |
| `src/app/rendering.rs:243` | `install_bitmap` | bewusst zurückgestellt | Neue Bitmaps stammen aus geprüfter Erstellung; die exklusive alte/native Bitmap-Lebensdauer und alle reentranten Aufrufketten sind nicht vollständig belegt. |
| `src/app/tray.rs:7` | `tray_menu` | bewusst zurückgestellt | Menüwerte sind eigenständig und ModalGuard ist vorhanden; gezielte Tests für gleichzeitiges Schließen und verschachtelte Folgeaktionen fehlen. |
| `src/app/tray.rs:166` | `install` | Beleg fehlt | Nachzuweisen: h ist ein lebendes eigenes Fenster und inst ein gültiges Modulhandle. |
| `src/app/tray.rs:192` | `remove` | Beleg fehlt | Nachzuweisen: h und ID bezeichnen das zuvor registrierte Symbol. |
| `src/app/window.rs:9` | `proc` | bewusst zurückgestellt | Userdata wird vor Freigabe genullt und ModalGuard schützt ausgewählte Pfade; vollständige Zerstörungsreihenfolge und Panikfreiheit aller Callbackzweige sind nicht belegt. |
| `src/app/window.rs:83` | `dispatch_window` | bewusst zurückgestellt | Der einzige Aufrufer prüft Userdata; die nachrichtenspezifischen Pointer- und Lebensdauerverträge aller sechs delegierten Handler bleiben gesondert nachzuweisen. |
| `src/app/window_commands.rs:8` | `dispatch_system` | Beleg fehlt | Nachzuweisen: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig. |
| `src/app/window_commands.rs:102` | `dispatch_shortcuts` | Beleg fehlt | Nachzuweisen: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig. |
| `src/app/window_commands.rs:191` | `dispatch_editor_keys` | Beleg fehlt | Nachzuweisen: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig. |
| `src/app/window_mouse.rs:8` | `dispatch_mouse_begin` | Beleg fehlt | Nachzuweisen: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig. |
| `src/app/window_mouse.rs:98` | `dispatch_mouse_continue` | Beleg fehlt | Nachzuweisen: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig. |
| `src/app/window_mouse.rs:151` | `dispatch_paint_and_close` | Beleg fehlt | Nachzuweisen: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig. |
| `src/clipboard.rs:106` | `copy_bitmap` | Beleg fehlt | Nachzuweisen: bitmap bleibt gültig und entspricht den Maßen; direkter DIB-Speicher ist lesbar und korrekt orientiert. |
| `src/clipboard.rs:226` | `open` | Beleg fehlt | Nachzuweisen: owner ist null oder ein gültiges Fenster; Clipboard wird auf dem richtigen Thread verwendet. |
| `src/clipboard.rs:237` | `close` | Beleg fehlt | Nachzuweisen: Dieser Guard besitzt eine erfolgreich geöffnete Clipboard-Sitzung auf demselben Thread. |
| `src/clipboard.rs:266` | `read_image` | Beleg fehlt | Nachzuweisen: owner ist null oder ein gültiges Fenster; Clipboard bleibt beim Lesen geöffnet. |
| `src/clipboard.rs:343` | `with_global_bytes` | Beleg fehlt | Nachzuweisen: handle bezeichnet lesbaren Global-Memory-Speicher, der während decode gültig und unverändert bleibt. |
| `src/clipboard.rs:367` | `read_png_format` | Beleg fehlt | Nachzuweisen: Clipboard ist offen und das Format liefert gültigen HGLOBAL-Speicher. |
| `src/clipboard.rs:378` | `read_dib_format` | Beleg fehlt | Nachzuweisen: Clipboard ist offen und das Format liefert gültigen HGLOBAL-DIB-Speicher. |
| `src/clipboard.rs:391` | `read_bitmap` | bewusst zurückgestellt | Größe und GDI-Leseergebnis werden geprüft; der Auswahlzustand einer fremden CF_BITMAP im Device Context und der Fallbackpfad sind nicht gezielt belegt. |
| `src/element_picker.rs:289` | `enter` | Beleg fehlt | Nachzuweisen: Der Guard wird auf demselben Thread und in passender Reihenfolge beendet. |
| `src/element_picker.rs:330` | `window_rect` | belegt | Enger Speichervertrag: lokales initialisiertes `Rect`, passende Strukturgröße und Fehlerprüfungen; `windows-hover-smoke.ps1` bestand im [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41523039080); dauerhafte Fensteridentität wird nicht zugesichert. |
| `src/element_picker.rs:360` | `is_cloaked` | Beleg fehlt | Nachzuweisen: hwnd ist ein opaker Handlewert und der Ausgabepuffer ist gültig. |
| `src/element_picker.rs:375` | `is_hit_test_transparent` | Beleg fehlt | Nachzuweisen: hwnd und exstyle beziehen sich für die Aussage auf dasselbe Fenster. |
| `src/element_picker.rs:400` | `process_id` | Beleg fehlt | Nachzuweisen: hwnd wird als opaker Handlewert behandelt. |
| `src/element_picker.rs:413` | `find_root` | bewusst zurückgestellt | Der Suchzustand lebt während des synchronen EnumWindows-Aufrufs; ein vollständiger Nachweis für Callback-Paniken und Reentranz der Fremdabfragen fehlt. |
| `src/element_picker.rs:442` | `collect_topmost_child` | Beleg fehlt | Nachzuweisen: search bleibt exklusiv geliehen und HWNDs sind opake Handles. |
| `src/file_dialog.rs:134` | `save_hook` | Beleg fehlt | Nachzuweisen: WM_NOTIFY enthält einen gültigen ausgerichteten OFNOTIFY-Zeiger mit beschreibbarem OFN. |
| `src/file_dialog.rs:249` | `open` | Beleg fehlt | Nachzuweisen: owner ist null oder ein gültiges Fenster auf dem passenden UI-Thread. |
| `src/file_dialog.rs:264` | `save` | Beleg fehlt | Nachzuweisen: owner ist null oder ein gültiges Fenster auf dem passenden UI-Thread; Hook-Vertrag gilt. |
| `src/file_dialog.rs:278` | `open` | Beleg fehlt | Nachzuweisen: owner ist null oder ein gültiges Fenster auf dem passenden UI-Thread. |
| `src/file_dialog.rs:287` | `save` | Beleg fehlt | Nachzuweisen: owner ist null oder ein gültiges Fenster auf dem passenden UI-Thread; Hook-Vertrag gilt. |
| `src/folder_dialog.rs:99` | `from_raw` | Beleg fehlt | Nachzuweisen: raw ist eine übertragene COM-Interface-Referenz im richtigen Apartment. |
| `src/folder_dialog.rs:161` | `vtable` | bewusst zurückgestellt | Die zwei aktuellen COM-Aufrufe haben passende Interface-Provenienz im Code; ein eigener ausgeführter Test des gesamten Ordnerdialogpfads fehlt. |
| `src/output_windows/native_clipboard.rs:26` | `copy_dib_section_to_clipboard` | Beleg fehlt | Nachzuweisen: pixels zeigt auf ausreichend viele initialisierte, unveränderte BGRA-Bytes mit passendem Stride und Orientierung. |
| `src/output_windows/native_clipboard.rs:42` | `copy_bgra_clipboard` | Beleg fehlt | Nachzuweisen: Der Quellpuffer erfüllt den BGRA- und Lebensdauervertrag; owner ist für Clipboard-Veröffentlichung geeignet. |
| `src/overlay/hover.rs:31` | `is_control_down` | Beleg fehlt | Nachzuweisen: GetKeyState wird mit einem gültigen Tastencode aufgerufen. |
| `src/overlay/hover.rs:39` | `is_shift_down` | Beleg fehlt | Nachzuweisen: GetKeyState wird mit einem gültigen Tastencode aufgerufen. |
| `src/overlay/hover.rs:60` | `update_hover` | Beleg fehlt | Nachzuweisen: state ist gültig, initialisiert und für die Dauer exklusiv auf dem UI-Thread. |
| `src/overlay/hover.rs:90` | `poll_window` | Beleg fehlt | Nachzuweisen: state ist gültig und alle Änderungen erfolgen exklusiv auf dem Overlay-Thread. |
| `src/overlay/hover.rs:175` | `choose_hover` | Beleg fehlt | Nachzuweisen: state ist gültig und exklusiv auf dem Overlay-Thread. |
| `src/overlay/hover.rs:205` | `poll_uia` | Beleg fehlt | Nachzuweisen: state ist gültig und exklusiv auf dem Overlay-Thread. |
| `src/overlay/hover.rs:242` | `mark_finished` | Beleg fehlt | Nachzuweisen: state ist gültig und exklusiv auf dem Overlay-Thread. |
| `src/overlay/input.rs:8` | `handle_mouse` | Beleg fehlt | Nachzuweisen: Windows-Callbackparameter und der serialisierte Overlayzustand sind gültig. |
| `src/overlay/input.rs:159` | `handle_keyboard` | Beleg fehlt | Nachzuweisen: Windows-Callbackparameter und der serialisierte Overlayzustand sind gültig. |
| `src/overlay/magnifier.rs:7` | `capture_desktop` | Beleg fehlt | Nachzuweisen: Positive Abmessungen und korrekte GDI-Ressourcenverwaltung sind erforderlich. |
| `src/overlay/magnifier.rs:60` | `release_desktop` | Beleg fehlt | Nachzuweisen: state ist gültig und enthält nur noch nicht freigegebene eigene GDI-Ressourcen. |
| `src/overlay/magnifier.rs:80` | `monitor_local_rect` | Beleg fehlt | Nachzuweisen: Win32-Monitorabfragen erhalten korrekt initialisierte Ausgabestrukturen. |
| `src/overlay/magnifier.rs:105` | `magnifier_rect` | Beleg fehlt | Nachzuweisen: state bleibt während der Berechnung gültig und unverändert. |
| `src/overlay/magnifier.rs:148` | `draw_magnifier` | bewusst zurückgestellt | Snapshot- und Paint-DC-Lebensdauer ist strukturiert; für alle GDI-Auswahlfehler und konkurrierende STATE-Änderungen fehlt ein vollständiger Nachweis. |
| `src/overlay/magnifier.rs:271` | `move_magnifier` | Beleg fehlt | Nachzuweisen: state ist gültig und enthält ein lebendes Magnifier-Handle. |
| `src/overlay/magnifier.rs:296` | `magnifier_proc` | bewusst zurückgestellt | BeginPaint und EndPaint sind gepaart; vollständiger globaler STATE-/Paint-Reentranz- und Callback-Paniknachweis fehlt. |
| `src/overlay/magnifier.rs:317` | `setup_magnifier` | Beleg fehlt | Nachzuweisen: instance und hwnd sind gültig; STATE ist auf dem UI-Thread initialisiert. |
| `src/overlay/painting.rs:8` | `handle_paint` | Beleg fehlt | Nachzuweisen: Windows-Callbackparameter und der serialisierte Overlayzustand sind gültig. |
| `src/overlay/window.rs:9` | `window_proc` | bewusst zurückgestellt | Das Overlay wird vor der Fensteranlage initialisiert; Lebensdauer und Panikfreiheit aller Unterhandler bei verschachtelten Nachrichten sind nicht vollständig belegt. |
| `src/overlay/window.rs:27` | `run_message_loop` | bewusst zurückgestellt | Die Schleife behandelt regulären Dispatch und Abbruch; gezielte Prüfungen für GetMessage-Fehler, WM_QUIT und verschachtelte Zerstörung fehlen. |
| `src/overlay.rs:332` | `select` | Beleg fehlt | Nachzuweisen: Fenstererzeugung und Nachrichtenverarbeitung erfolgen auf demselben Thread; kein rekursiver Aufruf. |
| `src/render/tests.rs:26` | `new` | belegt | Nur für aktuelle Testaufrufe: feste kleine Maße, geprüfte GDI-Handles und `GdiFlush`; `render::tests::gdi_shapes_and_blur_change_dib_pixels_without_a_window_or_display` bestand im [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41523039080). |
| `src/render.rs:194` | `select` | Beleg fehlt | Nachzuweisen: dc und obj sind kompatibel und leben länger als der Auswahlguard. |
| `src/render.rs:209` | `pen` | Beleg fehlt | Nachzuweisen: dc ist gültig; der Selected-Guard wird vor dem zugehörigen OwnedObject verworfen. |
| `src/render.rs:222` | `brush` | Beleg fehlt | Nachzuweisen: dc ist gültig; der Selected-Guard wird vor dem zugehörigen OwnedObject verworfen. |
| `src/render.rs:231` | `line` | Beleg fehlt | Nachzuweisen: dc und der ausgewählte Pen sind gültig; Positionsänderung ist erlaubt. |
| `src/render.rs:257` | `translucent_highlight` | Beleg fehlt | Nachzuweisen: Der Ziel-DC ist gültig; beteiligter Speicher wird nicht gleichzeitig von CPU und GDI geändert. |
| `src/render.rs:345` | `draw_shape` | Beleg fehlt | Nachzuweisen: dc und optionaler source-DC sind gültig und deren Ressourcen bleiben während aller Operationen lebendig. |
| `src/render.rs:464` | `draw_arrow` | Beleg fehlt | Nachzuweisen: dc ist ein gültiger geliehener GDI-DC mit stabiler Lebensdauer. |
| `src/render.rs:502` | `draw_step` | Beleg fehlt | Nachzuweisen: dc ist ein gültiger geliehener GDI-DC mit stabiler Lebensdauer. |
| `src/text_dialog.rs:167` | `read_text` | Beleg fehlt | Nachzuweisen: hwnd ist ein gültiges Edit-Fenster; der Ausgabepuffer fasst die angefragten UTF-16-Zeichen. |
| `src/text_dialog.rs:186` | `complete` | Beleg fehlt | Nachzuweisen: state zeigt auf lebenden DialogState und hwnd gehört zu diesem Dialog. |
| `src/text_dialog.rs:207` | `dialog_proc` | bewusst zurückgestellt | Borrows enden vor Text-/Fensteraufrufen; Klassenidentität, tatsächlicher Fensterabbau und Panikpfade über die Callback-ABI sind nicht vollständig belegt. |
| `src/text_dialog.rs:339` | `ensure_class` | bewusst zurückgestellt | Ein bestehender gleichnamiger Klassenname wird ohne Vergleich von Callback und Modul als Erfolg akzeptiert; ein Klassenkollisionstest fehlt. |
| `src/text_dialog.rs:365` | `prompt` | bewusst zurückgestellt | ModalGuard schützt relevante Ereignisse und der Stackzustand bleibt in der Schleife bestehen; Klassenidentität und HWND-Lebensdauer bei allen Reentranzpfaden sind offen. |
| `src/toolbar/drawing.rs:7` | `icon_line` | Beleg fehlt | Nachzuweisen: hdc und ausgewählter Pen sind gültig; Positionsänderung ist zulässig. |
| `src/toolbar/drawing.rs:16` | `icon_fill_rect` | bewusst zurückgestellt | Die lokale Brush wird zurückgesetzt und gelöscht; Fehler bei GDI-Erzeugung und Wiederherstellung der vorherigen Auswahl werden nicht vollständig geprüft. |
| `src/toolbar/drawing.rs:28` | `icon_fill_ellipse` | bewusst zurückgestellt | Die lokale Brush wird zurückgesetzt und gelöscht; Fehler bei GDI-Erzeugung und Wiederherstellung der vorherigen Auswahl werden nicht vollständig geprüft. |
| `src/toolbar/drawing.rs:40` | `draw_swatch` | bewusst zurückgestellt | Temporäre Pens werden wiederhergestellt; Anlage-, Auswahl- und Koordinatenfehler sind nicht vollständig geprüft. |
| `src/toolbar/drawing.rs:79` | `icon_polygon` | bewusst zurückgestellt | Die aktuellen Punktarrays sind klein und leben über den Aufruf; Konvertierung und GDI-Auswahlfehler sind nicht allgemein abgesichert. |
| `src/toolbar/drawing.rs:91` | `invalidate_tools` | Beleg fehlt | Nachzuweisen: toolbar und zugehörige Kinder sind lebende HWNDs auf dem UI-Thread. |
| `src/toolbar/drawing.rs:104` | `high_contrast_enabled` | Beleg fehlt | Nachzuweisen: SystemParametersInfoW erhält einen gültigen korrekt dimensionierten Ausgabepuffer. |
| `src/toolbar/drawing.rs:124` | `draw_icon` | bewusst zurückgestellt | Der DC stammt aus WM_DRAWITEM; vollständige Fehlerbehandlung für GDI-Anlage, Auswahl und Wiederherstellung der Unterfunktionen fehlt. |
| `src/toolbar.rs:203` | `toolbar_proc` | bewusst zurückgestellt | Die Klasse registriert den Callback; die Gültigkeit aller nachrichtenspezifischen Pointer und Panikfreiheit über die ABI sind nicht vollständig belegt. |
| `src/toolbar.rs:323` | `register_class` | bewusst zurückgestellt | Ein vorhandener gleichnamiger Klassenname wird ohne Prüfung seiner Callback-/Instanzidentität akzeptiert; ein Kollisionstest fehlt. |
| `src/toolbar.rs:358` | `set_active` | bewusst zurückgestellt | Die Map wird beim Erzeugen und Zerstören gepflegt; HWND-Wiederverwendung und Reentranz während des gehaltenen Map-Locks sind nicht vollständig belegt. |
| `src/toolbar.rs:378` | `set_color` | bewusst zurückgestellt | Farb- und Toolbar-Maps werden getrennt verwendet; HWND-Lebensdauer und Reentranz bei Zugriff unter dem Toolbar-Lock sind nicht vollständig belegt. |
| `src/toolbar.rs:406` | `resize` | bewusst zurückgestellt | Ein geklontes Rc hält das Rust-Objekt am Leben; ein Test für Zerstörung nativer Kindfenster während SetWindowPos-Callbacks fehlt. |
| `src/toolbar.rs:454` | `create` | bewusst zurückgestellt | Parent und Tooltiptext-Lebensdauer sind im Normalpfad sichtbar; Klassenkollision und vollständige HWND-Lebensdauer bei verschachtelter Erstellung sind nicht belegt. |
| `src/uia_picker.rs:286` | `create_automation` | Beleg fehlt | Nachzuweisen: COM ist im passenden Apartment initialisiert; die Referenz wird dort verwendet und freigegeben. |
| `src/uia_picker.rs:305` | `inspect` | belegt | Enger COM-Apartment-Vertrag: MTA-Initialisierung, Interface-Nutzung und Freigabe bleiben im Worker; der UIA-Auswahlpfad mit Tastaturkorrektur bestand im [Windows-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41523039080); Providerqualität und Handle-Identität sind nicht damit belegt. |
