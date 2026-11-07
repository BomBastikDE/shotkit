# Sicherheitsrichtlinie

## Unterstützte Versionen

| Version | Sicherheitskorrekturen |
|---|---|
| ShotKit 2.x, jeweils neueste stabile Version (Branch `main`) | ✅ |
| Vorabversionen (`v2.0.0-alpha.N`, `v2.0.0-beta.N`) | ❌ – bitte auf die neueste stabile Version aktualisieren |
| C#-Version 1.2.x (Branch `csharp/1.x`) | ✅ nur Sicherheitskorrekturen, keine neuen Funktionen |
| C#-Versionen vor 1.2 | ❌ |

## Sicherheitslücke melden

**Bitte keine öffentlichen Issues für Sicherheitslücken anlegen.**

Melde Lücken vertraulich über GitHub:
**[Sicherheitslücke melden](https://github.com/BomBastikDE/shotkit/security/advisories/new)**
(Reiter „Security“ → „Report a vulnerability“).

Hilfreich sind:

- betroffene Version (z. B. `v2.0.0` oder `1.2.0`) und Windows-Version,
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
- unsichere Verarbeitung von Pfaden, Einstellungen, temporären Dateien oder Autostart-Einträgen,
- Schwächen in Build, Release oder Veröffentlichung der EXE,
- Wege, auf denen **geschwärzte** Bildinhalte in gespeicherten oder kopierten Bildern sichtbar bleiben.

Nicht im Geltungsbereich:

- Angriffe, die bereits volle Kontrolle über das Benutzerkonto oder Administratorrechte voraussetzen,
- Warnungen von Windows SmartScreen zur unsignierten EXE,
- das Zurückrechnen von **verpixelten** oder **weichgezeichneten** Bereichen. Beide Werkzeuge sind ausdrücklich keine sichere Schwärzung; die Verpixelung ist gehärtet, ihre bekannten Grenzen stehen in [`rust/SECURITY_REVIEW.md`](rust/SECURITY_REVIEW.md). Hinweise auf praktikable Angriffe nehmen wir trotzdem gern als normales Issue entgegen.

Die Bewertung der Angriffsflächen steht in [`rust/SECURITY_REVIEW.md`](rust/SECURITY_REVIEW.md).

## Echtheit der Downloads

Zu jedem Release gibt es eine `.sha256`-Datei. Sie liegt auf derselben Release-Seite und schützt daher nur vor Übertragungsfehlern, nicht vor einer Manipulation des Releases. Die EXE ist nicht signiert; eine Signierung ist derzeit nicht geplant. Lade ShotKit nur von der offiziellen [Releases-Seite](https://github.com/BomBastikDE/shotkit/releases).
