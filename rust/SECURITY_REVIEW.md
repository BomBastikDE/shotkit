# Sicherheitsbewertung ShotKit 2.0.0

Risikobasierte Bewertung als Abschluss von #52 für v2.0.0 (Plan #72). Sie ersetzt keine Detailprüfungen; deren Verlauf steht in #52.

**Bewerteter Stand:** `rust/development` @ `2413c37` (2026-11-07). Vor dem Tag `v2.0.0` werden nur die Zeilen neu bewertet, deren Code sich seitdem geändert hat.

## Bedrohungsmodell

ShotKit ist ein lokales Desktop-Programm. Es läuft mit den Rechten des angemeldeten Nutzers, ohne Administratorrechte, ohne Netzwerkdienst und ohne Telemetrie. Netzwerkzugriffe gibt es nur auf ausdrückliche Nutzeraktion: den Link im Infodialog und das Öffnen von Dateien auf Netzlaufwerken nach Warnung.

**Im Fokus:**
1. **Vertraulichkeit der Bildschirminhalte.** Bei einem Screenshot-Werkzeug ist das das wichtigste Schutzgut. Unkenntlich gemachte Inhalte dürfen nicht wiederherstellbar sein; Kopien dürfen nicht unbemerkt an Orten liegen bleiben, die der Nutzer nicht erwartet.
2. **Nicht vertrauenswürdige Bilddateien,** die der Nutzer öffnet (Öffnen-Dialog, Kommandozeile, Weitergabe an die laufende Instanz).
3. **Lieferkette:** Herkunft und Unverändertheit der veröffentlichten EXE.

**Nicht im Fokus:** Prozesse desselben Nutzers in derselben Sitzung. Sie können ohnehin Bildschirm, Zwischenablage, Einstellungen und die EXE lesen oder ändern. Angriffe von dort (Fensternachrichten, Zwischenablageinhalt, Einstellungsdatei) gelten deshalb höchstens als Robustheitsproblem, z. B. Absturz, nicht als Rechteausweitung.

## Bewertung

Risiko = Auswirkung × Wahrscheinlichkeit, bezogen auf das Bedrohungsmodell. „Hoch“ blockiert v2.0.0.

| # | Angriffsfläche | Mögliche Auswirkung | Risiko | Stand | Nachweis / Issue |
|---|---|---|---|---|---|
| 1 | **Verpixeln** | Verpixelter Text kann unter Umständen rekonstruiert werden | mittel | Gehärtet: mindestens 48 px, zufällige Rasterphase, vier Farbstufen und Rauschen; keine sichere Schwärzung. Die Rauschableitung mit SplitMix ist nicht kryptografisch (#85) | #5; `tests/pixelation_attack.rs` (Kandidatenvergleich und Gegenprobe) |
| 2 | Weichzeichnen | Starke Unschärfe ist nicht dasselbe wie Entfernen; bei bekannter Schrift ist eine Rekonstruktion grundsätzlich denkbar | mittel | Werkzeug-Hinweis „keine sichere Schwärzung“; Handbuch verweist auf Schwärzen | nach 2.0.0 gleiche Messung wie #5 |
| 3 | Schwärzen | Ungeschwärzte Pixel im Export oder in Effekten, die die Fläche abtasten | niedrig | Deckendes `[0,0,0,255]` wird **nach** allen anderen Formen angewendet; Verpixeln, Weichzeichnen und Lupe tasten eine bereits geschwärzte Quelle ab; Exporttests | `alpha_document.rs` `apply_redactions`; #52 (Prüfungen vom 06.11.), CI |
| 4 | Automatische Ausgabe vor dem Schwärzen | Automatisches Speichern und Kopieren läuft vor dem Editor; spätere Schwärzungen ändern diese Rohkopien nicht | mittel | Dokumentiert (Anleitung). Standard beim ersten Start: automatisches Speichern und Kopieren **aus** | `rust/README.md`; Empfehlung: Hinweis auch im Handbuch, Abschnitt Sicherheit |
| 5 | Temporäre Dateien nach Absturz | Eigene Temp-Datei mit Bilddaten bleibt im Zielordner liegen | mittel | Hinweis beim Start beschlossen, im Plan; Windows-Abbruchtest vorhanden | #67 |
| 6 | Diagnosedateien der Scrollaufnahme | Bei unklarem Abgleich legt ShotKit zwei Bildausschnitte plus Text unter `<Speicherordner>\Diagnose` ab; sie bleiben bis zur manuellen Löschung | niedrig | Nur mit konfiguriertem Ordner; der Pfad wird in der Meldung genannt; in der Anleitung dokumentiert | `scroll_diagnostics.rs`; Empfehlung nach 2.0.0: Ablage nur nach Rückfrage |
| 7 | Zwischenablage-Verlauf / Cloud-Sync | Kopierte Screenshots landen im Windows-Verlauf oder in der Cloud-Zwischenablage | niedrig | Ausschluss über `CanIncludeInClipboardHistory`/`CanUploadToCloudClipboard` = 0, **standardmäßig an** | `native_clipboard.rs`, `settings.rs` (`exclude_from_clipboard_history`); **#11 kann geschlossen werden** |
| 8 | Vorschau nach Zeichenfehler | Alte, ungeschwärzte Pixel bleiben auf dem Bildschirm sichtbar | niedrig | Bereich wird deckend neutralisiert; betrifft nur die Anzeige, nicht den Export | #66, CI |
| 9 | Bildimport (PNG, JPEG, GIF, BMP, TIFF) | Speicherfehler im Decoder → Absturz oder Codeausführung; übergroße Bilder → Speichererschöpfung | niedrig | Decoder in Rust (`image` 0.25.10, `png` 0.18.1, `tiff`, `zune-jpeg`, `gif`), `cargo audit` in der CI, Größen-, Datei- und Budgetgrenzen; Header- und Metadaten-Allokationen der Decoder sind nicht vollständig begrenzt (nur Speichererschöpfung) | #51, #65, #52 (07.11.) |
| 10 | Netzlaufwerke / UNC-Pfade | Beim Öffnen authentifiziert sich Windows am Server (NTLM-Hash geht an einen fremden Server) | niedrig | Warnung vor dem Öffnen von UNC-Pfaden, zugeordneten und unbekannten Laufwerken, auch bei Weitergabe aus einer zweiten Instanz | #12, #55 |
| 11 | Zweite Instanz (`WM_COPYDATA`, Mutex) | Fremde Nachrichten öffnen Dateien oder beenden ShotKit | niedrig | Nur derselbe Nutzer bzw. dieselbe Integritätsstufe (UIPI); Größen- und UTF-16-Prüfung, Kopie vor dem Dialog, Bestätigung nötig; Beenden und Scrollende über interne Zustandsprüfung | #49, #52 (IPC-Prüfung vom 06.11.) |
| 12 | Einstellungsdatei | Manipulierte oder beschädigte `settings.json` | niedrig | Begrenztes Lesen, Versionsprüfung, gesicherte Wiederherstellung und Rücksetzen nur nach Zustimmung; Angreifer müsste derselbe Nutzer sein | #26, #56 |
| 13 | Speichern (atomar, Originalschutz) | Überschreiben fremder Dateien, Datenverlust bei Fehlern | niedrig | Geschützte Temp-Datei, Veröffentlichung erst nach Sync, vorhandene Dateien werden nicht ersetzt, gehaltene Handles | #61, #62, #63 |
| 14 | Autostart | Manipulierter Startbefehl | niedrig | `HKCU\…\Run` mit korrekt gequotetem EXE-Pfad; ändern kann ihn nur derselbe Nutzer | `autostart.rs` |
| 15 | `unsafe` / Windows-FFI | Speicherfehler an der Grenze zu Win32/GDI+/Zwischenablage | mittel | 155 `unsafe fn`, davon 112 ohne gesonderten Nachweis; Audit vom 02.11. ist veraltet. Von außen erreichbar nur über Zwischenablage-DIB (derselbe Nutzer) und Bildgrößen (geprüft) | #69 (nach 2.0.0) |
| 16 | Herkunft der EXE | Nutzer können eine untergeschobene EXE nicht von der echten unterscheiden; die SHA256-Datei liegt auf derselben Seite und schützt nur vor Übertragungsfehlern | mittel | Unsigniert; Signierung „nicht geplant“ (Owner-Entscheidung). SmartScreen-Hinweis dokumentiert | #9; Empfehlung siehe unten |
| 17 | Release-Pipeline | Release aus falschem Stand oder Token-Missbrauch | niedrig → nach #71 | Actions per SHA, getrennte Prüf- und Veröffentlichungsjobs, minimale Rechte, Upload-Ziel geprüft; **es fehlt die Prüfung, ob der Tag auf `main` liegt** | #57, #71 |
| 18 | Abhängigkeiten | Bekannte Schwachstellen in Crates oder Actions | niedrig | `cargo audit` und Dependabot (Cargo und Actions), `Cargo.lock` und Toolchain festgelegt | CI `rust-ci.yml` |

Zwei in #52 genannte, unbestätigte Hypothesen sind hier nicht enthalten. Sie werden nach [SECURITY.md](../SECURITY.md) vertraulich behandelt; der Bearbeiter bewertet sie dort mit derselben Skala. Wird eine davon „hoch“, blockiert sie v2.0.0.

## Ergebnis für v2.0.0

- **Hoch:** kein offenes bestätigtes hohes Risiko. Die zwei oben genannten Hypothesen sind weiterhin unbestätigt.
- **Mittel (kein Blocker, als bekannte Grenze dokumentieren):** Nr. 1, 2, 4, 5 (in Arbeit), 15, 16.
- **Niedrig:** keine Aufgabe für v2.0.0.

## Nachtrag v2.1.0

**Bewerteter Stand:** `main` @ `34fc190` (2026-11-08). Bewertet werden nur die neuen Funktionen; die Zeilen 1–18 gelten unverändert. Die Sicherheitsabschnitte der PRs #100, #103, #104 und #105 enthalten die Einzelprüfungen.

| # | Angriffsfläche | Mögliche Auswirkung | Risiko | Stand | Nachweis / Issue |
|---|---|---|---|---|---|
| 19 | Mauszeiger mit aufnehmen | Der Zeiger kann Bildteile verdecken; die Option läuft versehentlich mit | niedrig | Standardmäßig aus. Im Editor ist der Zeiger ein eigenes Objekt (verschieben, löschen); es werden nur Form und Position des eigenen Zeigers gelesen. Ist die Option an, schreibt ShotKit ein neues Feld in die Einstellungsdatei, das 2.0.x ablehnt (dokumentiert); ist sie aus, bleibt die Datei bytegleich zu 2.0.x | #36, PR #103; `capture_cursor.rs`, Test `default_settings_stay_valid_for_the_frozen_v1_reset_receipt` |
| 20 | Bild anheften | Das angeheftete Bild bleibt über allen Fenstern sichtbar, etwa bei einer Bildschirmfreigabe | niedrig | Nur auf ausdrücklichen Befehl (Strg+T oder Menü); kein Schreiben auf die Festplatte; Kopieren nur per Befehl; Schließen mit Esc oder Doppelklick. Das Fenster zeigt denselben Stand wie der Export, also mit Schwärzungen | #99, PR #104; `app/pin.rs` |
| 21 | Seitenende per UI Automation | Ein fremdes Programm antwortet langsam oder gar nicht; falsche Angaben eines Programms | niedrig | Nur lesende Abfrage der Scrollposition, keine Inhalte. UIA-Zeitlimits 700 ms, Gesamtfrist 1,5 s auf eigenem Thread, danach gilt „nicht bestätigt“. Eine falsche Angabe ändert nur den Meldungstext, nie das Bild. Diagnoseausgabe nur über eine Umgebungsvariable und nur in den Temp-Ordner, ohne Inhalte | #88, PR #105; `scroll_end_uia.rs`; Gegenprobe `-StopHalfway` |
| 22 | Rissrand | Speichererschöpfung bei sehr großen Bildern | niedrig | Rechnet innerhalb des gemeinsamen Bildbudgets wie Rahmen und Schatten; keine neue Eingabequelle | #98, PR #100; `image_edit.rs` |

**`unsafe`-Fläche (Nr. 15):** Das Audit führt jetzt 170 Funktionen, davon 127 ohne gesonderten Nachweis; neu in 2.1.0 sind unter anderem das Anheftfenster und die UIA-Abfrage (Abschnitt „Ergänzungen v2.1.0“ in [UNSAFE_AUDIT.md](UNSAFE_AUDIT.md)). Weiter #69.

**Ergebnis für v2.1.0:** kein neues hohes oder mittleres Risiko. Die bekannten Grenzen aus v2.0.0 gelten weiter.

## Empfehlungen nach 2.0.0

- **Herkunftsnachweis ohne Zertifikat (Nr. 16):** Der Release-Workflow kann mit `actions/attest-build-provenance` eine kostenlose GitHub-Attestierung erzeugen. Nutzer prüfen dann mit `gh attestation verify shotkit-….exe -R BomBastikDE/shotkit`, dass die EXE aus diesem Repository und Workflow stammt. Der Aufwand ist gering; ein Code-Signing-Zertifikat ist dafür nicht nötig.
- Weichzeichnen messen wie Verpixeln (Nr. 2).
- Diagnoseablage nur nach Rückfrage (Nr. 6).
- `unsafe`-Fläche verkleinern (#69).
