# Entwicklungsrichtlinien

Diese Regeln gelten für alle Beiträge zu ShotKit, von Menschen ebenso wie von KI-Agenten. Abweichungen nur nach ausdrücklicher Entscheidung des Owners; die Entscheidung im betroffenen Issue oder PR festhalten.

## Branches

- **`main` ist der einzige Entwicklungszweig.** Jede Änderung entsteht auf einem kurzlebigen Branch (`fix/…`, `feat/…`, `docs/…`, `chore/…`) und kommt per Pull Request nach `main`.
- Keine langlebigen Parallelzweige. Ein Branch, der länger als wenige Tage offen ist, wird in kleinere PRs aufgeteilt.
- `csharp/1.x` erhält nur noch Sicherheitskorrekturen.
- Keine direkten Pushes auf `main`.

## Pull Requests

- **Ein Thema pro PR.** Richtwert: höchstens etwa 500 geänderte Zeilen Produktivcode, Tests nicht mitgezählt. Größere Vorhaben in aufeinander aufbauende PRs zerlegen.
- Beschreibung mit **Warum**, **Was** und **Wie getestet**, dazu ein Verweis auf das Issue (`Fixes #N` oder `Refs #N`).
- Merge erst bei grüner CI und nach Review durch den Owner oder einen zweiten, unabhängigen Prüfer (Mensch oder Agent, der die Änderung nicht geschrieben hat).
- Funktionen, Kürzel, Menüs oder Werkzeuge geändert: Handbuch unter `docs/` im selben PR anpassen.

## Priorisierung

1. Sicherheitslücken und Issues mit `priorität: hoch`
2. übrige Fehler (`bug`)
3. neue Funktionen (`enhancement`, `roadmap`)

Neue Funktionen erst beginnen, wenn keine Issues mit `security` und `priorität: hoch` offen sind, außer der Owner entscheidet es ausdrücklich anders. Vom Owner ausgeschlossene Funktionen nicht umsetzen.

## Issues

- Jedes Issue hat ein **messbares Fertigkriterium**, zum Beispiel einen Test, eine Grenze oder ein Verhalten. „Funktioniert“ oder „nicht mehr lesbar“ genügen nicht.
- Schließen nur mit Nachweis: Commit oder PR, der zugehörige Test und ein grüner CI-Lauf. Ein Teilerfolg wird als Kommentar festgehalten; das Issue bleibt offen.
- Neue Befunde während der Arbeit als eigenes Issue anlegen statt sie nebenbei zu ändern.

## Code

- Vor jedem PR lokal: `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`. Die CI prüft zusätzlich `cargo audit` und die Windows-Szenarien.
- Jede Verhaltensänderung braucht einen Test, der ohne die Änderung fehlschlägt.
- Fehler fallibel behandeln (`try_reserve`, `Result`); kein `unwrap`/`expect` im Produktivcode außer bei nachweislich unmöglichen Fällen mit Kommentar.
- **`unsafe`** nur in kleinen, gekapselten Hilfsfunktionen oder RAII-Typen. Neue oder geänderte `unsafe`-Stellen im selben PR in [`rust/UNSAFE_AUDIT.md`](rust/UNSAFE_AUDIT.md) mit Vertrag und Nachweis eintragen. Das Audit muss zum aktuellen Code passen.
- Module überschaubar halten. Richtwert: höchstens etwa 1000 Zeilen Produktivcode je Datei, Tests in eigene Dateien.
- Neue Abhängigkeiten nur mit Begründung im PR. Versionen über `Cargo.lock`, GitHub Actions über Commit-SHA festlegen.

## Sicherheit und Datenschutz

- ShotKit verarbeitet Bildschirminhalte; diese gelten immer als vertraulich. Keine Telemetrie, keine Netzwerkzugriffe ohne ausdrückliche Nutzeraktion.
- Nur Schwärzen gilt als sichere Unkenntlichmachung. Verpixeln, Weichzeichnen und ähnliche Effekte dürfen nicht als sicher beschrieben werden, solange kein messbarer Nachweis vorliegt (siehe #5).
- Testdaten sind synthetisch, keine echten Nutzerdaten und keine öffentlichen Live-Systeme.
- Sicherheitslücken nach [SECURITY.md](SECURITY.md) melden, nicht in öffentlichen Issues.

## Versionen und Releases

CHANGELOG-Abschnitte heißen `## Version X.Y.Z` beziehungsweise `## Version X.Y.Z-beta.N`; andernfalls bricht die Release-Pipeline ab.

- Releases werden **nur von `main`** getaggt (`v2.X.Y-beta.N`, später `v2.X.Y`).
- Vor dem Tag im selben PR: Version in `rust/Cargo.toml` erhöhen und den Abschnitt in `rust/CHANGELOG.md` mit der Versionsnummer abschließen.
- Zeitnah veröffentlichen: Liegen auf `main` nennenswerte Änderungen für Nutzer, folgt eine neue Vorabversion, statt Dutzende Commits ungetaggt zu sammeln.
- Die Release-Notiz nennt die wichtigsten Änderungen und die **bekannten Fehler** (offene `bug`-Issues mit Priorität hoch oder mittel).
- Als „Latest“ markiert wird eine Version erst nach Freigabe durch den Owner.

## Dokumentation

- **CHANGELOG für Nutzer:** je Eintrag ein bis zwei verständliche Sätze, was sich für Nutzer ändert, mit Issue-Verweis. Technische Details gehören in Commit und PR.
- README, Handbuch (`docs/`) und `rust/README.md` beschreiben den tatsächlichen Stand: Status, Hauptzweig und Download.
- Commit-Nachrichten beschreiben eine logische Änderung, kurz und aussagekräftig.
