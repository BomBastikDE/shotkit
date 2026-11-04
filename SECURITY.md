# Sicherheitsrichtlinie

## Unterstützte Versionen

| Version | Sicherheitskorrekturen |
|---|---|
| C#-Version 1.2.x (Branch `main`, stabile Hauptversion) | ✅ |
| Rust-Version, jeweils neueste `v2.0.0-alpha.N` (Branch `rust/development`) | ✅ |
| Ältere Rust-Alphas | ❌ – bitte auf die neueste Alpha aktualisieren |
| C#-Versionen vor 1.2 | ❌ |

## Sicherheitslücke melden

**Bitte keine öffentlichen Issues für Sicherheitslücken anlegen.**

Melde Lücken vertraulich über GitHub:
**[Sicherheitslücke melden](https://github.com/BomBastikDE/shotkit/security/advisories/new)**
(Reiter „Security“ → „Report a vulnerability“).

Hilfreich sind:

- betroffene Version (z. B. `v2.0.0-alpha.19` oder `1.2.0`) und Windows-Version,
- Schritte zum Nachstellen, gegebenenfalls eine Beispieldatei,
- was ein Angreifer damit erreichen kann.

## Ablauf

- Eingangsbestätigung in der Regel innerhalb von 7 Tagen.
- Bewertung und Rückmeldung, ob und wie die Lücke behoben wird.
- Nach der Korrektur erscheint ein Security Advisory; auf Wunsch wirst du dort als Finder genannt.

ShotKit ist ein privates Hobbyprojekt ohne Bug-Bounty. Zeitangaben sind Richtwerte.

## Geltungsbereich

Relevant sind insbesondere:

- präparierte Bilddateien oder Zwischenablage-Inhalte, die Absturz, Speicherfehler oder Codeausführung auslösen,
- Wege, über die andere Programme ShotKit ohne Zutun des Nutzers zu Aktionen bringen (z. B. über die Einzelinstanz-Weitergabe),
- unsichere Verarbeitung von Pfaden, Einstellungen oder Autostart-Einträgen,
- Schwächen in Build, Release oder Veröffentlichung der EXE,
- Verpixeln, das sich zurückrechnen lässt.

Nicht im Geltungsbereich: Angriffe, die bereits volle Kontrolle über das Benutzerkonto oder Administratorrechte voraussetzen, sowie Warnungen von Windows SmartScreen zur bislang unsignierten EXE (siehe Issue #9).

## Echtheit der Downloads

Zu jeder Rust-Alpha gibt es eine `.sha256`-Datei. Sie liegt auf derselben Release-Seite und schützt daher nur vor Übertragungsfehlern, nicht vor einer Manipulation des Releases. Signierung und Herkunftsnachweis sind geplant (Issue #9).
