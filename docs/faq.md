# Häufige Fragen und Fehlerbehebung

[← Handbuch](README.md)

<details>
<summary><b>Beim Drücken von <kbd>Druck</kbd> passiert nichts.</b></summary>

- Läuft ShotKit? Das Symbol muss im Infobereich sichtbar sein.
- Ist das Kürzel belegt? ShotKit meldet das beim Start. Unter Windows 11 belegt oft das Snipping Tool die Taste
  (siehe [Aufnehmen](aufnehmen.md#tastenkürzel)).
- Ausweichkürzel <kbd>F11</kbd> oder Linksklick auf das Symbol verwenden.

</details>

<details>
<summary><b>Nach dem Start einer neuen Version erscheint die alte.</b></summary>

Die alte Instanz läuft noch und wird nur nach vorn geholt. Rechtsklick auf das Symbol → **Beenden**, dann die
neue EXE erneut starten.

</details>

<details>
<summary><b>Die Scroll-Aufnahme bricht sofort ab.</b></summary>

Die Maus wurde bewegt, oder der Bereich ist zu klein. Die Maus während der Aufnahme nicht berühren. Bricht sie bei
mehreren Monitoren mit unterschiedlicher Skalierung ohne Mausbewegung ab, bitte in
[#6](https://github.com/BomBastikDE/shotkit/issues/6) melden.

</details>

<details>
<summary><b>Ein Element auf einer Webseite wird nicht vorgeschlagen.</b></summary>

Die Erkennung nutzt die Windows-Bedienungshilfen der jeweiligen Anwendung; nicht jedes Element ist erkennbar.
Den Bereich dann frei aufziehen.

</details>

<details>
<summary><b>Windows warnt beim Start vor der Datei.</b></summary>

Die EXE ist noch nicht signiert. Prüfsumme vergleichen (siehe [Installation](installation.md#herunterladen)),
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
