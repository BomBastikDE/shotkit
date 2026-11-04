# ShotKit Rust: Testprotokoll

Dieses Protokoll hält fest, **was wann womit geprüft wurde**, damit Ergebnisse belegbar sind und nichts doppelt
gemacht wird. Jeder Eintrag nennt Version, Commit und Nachweis (CI-Lauf, Issue oder PR).

Verwandte Dokumente: [PARITY.md](PARITY.md) (Funktionsvergleich mit C# 1.2.0),
[WINDOWS_PRAXISTEST.md](WINDOWS_PRAXISTEST.md) (Checkliste und gemeldete Fehler aus Praxistests),
[CHANGELOG.md](CHANGELOG.md).

**Regel für neue Einträge:** neueste Version oben. Ein Nachweis gilt nur für die genannte Version und
Konfiguration. Automatische Tests ersetzen keinen Praxistest auf einem echten Rechner.

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
