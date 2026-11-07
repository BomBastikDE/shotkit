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
| 1 | **Verpixeln** | Verpixelter Text lässt sich bei bekannter Schrift per Kandidatenvergleich zurückrechnen; laut Simulation 100 % Treffer, auch mit 24-px-Blöcken | **hoch** | Härtung beschlossen, im Plan; Werkzeug-Hinweis „keine sichere Schwärzung“ vorhanden | #5 (Simulation und Kriterium dort) |
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

- **Hoch:** nur Nr. 1 (Verpixeln). Die Behebung ist beschlossen und Teil von #72 (#5). Danach gibt es kein offenes hohes Risiko.
- **Mittel (kein Blocker, als bekannte Grenze dokumentieren):** Nr. 2, 4, 5 (in Arbeit), 15, 16.
- **Niedrig:** keine Aufgabe für v2.0.0.

## Empfehlungen nach 2.0.0

- **Herkunftsnachweis ohne Zertifikat (Nr. 16):** Der Release-Workflow kann mit `actions/attest-build-provenance` eine kostenlose GitHub-Attestierung erzeugen. Nutzer prüfen dann mit `gh attestation verify shotkit-….exe -R BomBastikDE/shotkit`, dass die EXE aus diesem Repository und Workflow stammt. Der Aufwand ist gering; ein Code-Signing-Zertifikat ist dafür nicht nötig.
- Weichzeichnen messen wie Verpixeln (Nr. 2).
- Diagnoseablage nur nach Rückfrage (Nr. 6).
- `unsafe`-Fläche verkleinern (#69).
