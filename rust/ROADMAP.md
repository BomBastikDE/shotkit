# Rust-Portierung: Ausblick

Referenz für die angestrebte Funktionsgleichheit ist ShotKit C# v1.2.0, Commit `cd5fff3`. Die C#-Version bleibt bis zur Freigabe der Rust-Beta die stabile Hauptversion. Den aktuellen Funktions- und Nachweisstand dokumentiert [PARITY.md](PARITY.md); frühere CI-Läufe stehen im [Änderungsprotokoll](CHANGELOG.md).

## Bis zur Beta

- Fehlende Funktionen und noch nicht belegte Referenzabläufe aus der Paritätstabelle bearbeiten und durch passende Tests nachweisen.
- Den vorläufig nicht blockierenden Werkzeugleisten-Test wieder verbindlich machen und grün ausführen.
- Offene Rückmeldungen aus [WINDOWS_PRAXISTEST.md](WINDOWS_PRAXISTEST.md) bearbeiten und tatsächliche Owner-Ergebnisse dokumentieren.
- Die unsignierte EXE und ihre SHA256-Prüfsumme automatisiert als Alpha bereitstellen; Code-Signing bleibt eine spätere Entscheidung.
- Die Owner-Prüfung der Oberfläche, Nummernmarkierungen und des Tray-Symbols sowie echte Tests mit gemischter DPI, mehreren Monitoren und weiteren Browsern bleiben ausstehend. Automatische Tests ersetzen sie nicht.

## Nach der Beta

- Den globalen `static mut STATE` des Auswahl-Overlays durch fenstergebundenen Zustand (`GWLP_USERDATA`) oder thread-lokalen Zustand ersetzen. Das soll Zuständigkeit und Lebensdauer des Zustands klarer machen und die Prüfung verschachtelter Windows-Nachrichten erleichtern; die konkrete Lösung wird nach der Beta festgelegt.
- Code-Signing prüfen, um Warnungen bei der Verteilung der unsignierten EXE zu reduzieren.
- Mögliche spätere Erweiterungen: **vergrößerter Bildausschnitt im Editor** und **Bild anheften**. Vor einer Umsetzung werden Nutzen, Paketgröße, Bedienung und Windows-Tests bewertet und die Zustimmung eingeholt. Sprechblasen mit Pfeil und vereinfachtes Hervorheben eines Details sind weitere Ideen. Text aus Bildern/OCR ist ausdrücklich zurückgestellt.
- Über einen späteren Wechsel von Rust auf den Hauptzweig und eine Ablage der C#-Quellen unter `legacy/` entscheidet der Owner; der Tag v1.x bleibt als historische Referenz erhalten.
