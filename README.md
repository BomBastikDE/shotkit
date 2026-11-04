# ShotKit

Schlankes Screenshot- und Bildbearbeitungswerkzeug für Windows 10/11 mit deutscher Oberfläche.
Geschrieben in Rust, eine einzige EXE, keine Installation nötig.

> **Status: Alpha.** ShotKit wird in Rust neu entwickelt; der Code liegt in [`rust/`](rust/).
> Funktionsumfang und Windows-Verhalten sind noch nicht vollständig geprüft – siehe
> [Paritätstabelle](rust/PARITY.md) und [Roadmap](rust/ROADMAP.md). Rückmeldungen und Fehlermeldungen sind willkommen (Issues).
> Sicherheitslücken bitte vertraulich melden, siehe [SECURITY.md](SECURITY.md).

## Download

Die neueste Rust-Alpha (`shotkit-v2.0.0-alpha.N-windows-x64.exe`) von der
[Releases-Seite](https://github.com/BomBastikDE/shotkit/releases) laden und mit der zugehörigen `.sha256`-Datei prüfen:

```powershell
Get-FileHash .\shotkit-*.exe -Algorithm SHA256
```

Die EXE ist noch unsigniert; Windows SmartScreen kann warnen („Weitere Informationen → Trotzdem ausführen“).
Signierung und Herkunftsnachweis sind geplant (#9).

## Dokumentation

- **[Benutzerhandbuch](docs/README.md)** – Installation, Aufnehmen, Editor, Einstellungen, Sicherheit, häufige Fragen
- [Technische Übersicht, Bauen und Testen](rust/README.md) · [Testprotokoll](rust/TESTPROTOKOLL.md) · [Änderungen](rust/CHANGELOG.md)

## Bisherige C#-Version (1.2.0)

Bis zur ersten Rust-Beta ist die C#-Version **1.2.0** die letzte als stabil freigegebene Version
(Release [`v1.2.0`](https://github.com/BomBastikDE/shotkit/releases/tag/v1.2.0)). Ihr Quellcode liegt auf dem Branch
[`csharp/1.x`](https://github.com/BomBastikDE/shotkit/tree/csharp/1.x); dort gibt es nur noch Sicherheitskorrekturen.

Umstieg: C#-ShotKit über das Tray-Menü beenden, dann die Rust-EXE starten. Rust verwendet eigene Einstellungen unter
`%APPDATA%\ShotKitRust` und übernimmt oder ändert die C#-Einstellungen (`%APPDATA%\ShotKit`) nicht. Ausgabeoptionen
und Autostart deshalb im Rust-Tray-Menü neu einstellen und gegebenenfalls den C#-Autostart ausschalten.
Für die Rückkehr Rust beenden und die C#-EXE starten; Einstellungen und Bilder bleiben erhalten.

## Lizenz

MIT, siehe [LICENSE](LICENSE).
