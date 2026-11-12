# Häufige Fragen und Fehlerbehebung

[← Handbuch](README.md)

<details>
<summary><b>Beim Drücken von <kbd>Druck</kbd> passiert nichts.</b></summary>

- Läuft ShotKit? Das Symbol muss im Infobereich sichtbar sein.
- Ist das Kürzel belegt? ShotKit meldet das beim Start („… ist belegt“ bzw. „Nicht verfügbare Tastenkürzel“). Unter Windows 11 belegt oft das Snipping Tool die Taste
  (siehe [Aufnehmen](aufnehmen.md#tastenkürzel)).
- Ausweichkürzel <kbd>F11</kbd> oder Linksklick auf das Symbol verwenden.

</details>

<details>
<summary><b>Nach dem Start einer neuen Version erscheint die alte.</b></summary>

Die alte Instanz läuft noch und wird nur nach vorn geholt. Rechtsklick auf das Symbol → **Beenden**, dann die
neue EXE erneut starten.

</details>

<details>
<summary><b>Die Scroll-Aufnahme bricht sofort ab oder endet vorzeitig.</b></summary>

- Du hast <kbd>Esc</kbd> gedrückt oder die Maus bewegt – das beendet die Aufnahme gewollt. Die Maus während der
  Aufnahme nicht berühren.
- Der Bereich war zu klein: mindestens 100 × 100 Pixel und 200 Pixel in Scrollrichtung.
- ShotKit konnte einen Bildabschnitt nicht eindeutig zuordnen oder das Ende nicht sicher erkennen. Auch bei
  mehreren Monitoren mit unterschiedlicher Skalierung kann das vorkommen. Das bis dahin Aufgenommene bleibt als
  Teilbild erhalten; das Fenster „Scroll-Aufnahme beendet“ nennt den Grund. Bricht sie bei unterschiedlicher
  Skalierung ohne Mausbewegung ab, bitte in [#6](https://github.com/BomBastikDE/shotkit/issues/6) melden.

</details>

<details>
<summary><b>Was ist der Ordner „Diagnose“ im Speicherordner?</b></summary>

Konnte ShotKit bei einer Scroll-Aufnahme einen Abschnitt nicht zuordnen, hat es dort zwei Bilder und eine Textdatei
abgelegt (Unterordner `Scroll_…`). Sie helfen bei der Fehlersuche, können aber vertrauliche Inhalte enthalten und
werden nicht automatisch gelöscht. Siehe [Sicherheit](sicherheit.md#diagnoseordner-der-scroll-aufnahme).

</details>

<details>
<summary><b>„Einstellungen zurücksetzen?“ erscheint beim Start.</b></summary>

Die Einstellungsdatei ist ungültig, ShotKit erkennt sie aber eindeutig als Version 1. Mit **Ja** setzt ShotKit
sie auf Standardwerte zurück und behält die unveränderte Originaldatei samt Beleg als Sicherung im
Einstellungsordner (`%APPDATA%\ShotKitRust`); diese Dateien werden nicht automatisch gelöscht. Mit **Nein** bleibt
die Datei unverändert und das Speichern der Einstellungen ist abgeschaltet. Nach einer unterbrochenen
Wiederherstellung müssen die Dateien dort von Hand geprüft werden.

</details>

<details>
<summary><b>„Das Speicherlimit reicht für diese Bearbeitung nicht aus …“</b></summary>

Der Rückgängig-Verlauf eines Bildes ist begrenzt. ShotKit fragt, ob ältere Schritte entfernt oder der gesamte
Verlauf gelöscht werden soll (dann ist die Bearbeitung nicht rückgängig zu machen). Mit **Nein** bleibt alles
unverändert. Siehe [Editor](editor.md#rückgängig-verlauf).

</details>

<details>
<summary><b>Beim Öffnen einer Datei warnt ShotKit vor einem Netzwerkserver.</b></summary>

Die Datei liegt auf einem Netzlaufwerk oder UNC-Pfad. Beim Öffnen kontaktiert ShotKit den Server, und Windows kann
dabei deine Anmeldedaten automatisch verwenden. Öffne die Datei nur, wenn du dem Server vertraust. Siehe
[Sicherheit](sicherheit.md#dateien-von-netzlaufwerken).

</details>

<details>
<summary><b>Ein sehr großes Bild wird abgelehnt oder lässt sich nicht speichern.</b></summary>

Sehr große Bilder können beim Öffnen, Bearbeiten, Kopieren oder Speichern abgelehnt werden. Das Dokument bleibt
erhalten, eine vorhandene Zieldatei wird nicht verändert. Weitere geöffnete große Bilder verringern den verfügbaren Spielraum.

</details>

<details>
<summary><b>Ein Element auf einer Webseite wird nicht vorgeschlagen.</b></summary>

Die Erkennung nutzt die Windows-Bedienungshilfen der jeweiligen Anwendung; nicht jedes Element ist erkennbar.
Den Bereich dann frei aufziehen.

</details>

<details>
<summary><b>Windows warnt beim Start vor der Datei.</b></summary>

Die EXE ist nicht signiert; eine Signierung ist derzeit nicht geplant. Prüfsumme vergleichen (siehe [Installation](installation.md#herunterladen)),
dann „Weitere Informationen“ → „Trotzdem ausführen“.

</details>

## Fehler melden

Neues [Issue](https://github.com/BomBastikDE/shotkit/issues/new) mit:

- ShotKit-Version (Rechtsklick → **Über ShotKit**) und Windows-Version
- Anzahl der Monitore und ihre Skalierung
- Schritten zum Nachstellen, erwartetem und tatsächlichem Ergebnis
- Screenshot, sofern er keine vertraulichen Inhalte zeigt

---

[← Umstieg](umstieg.md) · [Zur Übersicht](README.md)
