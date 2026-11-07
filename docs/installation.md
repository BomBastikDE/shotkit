# Installation und erster Start

[← Handbuch](README.md)

## Herunterladen

1. Auf der [Releases-Seite](https://github.com/BomBastikDE/shotkit/releases) die neueste Version
   (`shotkit-v2.0.0-windows-x64.exe`) und die gleichnamige `.sha256`-Datei laden.
2. Optional die Echtheit der Übertragung prüfen (PowerShell im Download-Ordner):

   ```powershell
   Get-FileHash .\shotkit-*.exe -Algorithm SHA256
   ```

   Der angezeigte Wert muss mit dem Inhalt der `.sha256`-Datei übereinstimmen.

> [!NOTE]
> Eine Installation ist nicht nötig – ShotKit ist eine einzelne EXE.

## Starten

Die EXE per Doppelklick starten. Beim ersten Start ohne vorhandene Einstellungen zeigt ShotKit einmal einen Hinweis zu Infobereich und Tastenkürzeln. Beim Start mit einer Bilddatei oder über den Autostart erscheint der Hinweis nicht.

> [!WARNING]
> Die EXE ist nicht digital signiert. Windows SmartScreen zeigt deshalb eine Warnung:
> „Weitere Informationen“ → „Trotzdem ausführen“. Mehr dazu unter
> [Sicherheit](sicherheit.md#echtheit-der-exe).

ShotKit läuft danach im **Infobereich der Taskleiste** (rotes ShotKit-Symbol, ggf. unter dem Pfeil
„Ausgeblendete Symbole“):

- **Linksklick** auf das Symbol: Bereich aufnehmen
- **Rechtsklick**: Menü mit allen Aufnahmearten, Editor, Bild öffnen, Einstellungen, „Über ShotKit“ und „Beenden“ (siehe [Einstellungen](einstellungen.md#menü-im-infobereich))

Ist ein Tastenkürzel schon von einem anderen Programm belegt, meldet ShotKit das direkt nach dem Start
(siehe [Aufnehmen](aufnehmen.md#tastenkürzel)).

## Automatisch mit Windows starten

Rechtsklick → **Einstellungen** → **Mit Windows starten**. Steht dort „Autostart nicht verfügbar“, lässt sich die Option nicht ändern.

## Aktualisieren

Läuft bereits eine ShotKit-Instanz, holt ein erneuter Start nur die laufende nach vorn. Vor einem Update deshalb
die alte Version über Rechtsklick → **Beenden** schließen, dann die neue EXE starten.

---

Weiter: [Aufnehmen →](aufnehmen.md)
