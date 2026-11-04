# Multi-Editor: Dokumentzustand und Lebenszyklus

Stand der Implementierung: 2026-11-02. Referenz (Branch `csharp/1.x`): C# `src/Program.cs`
(`ShowEditor`, `Quit`) und `src/EditorForm.cs` (`OnFormClosing`).

Jedes Bild erhält ein eigenes natives Fenster (`ShotKitRustEditor`). Über
`GWLP_USERDATA` besitzt dieses Fenster einen eigenen `WindowState`: Controller mit
Bild und Undo/Redo, GDI-Bitmap, Toolbar, Dateipfad, Änderungsstatus, Zoom/Pan sowie
laufende Zeichen- und Crop-Gesten. Es gibt keinen global ausgetauschten aktiven
Dokumentzustand. Die Win32-Nachrichten werden anhand ihres HWND zugeordnet.

Der erste, versteckte HWND (`ShotKitRustPrototype`) bleibt ausschließlich für
Tray, globale Hotkeys, Einstellungen, Aufnahme-Ergebnisse und Einzelinstanz-IPC
zuständig. Das Schließen des letzten Editors beendet diesen Host nicht.
`Editor anzeigen` und ein erneuter Start ohne Dateipfad aktivieren vorhandene
Editorfenster. Ohne geöffnetes Bild wird ein leerer Editor mit sichtbarem Hinweis
auf Öffnen (`Strg+O`) und Einfügen (`Strg+V`) angelegt. Dieser leere Editor ist
nicht ungespeichert und lässt sich ohne Speicherdialog schließen. Öffnen, Einfügen, Aufnahme und jedes Startargument
legen jeweils ein neues Bildfenster an. Ein bestätigtes Dateiargument aus einer
zweiten Instanz öffnet ebenfalls ein zusätzliches Fenster.

## Speichern und Schließen

- `Strg+S` und der Speichern-Button betreffen nur ihren HWND; `Strg+Umschalt+S`
  öffnet dessen Speichern-unter-Dialog. Erst nach erfolgreichem atomarem Schreiben
  werden Dateipfad und Dirty-Markierung geändert. Fehler und Dialogabbruch lassen
  das Dokument geöffnet und ungespeichert.
- Die Titelleiste nennt den zugehörigen Pfad, bei neuen Bildern `Neue Aufnahme`.
  Ein führendes `*` markiert ungespeicherte Änderungen. Undo/Redo setzt vorsorglich
  Dirty; eine exakte Rückkehr zur gespeicherten Revision wird weiterhin nicht
  gesondert erkannt.
- Schließen eines Editors beendet laufende Gesten und fragt nur bei dessen
  Änderungen nach Speichern/Verwerfen/Abbrechen. Das Verwerfen betrifft dieses
  Fenster. Fehlgeschlagenes oder abgebrochenes Speichern verhindert Schließen.
- Beenden im Tray prüft zunächst alle Editorfenster und zurückgehaltenen
  Aufnahmen. Solange ein Editor einen modalen Vorgang ausführt, wird Beenden
  nicht fortgesetzt. Ein Abbruch lässt sämtliche Fenster bestehen. Erst wenn
  alle Dokumente bestätigt wurden, werden sie zerstört; anschließend beendet
  eine eigene gepostete Nachricht den primären Host. Bereits erfolgreich
  gespeicherte Dokumente bleiben bei einem späteren Abbruch gespeichert.
- Während laufender oder geplanter Aufnahmen wird Beenden zurückgestellt.
  Verzögerte Aufnahmen öffnen wie zuvor die Bereichsauswahl. Trifft deren
  Timer-Nachricht während eines modalen Host-Dialogs oder einer laufenden
  Aufnahme ein, wird sie nach 250 ms erneut zugestellt; die Pending-Sperre
  bleibt bis zum Abschluss der Bereichsauswahl gesetzt. Jede Anfrage besitzt
  ein monoton steigendes Ticket in WPARAM. Worker prüfen es vor dem Posten;
  der Dispatcher prüft erneut und erlaubt den Übergang Waiting → Running nur
  einmal. Verspätete Wakeups, Wiederholungen und alte Fehler-Callbacks können
  weder eine zweite Auswahl noch eine neuere Anfrage starten oder freigeben.
  Der plattformneutrale Kern `src/delayed_capture.rs` sichert diese Fälle mit
  Regressionstests ab. Tickets verwenden die native WPARAM-Breite; bei
  Zählererschöpfung wird ausdrücklich ein Neustart verlangt statt gewrappt. Der
  vorhandene Recovery-Puffer bleibt anwendungsweit und wird vor Beenden geprüft.
  Er wird weiterhin nicht auf Disk persistiert; Prozessabsturz oder erzwungenes
  Beenden kann ungespeicherte Daten verlieren.

Modale Win32-Dialoge pumpen weitere Nachrichten. `ModalGuard` hält den jeweiligen
HWND deshalb logisch belegt; erneute Eingabe/Close und ein konkurrierendes Quit
können ihn während eines solchen Aufrufs nicht entfernen. IPC meldet bei einem
belegten Host den bestehenden Busy-Status. Während der Quit-Bestätigungen werden
neue Eingabe- und Aufnahmevorgänge blockiert. Paint und Resize bleiben möglich.
Die Zustandsallokation wird in `WM_NCCREATE` angelegt und erst bei
`WM_NCDESTROY` freigegeben; GDI und Toolbar werden in `WM_DESTROY` entsorgt.
Die Toolbar wird vorher per `take()` aus dem Fensterzustand entfernt und erst
nach Ende dieses Borrows freigegeben, weil das Zerstören ihrer nativen Kinder
synchron den Parent aufrufen kann. Beim Resize hält eine lokale `Rc`-Kopie die
Toolbar am Leben, ohne einen Borrow des Parent-Zustands über `SetWindowPos`
hinweg zu behalten. `WM_NCDESTROY` entfernt `GWLP_USERDATA` vor dem Box-Drop.

## Prüfungen und Grenzen

`tests/windows-multi-editor-smoke.ps1` erzeugt ausschließlich temporäre Bilder
und ein isoliertes Profil. Geprüft werden zwei gleichzeitige Startdokumente,
getrennter Dirty-/Save-/Undo-Zustand mit Pixelauswertung, ein absichtlich
verursachter Schreibfehler, Close-/Quit-Abbruch, unabhängiges Schließen und das
Weiterlaufen des Tray-Hosts ohne Editor sowie einen sichtbaren leeren Editor
bei anschließender Aktivierung. Der Test verweigert vorhandene Prozesse
und räumt nur den selbst gestarteten Prozess mit passender Identität auf.
Er wurde in die Windows-CI aufgenommen; die bisherigen Capture-/IPC-/Editor-
Smokes unterscheiden nun den primären Host von sichtbaren Editorfenstern.

Linux-Verifikation mit `shotkit-rust-build:local`: 116 Bibliothekstests bestanden;
Windows-GNU-Crosscheck kompiliert. Die interaktiven PowerShell-Smokes benötigen
einen Windows-Desktop und sind im Linux-Arbeitsraum nicht ausführbar. Diese
Notiz behauptet deshalb keinen erfolgreichen Windows-Runtime-Test. Insbesondere
Fokus, modale Dialoge und HWND-Lebenszyklen benötigen den CI-Nachweis.

Rückweg: den Multi-Editor-Commit zurücknehmen. Einstellungen und Dateiformate
werden durch die Architekturänderung nicht migriert.
