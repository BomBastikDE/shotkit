# ShotKit

Schlankes Screenshot- und Bildbearbeitungswerkzeug für Windows 10/11 mit deutscher Oberfläche.
Geschrieben in Rust, eine einzige EXE, keine Installation nötig. Die Entwicklung erfolgt auf `main`.

> **Status: stabil – Version 2.0.0.** Erste stabile Rust-Version; der Code liegt in [`rust/`](rust/).
> Was neu ist und welche Grenzen bekannt sind, steht im [Änderungsprotokoll](rust/CHANGELOG.md); der Ausblick in der
> [Roadmap](rust/ROADMAP.md). Rückmeldungen und Fehlermeldungen sind willkommen (Issues).
> Sicherheitslücken bitte vertraulich melden, siehe [SECURITY.md](SECURITY.md).

## Download

Die neueste Version (z. B. `shotkit-v2.0.0-windows-x64.exe`) von der
[Releases-Seite](https://github.com/BomBastikDE/shotkit/releases/latest) laden und mit der zugehörigen `.sha256`-Datei prüfen:

```powershell
Get-FileHash .\shotkit-*.exe -Algorithm SHA256
```

Die EXE ist nicht signiert; Windows SmartScreen kann warnen („Weitere Informationen → Trotzdem ausführen“).
Eine Signierung ist derzeit nicht geplant.

## Dokumentation

- **[Benutzerhandbuch](docs/README.md)** – Installation, Aufnehmen, Editor, Einstellungen, Sicherheit, häufige Fragen
- [Technische Übersicht, Bauen und Testen](rust/README.md) · [Testprotokoll](rust/TESTPROTOKOLL.md) · [Änderungen](rust/CHANGELOG.md)

## Bisherige C#-Version (1.2.0)

Die C#-Version **1.2.0** war die letzte C#-Version
(Release [`v1.2.0`](https://github.com/BomBastikDE/shotkit/releases/tag/v1.2.0)); ShotKit 2.0.0 löst sie ab. Ihr Quellcode liegt auf dem Branch
[`csharp/1.x`](https://github.com/BomBastikDE/shotkit/tree/csharp/1.x); dort gibt es nur noch Sicherheitskorrekturen.

Umstieg: C#-ShotKit über das Tray-Menü beenden, dann die Rust-EXE starten. Rust verwendet eigene Einstellungen unter
`%APPDATA%\ShotKitRust` und übernimmt oder ändert die C#-Einstellungen (`%APPDATA%\ShotKit`) nicht. Ausgabeoptionen
und Autostart deshalb im Rust-Tray-Menü neu einstellen und gegebenenfalls den C#-Autostart ausschalten.
Für die Rückkehr Rust beenden und die C#-EXE starten; Einstellungen und Bilder bleiben erhalten.

## Lizenz

MIT, siehe [LICENSE](LICENSE).
