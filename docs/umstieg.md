# Umstieg von der C#-Version 1.2.0

[← Handbuch](README.md)

Die C#-Version 1.2.0 ist die letzte C#-Version und wird nicht weiterentwickelt
([Release v1.2.0](https://github.com/BomBastikDE/shotkit/releases/tag/v1.2.0), Quellcode auf Branch
[`csharp/1.x`](https://github.com/BomBastikDE/shotkit/tree/csharp/1.x)).

## Wechseln

1. Die C#-Version über ihr Tray-Menü **beenden** – sonst blockieren sich die Tastenkürzel.
2. Die Rust-EXE starten.
3. Einstellungen neu vornehmen: Die Rust-Version nutzt `%APPDATA%\ShotKitRust` und übernimmt die alten
   Einstellungen aus `%APPDATA%\ShotKit` nicht.
4. Bei Bedarf den Autostart der C#-Version ausschalten – beide Versionen haben getrennte Autostart-Einträge.

## Wichtige Unterschiede

| | C# 1.2.0 | Rust (2.0.0) |
|---|---|---|
| Scroll-Aufnahme | <kbd>Strg</kbd>+<kbd>Alt</kbd>+<kbd>Druck</kbd> | zusätzlich <kbd>F10</kbd>, <kbd>Strg</kbd>+<kbd>Umschalt</kbd>+<kbd>4</kbd>; Richtung wird vorher abgefragt |
| Bereich | <kbd>Druck</kbd> | zusätzlich <kbd>F11</kbd> |
| Bildschirm | <kbd>Strg</kbd>+<kbd>Druck</kbd> | zusätzlich <kbd>F9</kbd> |
| Automatisch kopieren/speichern | standardmäßig an | standardmäßig aus |
| <kbd>Strg</kbd>+<kbd>V</kbd> im Editor | ersetzt das Bild | öffnet einen neuen Editor |

Neu in der Rust-Version sind unter anderem Schwärzen, Weichzeichnen, Sprechblasen, Stempel, Spotlight, Bildlupe,
Formstile, Leinwandgröße und die Option, Aufnahmen vom Zwischenablageverlauf auszunehmen.

Den vollständigen Funktionsvergleich enthält die [Paritätstabelle](../rust/PARITY.md).

## Zurück zur C#-Version

Rust-Version beenden und die alte `ShotKit.exe` starten. Einstellungen und Bilder beider Versionen bleiben
erhalten.

---

[← Sicherheit](sicherheit.md) · Weiter: [Häufige Fragen →](faq.md)
