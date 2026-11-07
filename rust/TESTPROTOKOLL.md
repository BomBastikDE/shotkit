# ShotKit Rust: Testprotokoll

Dieses Protokoll hält fest, **was wann womit geprüft wurde**, damit Ergebnisse belegbar sind und nichts doppelt
gemacht wird. Jeder Eintrag nennt Version, Commit und Nachweis (CI-Lauf, Issue oder PR).

Verwandte Dokumente: [PARITY.md](PARITY.md) (Funktionsvergleich mit C# 1.2.0),
[WINDOWS_PRAXISTEST.md](WINDOWS_PRAXISTEST.md) (Checkliste und gemeldete Fehler aus Praxistests),
[CHANGELOG.md](CHANGELOG.md).

**Regel für neue Einträge:** neueste Version oben. Ein Nachweis gilt nur für die genannte Version und
Konfiguration. Automatische Tests ersetzen keinen Praxistest auf einem echten Rechner.

---

## Stand für v2.0.0 (noch kein Release) — Commit `2413c37` — 2026-11-07

### Automatische Tests (CI)

Nachweis: [Lauf „Rust checks“ auf `rust/development`](https://github.com/BomBastikDE/shotkit/actions/runs/42353579555): Linux und Windows bestanden.

- Formatierung, Clippy ohne Warnungen (alle Targets), `cargo audit`, Unit- und Integrationstests: bestanden.
- **Interaktive Windows-Szenarien: 63 von 63 bestanden.** Neu gegenüber Beta 1 sind unter anderem Schwärzen-Export, Callouts, Formstile, Leinwandgröße, Einstellungs-Wiederherstellung, PNG-Export mit Speichermessung und der Abbruchtest beim Speichern (`atomic_save_abort_probe`, #67).
- Der Selbsttest der Suite („Budget-Vertrag“) meldet planmäßig „3 scenarios, 3 failures“. Er prüft, dass die Suite bei überschrittenem Zeitbudget sauber abbricht, und ist kein Produktfehler.

### Offen bis v2.0.0

Siehe Plan [#72](https://github.com/BomBastikDE/shotkit/issues/72). Ein Praxistest auf Owner-Hardware ist laut Owner-Entscheidung vom 07.11. **keine** Voraussetzung für v2.0.0. Was die CI nicht abdeckt (mehrere Monitore, gemischte Skalierung, sichtbare Vorschau), steht als bekannte Grenze in der Release-Notiz.

---

## v2.0.0-beta.1 — Commit `a130e6a` — 2026-11-05

### Automatische Tests (CI) und Release

Nachweis: [Tag-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/42103815555): Linux und Windows bestanden, Veröffentlichung erfolgreich.

- **Interaktive Windows-Szenarien: 43 von 43 bestanden.**
- Release-Prüfung am 05.11.: Vorabversion, nicht als „Latest“ markiert. `shotkit-v2.0.0-beta.1-windows-x64.exe` (PE32+, x86-64) hat die SHA256-Prüfsumme `6939ff76e5b4dcab642f295a723ea00cce0fdc205c4912af2b773345c6807c4c`, übereinstimmend mit der `.sha256`-Datei. Die Release-Notiz wurde um „Bekannte Fehler“ ergänzt.
- **Befund:** Der Tag wurde von `rust/development` statt von `main` erstellt (`main` lag 136 Commits zurück). Zusammenführung per [PR #50](https://github.com/BomBastikDE/shotkit/pull/50); künftig verhindert durch [#71](https://github.com/BomBastikDE/shotkit/issues/71) (Tag muss auf `main` liegen).

### Praxistest auf dem Rechner des Owners

Nicht durchgeführt.

---

## v2.0.0-alpha.20 — Commit `8979e24` — 2026-11-04

### Automatische Tests (CI)

Nachweis: [Lauf „Rust checks“ auf `main`](https://github.com/BomBastikDE/shotkit/actions/runs/41903517080) –
Linux (`ubuntu-24.04`) und Windows (`windows-2022`, interaktiver Desktop) bestanden. Das Release wurde im
[Tag-Lauf](https://github.com/BomBastikDE/shotkit/actions/runs/41902749425) aus demselben Commit gebaut.

- Formatierung (`cargo fmt`), Clippy ohne Warnungen, `cargo audit`, Unit- und Integrationstests: bestanden.
- **Interaktive Windows-Szenarien: 23 von 23 bestanden.**

| Bereich | Szenario | Ergebnis |
|---|---|---|
| Programm | Info-Fenster (Version, Entwickler, Website-Schaltfläche) | ✅ |
| Programm | Einzelinstanz: Zweitstart aktiviert erste Instanz, Dateianfrage mit Ablehnung | ✅ |
| Programm | Mehrere Editorfenster: getrenntes Speichern/Rückgängig, Schließen und Beenden mit Abbruch | ✅ |
| Aufnahme | Bereichsaufnahme öffnet den Editor | ✅ |
| Aufnahme | Auswahl per Tastatur: Esc, Pfeiltasten (verschieben), Umschalt+Pfeil (Größe), Enter – pixelgenau | ✅ |
| Aufnahme | „Letzten Bereich erneut aufnehmen“ als Datei und in die Zwischenablage | ✅ ✅ |
| Aufnahme | Element-Vorschlag unter der Maus (Testfenster) | ✅ |
| Aufnahme | Element-Vorschlag auf Testseite in Edge und Chrome, Edge zusätzlich mit Tastatur-Korrektur | ✅ ✅ ✅ |
| Scroll | senkrecht bis Seitenende, 2050 Zeilen pixelgenau | ✅ |
| Scroll | Abbruch mit Esc behält das Teilbild pixelgenau | ✅ |
| Scroll | nach rechts und links, je 2600 Spalten | ✅ |
| Scroll | Edge-Testseite, auch mit fester Kopf- und Fußleiste | ✅ ✅ |
| Editor | Werkzeugleiste: 24 Symbole, Tooltips, Farbwahl, Werkzeugwahl per Maus und Taste | ✅ |
| Editor | Nummern 1–12 auf hellem und dunklem Grund, Rückgängig/Wiederholen | ✅ |
| Editor | Umschalt-Formen, Zuschneiden | ✅ |
| Editor | Verpixeln mit Rückgängig/Wiederholen | ✅ |
| Editor | Griffpunkte einer ausgewählten Form | ✅ |
| Editor | Text anlegen, erneut öffnen, ändern, Rückgängig | ✅ |
| Zwischenablage | Kopieren/Einfügen mit Transparenz, pixelgenau (CF_DIBV5) | ✅ |

**Beobachtung – unzuverlässige Tests:** Im [Lauf zu PR #13](https://github.com/BomBastikDE/shotkit/actions/runs/41903726721)
(nur `SECURITY.md` geändert, Code identisch zu `8979e24`) schlugen im ersten Versuch zwei Szenarien fehl:
„keyboard selection“ (Testprozess mit Exitcode 1) und „Edge browser hover“
(`UIA_PRECHECK_TIMEOUT: Edge UI Automation did not return within 15 seconds`). Im zweiten Versuch desselben
Laufs bestanden alle 23. Der erfolgreiche Wiederholungsversuch zeigt, dass der Fehler nicht in jedem Lauf auftritt. Seine Ursache ist
nicht geklärt; ein zeitabhängiges Test- oder Umgebungsproblem ist eine Möglichkeit, ein Programmfehler ist damit
nicht ausgeschlossen.

### Praxistest auf dem Rechner des Owners

**Noch nicht durchgeführt.** Offen sind insbesondere die Bestätigungen zu
[#1](https://github.com/BomBastikDE/shotkit/issues/1) (Symbole nach Rückgängig – die Korrektur `2b6d4d9` ist in
Alpha 20 enthalten), [#2](https://github.com/BomBastikDE/shotkit/issues/2) (schnelles Freihandzeichnen),
[#3](https://github.com/BomBastikDE/shotkit/issues/3) (Schärfe der Symbole) und
[#4](https://github.com/BomBastikDE/shotkit/issues/4) (pixelige Vorschau) sowie die Checkliste in
[WINDOWS_PRAXISTEST.md](WINDOWS_PRAXISTEST.md). Mehrere Monitore, gemischte Skalierung und echte Webseiten
deckt die CI nicht ab.

---

## Reviews

### Sicherheitsbewertung v2.0.0 – 2026-11-07, Stand `2413c37`

Risikobasierte Bewertung als Abschluss von [#52](https://github.com/BomBastikDE/shotkit/issues/52): Bedrohungsmodell und 18 Angriffsflächen, in [`SECURITY_REVIEW.md`](SECURITY_REVIEW.md) ([PR #73](https://github.com/BomBastikDE/shotkit/pull/73)). Einziges hohes Risiko ist die Verpixelung (#5), bereits im Plan. Nebenbefund: #11 ist umgesetzt und kann geschlossen werden.

### Prüfung des Entwicklungsstands – 2026-11-07, Stand `f7a1008`/`2413c37`

133 Commits seit Beta 1, gezielt geprüft auf Sicherheit (Schwärzen, Verpixeln, Weichzeichnen), CI und Arbeitsweise. Kein Build.

- Schwärzen überschreibt deckend `[0,0,0,255]`, und zwar nach allen anderen Formen. Verpixeln und Weichzeichnen sind als „keine sichere Schwärzung“ gekennzeichnet. Die Verpixelung selbst ist unverändert ([#5](https://github.com/BomBastikDE/shotkit/issues/5)).
- CI: Actions per SHA, `fmt`, Clippy `-D warnings`, `cargo audit`, Toolchain festgelegt. #10 ist im Code erledigt.
- Neue Issues: [#68](https://github.com/BomBastikDE/shotkit/issues/68) Arbeitsweise und Release von `main`, [#69](https://github.com/BomBastikDE/shotkit/issues/69) `unsafe`-Fläche (112 von 155 `unsafe fn` ohne Nachweis, Audit veraltet), [#71](https://github.com/BomBastikDE/shotkit/issues/71) Release-Pipeline für finale Versionen. Entwicklungsrichtlinien: [PR #70](https://github.com/BomBastikDE/shotkit/pull/70).

### Verpixelung: Rückrechnungs-Simulation – 2026-11-05, Stand `a130e6a`

Kandidatenvergleich nach dem Depix-Prinzip (400 sechsstellige Zahlen, Schrift bekannt): Die bisherige Verpixelung lässt sich zu **100 %** zurückrechnen, auch mit 24-px-Blöcken. Mit Farbstufen, Rauschen und Zufallsraster sinkt die Trefferquote auf Zufallsniveau. Skript und Härtungsvorschlag in [#5](https://github.com/BomBastikDE/shotkit/issues/5).

### Vollständiges Code-Review – 2026-11-04, Stand `8288105` (v2.0.0-alpha.20)

Gesamter Rust-Code in vier Bereichen (Win32/App, Editor/Rendering, Aufnahme/Scroll/Overlay, Ein- und Ausgabe/Einstellungen/CI) durchgesehen; jeder gemeldete Befund wurde an der Codestelle nachgeprüft. Kein Build, kein Windows-Test.

- Bestätigt und als Issue angelegt: [#18](https://github.com/BomBastikDE/shotkit/issues/18) Zuschneiden verliert letzte Spalte/Zeile,
  [#19](https://github.com/BomBastikDE/shotkit/issues/19) falsche Auswahl nach Rückgängig,
  [#20](https://github.com/BomBastikDE/shotkit/issues/20) Strg+S überschreibt Originale verlustbehaftet,
  [#21](https://github.com/BomBastikDE/shotkit/issues/21) unbegrenzte Ersatz-Scrollweite,
  [#22](https://github.com/BomBastikDE/shotkit/issues/22) Doppelklick wechselt Werkzeug,
  [#23](https://github.com/BomBastikDE/shotkit/issues/23) Vorschau beim Ziehen,
  [#24](https://github.com/BomBastikDE/shotkit/issues/24) Tag-/Cargo-Version im Release,
  [#25](https://github.com/BomBastikDE/shotkit/issues/25) Ursachen der unzuverlässigen Windows-Tests,
  [#26](https://github.com/BomBastikDE/shotkit/issues/26) Einstellungsformat.
- Gemeldet, aber nicht einzeln nachgeprüft: Sammel-Issue [#27](https://github.com/BomBastikDE/shotkit/issues/27).
- Geprüft, kein Fehler: Alt+F4 im Editor, offene Editorfenster in Bildschirmaufnahmen (erwartet), Lücken bei Schritt-Nummern (wie C# 1.2.0), GDI-/Clipboard-Handles, Überlaufprüfungen bei Bildgrößen, Virtual-Screen-Koordinaten, UIA-Worker, Stitcher-Nähte, Autostart, Kollisionsschutz beim Autosave-Dateinamen.

### Code-Review – 2026-11-04, Stand `151ebb0` (vor Alpha 20)

Durchsicht ausgewählter Stellen in `rust/src` (Undo-Historie, Verpixeln, Einfügen, Scroll-Aufnahme,
Einzelinstanz, Einstellungen, Eingabe-Erzeugung) gegen Befunde aus der C#-Version 1.2.0. Kein Build, kein Test.

- Bereits in Rust gelöst (kein Handlungsbedarf): begrenzte Undo-Historie mit Speicherschätzung, `Strg+V` öffnet
  neuen Editor, Dateiweitergabe an laufende Instanz (`WM_COPYDATA`), atomares Speichern der Einstellungen,
  automatische Builds und Releases.
- Neue Issues: [#5](https://github.com/BomBastikDE/shotkit/issues/5) Verpixeln bei kleinem Text zu schwach,
  [#6](https://github.com/BomBastikDE/shotkit/issues/6) exakter Mausvergleich bei der Scroll-Aufnahme,
  [#7](https://github.com/BomBastikDE/shotkit/issues/7) `mouse_event` → `SendInput`.

### Security-Check – 2026-11-04, Stand `151ebb0`

Geprüft: Bilddateien öffnen (Größen- und Speichergrenzen), Zwischenablage lesen (DIB/Bitmap-Parsing),
Einzelinstanz-Nachrichten, Autostart-Eintrag, Diagnose-Log, `unsafe`-Verträge (Stichproben), CI- und
Release-Workflows, Repo-Einstellungen. Ergebnis: keine kritische Lücke im Programmcode gefunden.

- Neue Issues: [#9](https://github.com/BomBastikDE/shotkit/issues/9) Signierung/Herkunftsnachweis,
  [#10](https://github.com/BomBastikDE/shotkit/issues/10) Actions auf Commit-SHA festnageln,
  [#11](https://github.com/BomBastikDE/shotkit/issues/11) Zwischenablageverlauf,
  [#12](https://github.com/BomBastikDE/shotkit/issues/12) Hinweis bei Netzwerkpfaden.
- Repo-Einstellungen umgesetzt: Dependabot-Warnungen und -Sicherheitsupdates, private Sicherheitsmeldungen,
  Schutzregeln für `main`, `csharp/1.x` und Release-Tags `v*`. Sicherheitsrichtlinie:
  [PR #13](https://github.com/BomBastikDE/shotkit/pull/13).
