# Reentranzprüfung der Fensterzustände

Prüfbereich: Rust-Quellcode des Entwicklungszweigs. Dieser statische Befund betrifft die Frage, ob eine Rust-Referenz auf den Overlay-Zustand beziehungsweise `WindowState` oder `EditorController` über einen möglicherweise synchron verschachtelten Windows-Fensteraufruf hinweg benutzt wird. Er ist kein allgemeiner Beweis für die Sicherheit aller FFI-Aufrufe. Die jeweils geprüften Aufrufe sind `MoveWindow`, `SetWindowPos`, `SendMessage*`, `DestroyWindow`, `ShowWindow`, `UpdateWindow` und `DefWindowProc*`; zusätzlich werden explizite Referenzen `&*state` und `&mut *state` betrachtet.

## Overlay

| Stelle | Befund für die geprüfte Reentranzfrage |
|---|---|
| `overlay::select`: Initialisierung und Picker-Übernahme | Zustandszugriffe enden vor der nächsten Fenstererstellung beziehungsweise vor dem Freigeben des Pickers. |
| `overlay::magnifier::setup_magnifier`: `magnifier_rect` vor `CreateWindowExW` | Die temporäre Referenz dient nur der Koordinatenberechnung und endet vor dem Fensteraufruf. |
| `overlay::select`: `ShowWindow`, `UpdateWindow`, `DestroyWindow` | Die Aufrufe erhalten kopierte Fenster-Handles; es bleibt keine Zustandsreferenz über die Aufrufe aktiv. |
| `overlay::hover::update_hover` | Zustandsänderung und Worker-Anforderung ohne geprüften synchronen Fensteraufruf innerhalb des Borrows. |
| `overlay::hover::poll_window` | Einzelne Borrows lesen Worker-Ergebnisse oder ändern Zustand; sie enden vor `choose_hover` beziehungsweise `mark_finished`. |
| `overlay::hover::choose_hover` | Änderungen und Rechteckberechnung ohne geprüften synchronen Fensteraufruf im Borrow. |
| `overlay::hover::poll_uia` | Worker-Ergebnis wird übernommen; der Borrow endet vor `choose_hover`. |
| `overlay::hover::mark_finished` | Aktualisiert Felder und gibt ein kopiertes Handle zurück; kein geprüfter synchroner Fensteraufruf. |
| `overlay::magnifier::release_desktop` | Zustand wird bei GDI-Ressourcenoperationen referenziert; keiner der oben genannten synchronen Fensteraufrufe kommt in diesem Abschnitt vor. |
| `overlay::magnifier::magnifier_rect`, `magnifier_source_rect`, `draw_magnifier` | Geometrie, Monitorabfrage beziehungsweise GDI-Zeichnung; kein oben genannter Fensteraufruf während der Referenznutzung. |
| `overlay::magnifier::move_magnifier` | Handle und Koordinaten werden kopiert; die Referenz endet vor `MoveWindow`. |
| `overlay::magnifier::magnifier_proc` | `draw_magnifier` nutzt die Referenz zwischen `BeginPaint` und `EndPaint`; `DefWindowProcW` liegt in einem anderen Zweig ohne Zustandsreferenz. |
| `overlay::input::handle_mouse`: Maustaste und Mausbewegung | Die Zustandsblöcke enden vor `SetCapture`, `ReleaseCapture` und dem indirekten `MoveWindow`. |
| `overlay::input::handle_mouse` / `handle_keyboard`: Abschluss, Escape und Enter | Rechtecke und Handles werden kopiert; `DestroyWindow` folgt nach den Zustandsreferenzen. |
| `overlay::input::handle_keyboard`: Timer | `poll_window` kehrt vor dem möglichen `DestroyWindow` zurück; verwendet wird ein kopiertes Handle. |
| `overlay::painting::handle_paint`: Zeichnen | Werte werden vor dem Zeichnen übernommen; im geprüften Ablauf wird keine Zustandsreferenz über einen der genannten Fensteraufrufe gehalten. |
| `overlay::painting::handle_paint` und `overlay::window::window_proc`: Zerstörung und Standardbehandlung | Magnifier-Handle wird vor `DestroyWindow` entnommen; bei `DefWindowProcW` ist keine Zustandsreferenz aktiv. |

`SetWindowPos` und `SendMessage*` werden in den Overlay-Dateien nicht aufgerufen. Der Screenshot-/Worker- und der sonstige GDI-Vertrag sind durch diese Teilprüfung nicht vollständig abgedeckt.

## Anwendungsfenster

| Stelle | Befund für die geprüfte Reentranzfrage |
|---|---|
| `app::window_state`, `app::editor` | **Offen auf API-Ebene:** `&'static mut` erzwingt weder die Lebensdauer des Fensters noch exklusive Zugriffe. Alle weiteren Aufrufer und verschachtelten Nachrichten müssen diese Bedingungen einhalten. |
| `app::toolbar_height`, `ModalGuard`, `refresh_title` | Kurzzeitiger Zugriff beziehungsweise besitzender Titel; kein Zustandsborrow über den anschließenden Windows-Aufruf. Die Lebensdauer des Guard-Handles bleibt ein eigener Vertrag. |
| `app::show_editors`, `quit_application` | Fensterliste und Modal-Tiefe werden kopiert; Dialoge und Fensteraufrufe folgen ohne gehaltenen Zustandsborrow. |
| `app::capture` und `app::region`: Aufnahme-, Wiederholungs- und Scrollabläufe | `ShowWindow` verwendet Handles und besitzende Aufnahme-/Einstellungsdaten; keine `WindowState`-/Editorreferenz über dem Aufruf gefunden. `repeat_last_region` kopiert die Auswahl und beendet den `LAST_REGION`-MutexGuard vor dem Fehlerdialog. Die Lebensdauer aller Handles ist damit nicht allgemein bewiesen. |
| `app::document`: Speichern, Wiederherstellen, Öffnen und Einfügen | Feldzugriffe beziehungsweise besitzende Pfadkopien; keine gehaltene Zustandsreferenz über die geprüften Dialog-/Fensteraufrufe gefunden. Die Lebensdauer des Dialogbesitzers bleibt zu prüfen. |
| `app::document::create_editor_window` | Toolbar wird erst nach Fenstererstellung im Zustand ersetzt; die vorherige Toolbar wird nach Ende des Borrows freigegeben. |
| `app::document::display_image`, `initialize_image` | Fenster-/Bildzustand wird ohne gehaltenen Borrow vor `ShowWindow`, `DestroyWindow` und `SetWindowTextW` aktualisiert. |
| `app::document::apply_crop`, `history` | Editoroperationen liefern eigene Bildreferenzen (`Arc`); keine Editorreferenz über Bitmap-/Fensteraufrufe gefunden. |
| `app::rendering::composited_bitmap`, `bitmap_image` | Bild, Formen und Auswahl werden vor Renderer-/GDI-Aufrufen als eigene Werte übernommen. |
| `app::window::proc`, `dispatch_window` sowie die Handler in `window_commands` und `window_mouse` | Userdata wird vor der Freigabe entfernt; Toolbar wird vor `SetWindowPos` geklont und vor der Zerstörung entnommen. Zeichen-, Eingabe- und Dialogpfade wurden auf die genannten Borrow-Grenzen geprüft. |
| `app::actions` | Bildmaße, Farben, Styles und Fehlermeldungen werden vor Clipboard-, Dialog- oder Fensteraufrufen kopiert beziehungsweise besitzend übernommen. |
| `app::config::cancel_gesture` | Der Borrow des Editorfeldes endet vor `ReleaseCapture`. |
| `app::tray`, `hotkeys` | Einstellungen und Handles werden kopiert; kein Zustandsborrow über Menü-/Dialogaufrufe gefunden. |
| `app::instance` | Kein `WindowState`-/Editorborrow über `SendMessageTimeoutW` oder Dialoge; Sendepuffer lebt während des synchronen Sendens, empfangener Pfad wird vor dem Dialog kopiert. |
| `text_dialog::dialog_proc`, `complete` | Die zuvor über `SetWindowTextW` beziehungsweise Textabfrage/Zerstörung gehaltene veränderliche Dialogreferenz wurde auf kurze Blöcke begrenzt. `read_text` und `DestroyWindow` laufen ohne solchen Borrow. |

## Weitere explizite Zeigerreferenzen

| Stelle | Befund für die geprüfte Reentranzfrage |
|---|---|
| `element_picker::find_root` | Der Callback referenziert den während `EnumWindows` lebenden lokalen Suchzustand; die aufgerufenen Fenster-/DWM-Abfragen enthalten keinen der oben genannten expliziten Fensteraufrufe. Allgemeine Reentranzfreiheit der Fremd-APIs ist nicht bewiesen. |
| `app::region::collect_monitor` | Der Callback referenziert die lokale Liste während `EnumDisplayMonitors`; er speichert den Pointer nicht. Die synchrone Callback-Grenze und der Lebensdauervertrag sind im Funktionskommentar beschrieben. |
| `app::instance::handle_instance_file` | Die empfangene `WM_COPYDATA`-Struktur und der Datenpuffer werden vor dem Dialog zu einem eigenen Pfad kopiert. Die Nutzdaten-Ausrichtung wird geprüft; Lesbarkeit setzt den korrekten Nachrichtenvertrag voraus. |
| `toolbar::toolbar_proc`: `CreateStruct` | Die aus `WM_NCCREATE` gelesenen Werte werden vor der weiteren Nachrichtenverarbeitung übernommen. |
| `toolbar::toolbar_proc`: `DrawItem` | Die während `WM_DRAWITEM` geliehene Struktur wird beim GDI-Zeichnen verwendet; im Abschnitt findet keiner der oben genannten Fensteraufrufe statt. GDI-Handle- und Callbackverträge bleiben gesondert relevant. |
| `file_dialog::save_hook` | Referenzen auf `OFNOTIFY` und `OPENFILENAME` werden nur zur Aktualisierung der Dateiendung verwendet; im betreffenden Abschnitt erfolgt keiner der genannten Fensteraufrufe. Die Gültigkeit des von Windows gelieferten Payloads ist Voraussetzung. |
| `folder_dialog` | Die COM-VTable wird für Interfaceaufrufe referenziert; es handelt sich nicht um `STATE` oder `WindowState`. COM-Aufrufe können fremden Code aufrufen und sind nicht pauschal reentranzfrei. |
| `text_dialog::dialog_proc`, `complete` | Der Dialogzustand wird vor synchronen Text-/Fensteraufrufen nicht mehr als veränderliche Referenz gehalten; der Pointer muss während der modalen Schleife gültig bleiben. |

Offene Verträge: `window_state` und `editor` liefern derzeit `&'static mut`; das Typsystem erzwingt die erforderlichen exklusiven, kurzen Zugriffe nicht. Auch die Lebensdauer von Fenster-Handles bei verschachtelten Dialogen sowie das Verhalten bei Panik über `extern "system"`-Callbacks müssen gesondert geprüft werden. Die größere Ablösung des globalen Overlay-Zustands ist für nach der Beta in `ROADMAP.md` notiert.
