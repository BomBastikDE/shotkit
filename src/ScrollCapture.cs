using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

namespace ShotKit;

enum ScrollDirection { Down, Right, Left }

/// <summary>
/// Scroll-Aufnahme: nach Klick auf „Start“ wird der gewählte Bereich per Mausrad weitergescrollt,
/// nach jedem Schritt ein Bild aufgenommen und alles zu einem langen Bild zusammengesetzt.
///
/// Abgleich (Mehrheitsentscheid): Das neue Bild wird in Bänder von je 40 Zeilen geteilt. Jedes Band
/// sucht unabhängig, um wie viele Pixel es sich gegenüber dem vorherigen Bild verschoben hat
/// (toleranter Vergleich grober Helligkeitsprofile). Mehrdeutige Bänder (leere Flächen, Verläufe)
/// enthalten sich. Die Verschiebung mit den meisten Stimmen gewinnt – Ausreißer wie halbtransparente
/// Kopfleisten, Animationen oder Werbung überstimmen den Inhalt so nicht. Gewinnt „0“, ist das Ende erreicht.
/// Rechts/links wird über 90°-Drehung auf denselben Ablauf zurückgeführt.
/// </summary>
sealed class ScrollCapture
{
    const int MaxHeight = 30000;       // GDI+/Steuerelement-Grenzen bleiben sicher darunter
    const int MaxSteps = 150;
    const int MinLength = 200;         // kürzer: schon eine Mausrad-Raste kann die Überlappung verhindern
    const int MinOverlap = 24;         // Mindestüberlappung zwischen zwei Bildern (Zeilen)
    const int Band = 40;               // Zeilen je Band für den Mehrheitsentscheid
    const int MinBandRows = 6;         // Zeilen mit Struktur, die ein Band mindestens braucht
    const int BinTolerance = 10;       // erlaubte Helligkeitsabweichung je Profilwert (0–255)
    const double RowShare = 0.75;      // Anteil passender Profilwerte, damit eine Zeile als gleich gilt
    const int MinRowContrast = 6;      // Zeilen mit weniger Kontrast (leerer Hintergrund) zählen nicht
    const int StaticTolerance = 1;     // „steht fest“: Abweichung je Profilwert höchstens 1

    sealed class Frame : IDisposable
    {
        public Bitmap Bitmap;
        public ulong[] Hash;   // exakter Zeilen-Hash
        public byte[] Sig;     // grobes Helligkeitsprofil: K Werte je Zeile
        public int K;
        public int Height => Hash.Length;
        public void Dispose() => Bitmap.Dispose();
    }

    /// <summary>Dy == 0: keine Bewegung (Ende erreicht).</summary>
    sealed record MatchResult(int Dy, int Top, int Bottom);

    readonly Rectangle _rect;
    readonly ScrollDirection _dir;
    readonly CancellationToken _ct;
    readonly Action<string> _status;
    Point _cursorTarget;
    string _lastMatchInfo;

    public string StopReason { get; private set; }

    ScrollCapture(Rectangle rect, ScrollDirection dir, CancellationToken ct, Action<string> status)
    {
        _rect = rect;
        _dir = dir;
        _ct = ct;
        _status = status;
    }

    /// <summary>
    /// Zeigt Rahmen + Auswahlfenster; erst nach Klick auf „Start“ wird in der gewählten Richtung gescrollt.
    /// image == null: abgebrochen bzw. nicht möglich (info enthält dann ggf. den Grund).
    /// </summary>
    public static async Task<(Bitmap image, string info)> RunAsync(Rectangle screenRect, ScrollDirection initial,
        CancellationToken ct, Action<ScrollDirection> rememberDirection)
    {
        using var frame = new SelectionFrame(screenRect);
        using var panel = new ScrollPanel(screenRect, initial);
        frame.Show();
        panel.Show();
        try
        {
            var dir = await panel.WaitForStartAsync(ct);
            if (dir == null) return (null, null);
            rememberDirection(dir.Value);
            int length = dir.Value == ScrollDirection.Down ? screenRect.Height : screenRect.Width;
            if (length < MinLength)
                return (null, $"Der Bereich ist in Scrollrichtung zu kurz (mind. {MinLength} px) – bitte größer wählen.");
            panel.SetRunning(dir.Value);
            var sc = new ScrollCapture(screenRect, dir.Value, ct, panel.SetStatus);
            var bmp = await sc.Run();
            return (bmp, sc.StopReason);
        }
        finally
        {
            panel.Close();
            frame.Close();
        }
    }

    async Task<Bitmap> Run()
    {
        var originalCursor = Cursor.Position;
        // Mauszeiger an den Rand der Scrollrichtung (Scrollleisten-Zone)
        _cursorTarget = _dir == ScrollDirection.Down
            ? new Point(_rect.Right - Math.Min(8, _rect.Width / 4), _rect.Y + _rect.Height / 2)
            : new Point(_rect.X + _rect.Width / 2, _rect.Bottom - Math.Min(8, _rect.Height / 4));
        var segments = new List<(Bitmap bmp, int height)>();
        Frame prev = null;
        // nach Drehung: W = Breite quer zur Scrollrichtung, H = Länge in Scrollrichtung
        int W = _dir == ScrollDirection.Down ? _rect.Width : _rect.Height;
        int H = _dir == ScrollDirection.Down ? _rect.Height : _rect.Width;

        try
        {
            await Task.Delay(200);   // Overlay vollständig verschwinden lassen
            prev = await CaptureSettled();
            // Nahtstellen liegen mittig zwischen zwei Aufnahmen: jeder Abschnitt kommt aus dem Bild, in dem er
            // am weitesten in der Mitte lag. Ränder mit Bannern, Schatten, fixierten Leisten werden so nur
            // einmal (ganz oben/unten) übernommen.
            int start = 0;          // erste noch nicht übernommene Zeile in prev
            int center = -1;        // Mitte des bewegten Bereichs (aus dem ersten Abgleich)
            int total = 0, notches = 1, expected = H / 3, still = 0;
            float knownPxPerNotch = 0;      // gemessene Scrollweite je Raste (für das Sicherheitsnetz)
            int lastTop = 0, lastBottom = 0, fallbacks = 0;

            for (int step = 0; step < MaxSteps; step++)
            {
                if (_ct.IsCancellationRequested) { StopReason = "Mit Esc beendet."; break; }
                if (step > 0 && Cursor.Position != _cursorTarget) { StopReason = "Durch Mausbewegung beendet."; break; }
                if (total + H >= MaxHeight) { StopReason = $"Maximale Länge von {MaxHeight} px erreicht."; break; }

                Cursor.Position = _cursorTarget;
                Scroll(notches);
                await Task.Delay(150);
                var cur = await CaptureSettled();

                var m = Same(prev, cur) ? new MatchResult(0, 0, 0) : FindOffset(prev, cur, expected);
                if (m == null && notches > 1)
                {
                    // evtl. zu weit gescrollt (keine Überlappung) -> zurück und mit halber Schrittweite erneut
                    cur.Dispose();
                    Scroll(-notches);
                    notches = Math.Max(1, notches / 2);
                    expected = Math.Max(1, expected / 2);
                    await Task.Delay(150);
                    prev.Dispose();
                    prev = await CaptureSettled();
                    continue;
                }
                if (m == null)
                {
                    // evtl. werden noch Bilder nachgeladen/eingeblendet: kurz warten, neu vergleichen
                    await Task.Delay(700);
                    cur.Dispose();
                    cur = await CaptureSettled();
                    m = Same(prev, cur) ? new MatchResult(0, 0, 0) : FindOffset(prev, cur, expected);
                }
                if (m == null && knownPxPerNotch > 0)
                {
                    // Sicherheitsnetz: Browser scrollen je Raste gleich weit -> gemessene Weite verwenden
                    m = new MatchResult((int)Math.Round(knownPxPerNotch * notches), lastTop, lastBottom);
                    fallbacks++;
                }
                if (m == null)
                {
                    string diag = SaveDiagnostics(prev, cur);
                    cur.Dispose();
                    StopReason = "Bildinhalt ließ sich nicht eindeutig zuordnen – Aufnahme bis hierhin übernommen."
                               + (diag != null ? $"\nDiagnose gespeichert: {diag}" : "");
                    break;
                }

                if (m.Dy == 0)
                {
                    cur.Dispose();
                    // Ende der Seite – ein zweiter Versuch, falls Inhalte nachgeladen werden
                    if (++still >= 2) { StopReason = "Ende des scrollbaren Bereichs erreicht."; break; }
                    await Task.Delay(400);
                    continue;
                }
                still = 0;

                float ppn = (float)m.Dy / notches;
                if (knownPxPerNotch == 0 || Math.Abs(ppn - knownPxPerNotch) < 2) knownPxPerNotch = ppn;
                lastTop = m.Top;
                lastBottom = m.Bottom;

                if (center < 0) center = (m.Top + H - m.Bottom) / 2;
                int dy = m.Dy;
                int seam = Math.Clamp(center + dy / 2, start, H);      // Naht in prev-Koordinaten
                if (seam > start)
                {
                    segments.Add((ScreenCapture.Copy(prev.Bitmap, new Rectangle(0, start, W, seam - start)), seam - start));
                    total += seam - start;
                }
                start = Math.Max(0, seam - dy);                          // dieselbe Stelle in cur

                // Scrollweite anpassen: ca. 40 % des beweglichen Bereichs pro Schritt
                int movable = H - m.Top - m.Bottom;
                notches = Math.Clamp((int)(movable * 0.4f / Math.Max(1f, ppn)), 1, 15);
                expected = (int)(ppn * notches);

                prev.Dispose();
                prev = cur;
                _status($"{W} × {total + H - start} px");
            }
            StopReason ??= "Maximale Anzahl Scroll-Schritte erreicht.";
            if (fallbacks > 0)
                StopReason += $"\n{fallbacks}× wurde die gemessene Scrollweite verwendet – bitte Übergänge kurz prüfen.";

            // Rest des letzten Bildes inkl. Fußbereich
            segments.Add((ScreenCapture.Copy(prev.Bitmap, new Rectangle(0, start, W, H - start)), H - start));
            total += H - start;
            var result = Compose(segments, W, total);
            if (_dir == ScrollDirection.Right) result.RotateFlip(RotateFlipType.Rotate270FlipNone);
            else if (_dir == ScrollDirection.Left) result.RotateFlip(RotateFlipType.Rotate90FlipNone);
            return result;
        }
        finally
        {
            prev?.Dispose();
            foreach (var s in segments) s.bmp.Dispose();
            Cursor.Position = originalCursor;
        }
    }

    static Bitmap Compose(List<(Bitmap bmp, int height)> segments, int width, int total)
    {
        var result = new Bitmap(width, Math.Max(1, total), PixelFormat.Format32bppArgb);
        using var g = Graphics.FromImage(result);
        g.CompositingMode = CompositingMode.SourceCopy;
        int y = 0;
        foreach (var (bmp, h) in segments)
        {
            if (h <= 0) continue;
            g.DrawImage(bmp, new Rectangle(0, y, width, h), new Rectangle(0, 0, width, h), GraphicsUnit.Pixel);
            y += h;
        }
        return result;
    }

    // ---------- Aufnahme ----------

    void Scroll(int notches)
    {
        switch (_dir)
        {
            case ScrollDirection.Down: Native.mouse_event(Native.MOUSEEVENTF_WHEEL, 0, 0, -120 * notches, UIntPtr.Zero); break;
            case ScrollDirection.Right: Native.mouse_event(Native.MOUSEEVENTF_HWHEEL, 0, 0, 120 * notches, UIntPtr.Zero); break;
            case ScrollDirection.Left: Native.mouse_event(Native.MOUSEEVENTF_HWHEEL, 0, 0, -120 * notches, UIntPtr.Zero); break;
        }
    }

    /// <summary>So lange aufnehmen, bis das Bild zur Ruhe kommt (sanftes Scrollen, Nachladen).</summary>
    async Task<Frame> CaptureSettled()
    {
        var f = Grab();
        for (int i = 0; i < 12; i++)
        {
            await Task.Delay(90);
            var g = Grab();
            if (Settled(f, g)) { g.Dispose(); return f; }
            f.Dispose();
            f = g;
        }
        return f;
    }

    Frame Grab()
    {
        var bmp = ScreenCapture.Capture(_rect);
        // so drehen, dass neuer Inhalt immer unten erscheint
        if (_dir == ScrollDirection.Right) bmp.RotateFlip(RotateFlipType.Rotate90FlipNone);
        else if (_dir == ScrollDirection.Left) bmp.RotateFlip(RotateFlipType.Rotate270FlipNone);
        return Analyze(bmp);
    }

    static Frame Analyze(Bitmap bmp)
    {
        int w = bmp.Width, h = bmp.Height;
        // Randspalten (Scrollleiste, Mauszeiger-Hover) nicht vergleichen
        int margin = w > 60 ? Math.Min(24, w / 10) : 0;
        int cols = w - 2 * margin;
        int k = Math.Clamp(cols / 12, 8, 64);
        var hash = new ulong[h];
        var sig = new byte[h * k];
        var row = new int[w];
        var sums = new int[k];
        var counts = new int[k];
        for (int x = 0; x < cols; x++) counts[x * k / cols]++;

        var data = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        try
        {
            for (int y = 0; y < h; y++)
            {
                Marshal.Copy(IntPtr.Add(data.Scan0, y * data.Stride), row, 0, w);
                ulong hv = 14695981039346656037UL;
                Array.Clear(sums);
                for (int x = 0; x < cols; x++)
                {
                    int px = row[x + margin];
                    hv = (hv ^ (uint)px) * 1099511628211UL;
                    sums[x * k / cols] += (((px >> 16) & 0xFF) * 77 + ((px >> 8) & 0xFF) * 150 + (px & 0xFF) * 29) >> 8;
                }
                hash[y] = hv;
                for (int i = 0; i < k; i++) sig[y * k + i] = (byte)(sums[i] / counts[i]);
            }
        }
        finally
        {
            bmp.UnlockBits(data);
        }
        return new Frame { Bitmap = bmp, Hash = hash, Sig = sig, K = k };
    }

    static bool Same(Frame a, Frame b) => a.Hash.AsSpan().SequenceEqual(b.Hash);

    /// <summary>Zur Ruhe gekommen: mind. 90 % der Zeilen pixelgleich (kleine Animationen erlaubt).</summary>
    static bool Settled(Frame a, Frame b)
    {
        int same = 0;
        for (int i = 0; i < a.Height; i++) if (a.Hash[i] == b.Hash[i]) same++;
        return same >= a.Height * 0.9;
    }

    // ---------- Abgleich ----------

    /// <summary>Spalten-Abschnitte, die sich bewegen (feste Seitenleisten/Scrollleisten fallen heraus).</summary>
    static int[] ActiveBins(Frame a, Frame b)
    {
        int k = a.K, h = a.Height;
        var list = new List<int>();
        for (int c = 0; c < k; c++)
        {
            int eq = 0;
            for (int y = 0; y < h; y++)
                if (Math.Abs(a.Sig[y * k + c] - b.Sig[y * k + c]) <= StaticTolerance) eq++;
            if (eq < h * 0.97) list.Add(c);
        }
        return list.ToArray();
    }

    static bool HasStructure(Frame f, int row, int[] act)
    {
        int o = row * f.K, min = 255, max = 0;
        foreach (int c in act)
        {
            int v = f.Sig[o + c];
            if (v < min) min = v;
            if (v > max) max = v;
        }
        return max - min >= MinRowContrast;
    }

    static bool RowsStatic(Frame a, Frame b, int row, int[] act)
    {
        int o = row * a.K;
        foreach (int c in act)
            if (Math.Abs(a.Sig[o + c] - b.Sig[o + c]) > StaticTolerance) return false;
        return true;
    }

    /// <summary>Zeile ia aus a gegen Zeile ib aus b: passt (Mehrheit der Abschnitte), mittlere Abweichung.</summary>
    static bool RowsMatch(Frame a, int ia, Frame b, int ib, int[] act, out float err)
    {
        int oa = ia * a.K, ob = ib * b.K, ok = 0, sum = 0, allowedMiss = act.Length - (int)Math.Ceiling(act.Length * RowShare), miss = 0;
        foreach (int c in act)
        {
            int d = Math.Abs(a.Sig[oa + c] - b.Sig[ob + c]);
            if (d <= BinTolerance) { ok++; sum += d; }
            else if (++miss > allowedMiss) { err = 0; return false; }
        }
        err = ok > 0 ? (float)sum / ok : 0;
        return true;
    }

    /// <summary>Fest stehende Zeilen am Rand (Kopf-/Fußzeile): nur Zeilen mit Struktur zählen als Beleg.</summary>
    static int StaticEdge(Frame a, Frame b, int[] act, int start, int step, int limit)
    {
        int last = 0;
        for (int c = 0, row = start; c < limit; c++, row += step)
        {
            if (!HasStructure(b, row, act)) continue;
            if (RowsStatic(a, b, row, act)) last = c + 1;
            else break;
        }
        return last;
    }

    MatchResult FindOffset(Frame a, Frame b, int expected)
    {
        int h = a.Height;
        var act = ActiveBins(a, b);
        if (act.Length < 4) { _lastMatchInfo = "keine bewegten Spalten"; return new MatchResult(0, 0, 0); }

        int top = StaticEdge(a, b, act, 0, 1, h / 3);
        int bottom = StaticEdge(a, b, act, h - 1, -1, h / 3);
        int maxDy = h - top - bottom - MinOverlap;
        var structured = new bool[h];
        for (int i = 0; i < h; i++) structured[i] = HasStructure(b, i, act);

        var votes = new Dictionary<int, int>();
        var bandVotes = new List<int>();
        var candVotes = new int[Math.Max(1, maxDy + 1)];
        var candErr = new float[candVotes.Length];
        var rows = new List<int>(Band);

        for (int b0 = top; b0 < h - bottom; b0 += Band)
        {
            rows.Clear();
            for (int i = b0; i < Math.Min(b0 + Band, h - bottom); i++) if (structured[i]) rows.Add(i);
            if (rows.Count < MinBandRows) continue;

            int vmax = 0, last = Math.Min(maxDy, h - 1 - rows[^1]);
            for (int dy = 0; dy <= last; dy++)
            {
                int v = 0; float e = 0;
                foreach (int i in rows)
                    if (RowsMatch(a, i + dy, b, i, act, out float er)) { v++; e += er; }
                candVotes[dy] = v;
                candErr[dy] = v > 0 ? e / v : float.MaxValue;
                if (v > vmax) vmax = v;
            }
            if (vmax < rows.Count * 0.6) continue;

            // unter den Kandidaten mit ≥ 90 % der Stimmen den mit der kleinsten Abweichung wählen
            int bestDy = -1;
            for (int dy = 0; dy <= last; dy++)
                if (candVotes[dy] >= vmax * 0.9 && (bestDy < 0 || candErr[dy] < candErr[bestDy])) bestDy = dy;
            // mehrdeutig (leere Fläche, Verlauf)? -> Band enthält sich
            bool ambiguous = false;
            for (int dy = 0; dy <= last && !ambiguous; dy++)
                if (Math.Abs(dy - bestDy) > 2 && candVotes[dy] >= vmax * 0.9 && candErr[dy] <= candErr[bestDy] + 1f) ambiguous = true;
            if (ambiguous) continue;

            votes[bestDy] = votes.GetValueOrDefault(bestDy) + candVotes[bestDy];
            bandVotes.Add(bestDy);
        }

        if (votes.Count == 0) { _lastMatchInfo = $"kein Band eindeutig (oben fest {top}, unten fest {bottom})"; return null; }

        // benachbarte Verschiebungen (±1 px) zusammenfassen
        int Score(int d) => votes.GetValueOrDefault(d - 1) + votes[d] + votes.GetValueOrDefault(d + 1);
        int winner = votes.Keys
            .OrderByDescending(Score)
            .ThenByDescending(d => votes[d])
            .ThenBy(d => Math.Abs(d - expected))
            .First();
        _lastMatchInfo = $"dy={winner}, Bänder: {string.Join(", ", bandVotes)}, oben fest {top}, unten fest {bottom}";
        return new MatchResult(winner, top, bottom);
    }

    /// <summary>Bei Fehlschlag beide Bilder + Kennwerte speichern, damit sich der Fall nachvollziehen lässt.</summary>
    string SaveDiagnostics(Frame a, Frame b)
    {
        try
        {
            string dir = Path.Combine(TrayApp.Instance.Settings.SaveFolder, "Diagnose", $"Scroll_{DateTime.Now:yyyy-MM-dd_HH-mm-ss}");
            Directory.CreateDirectory(dir);
            a.Bitmap.Save(Path.Combine(dir, "vorher.png"), ImageFormat.Png);
            b.Bitmap.Save(Path.Combine(dir, "nachher.png"), ImageFormat.Png);
            float dpi;
            using (var g = Graphics.FromHwnd(IntPtr.Zero)) dpi = g.DpiX;
            File.WriteAllText(Path.Combine(dir, "info.txt"),
                $"Richtung: {_dir}\r\nBereich: {_rect}\r\nDPI: {dpi}\r\nAbgleich: {_lastMatchInfo}\r\n");
            return dir;
        }
        catch
        {
            return null;
        }
    }
}

/// <summary>Durchklickbarer roter Rahmen um den Aufnahmebereich.</summary>
sealed class SelectionFrame : Form
{
    const int Thickness = 3;

    public SelectionFrame(Rectangle screenRect)
    {
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        ShowInTaskbar = false;
        TopMost = true;
        AutoScaleMode = AutoScaleMode.None;
        BackColor = Color.Magenta;
        TransparencyKey = Color.Magenta;
        var r = screenRect;
        r.Inflate(Thickness + 1, Thickness + 1);
        Bounds = r;
    }

    protected override bool ShowWithoutActivation => true;

    protected override CreateParams CreateParams
    {
        get
        {
            var cp = base.CreateParams;
            cp.ExStyle |= Native.WS_EX_LAYERED | Native.WS_EX_TRANSPARENT | Native.WS_EX_TOOLWINDOW | Native.WS_EX_NOACTIVATE;
            return cp;
        }
    }

    protected override void OnHandleCreated(EventArgs e)
    {
        base.OnHandleCreated(e);
        Native.SetWindowDisplayAffinity(Handle, Native.WDA_EXCLUDEFROMCAPTURE);
    }

    protected override void WndProc(ref Message m)
    {
        if (m.Msg == Native.WM_DPICHANGED) { m.Result = IntPtr.Zero; return; }
        base.WndProc(ref m);
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        using var pen = new Pen(Color.FromArgb(230, 40, 40), Thickness);
        e.Graphics.DrawRectangle(pen, Thickness / 2, Thickness / 2, Width - Thickness, Height - Thickness);
    }
}

/// <summary>
/// Steuerfenster neben dem Bereich: Richtung ankreuzen, Start/Abbrechen, danach Fortschritt.
/// Taucht selbst nicht in der Aufnahme auf.
/// </summary>
sealed class ScrollPanel : Form
{
    readonly Rectangle _target;
    readonly Label _title, _status;
    readonly RadioButton _down, _right, _left;
    readonly Button _start, _cancel;
    readonly FlowLayoutPanel _choices;
    TaskCompletionSource<ScrollDirection?> _tcs;

    public ScrollPanel(Rectangle screenRect, ScrollDirection initial)
    {
        _target = screenRect;
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        ShowInTaskbar = false;
        TopMost = true;
        AutoScaleMode = AutoScaleMode.None;
        AutoSize = true;
        AutoSizeMode = AutoSizeMode.GrowAndShrink;
        BackColor = Color.FromArgb(32, 32, 34);
        ForeColor = Color.White;
        Font = new Font("Segoe UI", 9.5f);
        Padding = new Padding(12, 10, 12, 10);

        var layout = new FlowLayoutPanel { FlowDirection = FlowDirection.TopDown, AutoSize = true, WrapContents = false, Margin = new Padding(0) };
        _title = new Label { Text = "Scroll-Aufnahme: Richtung wählen", AutoSize = true, Font = new Font("Segoe UI", 10f, FontStyle.Bold), Margin = new Padding(0, 0, 0, 6) };

        _choices = new FlowLayoutPanel { FlowDirection = FlowDirection.LeftToRight, AutoSize = true, WrapContents = false, Margin = new Padding(0) };
        _down = MakeRadio("↓  Nach unten", initial == ScrollDirection.Down);
        _right = MakeRadio("→  Nach rechts", initial == ScrollDirection.Right);
        _left = MakeRadio("←  Nach links", initial == ScrollDirection.Left);
        _choices.Controls.AddRange(new Control[] { _down, _right, _left });

        var buttons = new FlowLayoutPanel { FlowDirection = FlowDirection.LeftToRight, AutoSize = true, WrapContents = false, Margin = new Padding(0, 8, 0, 0) };
        _start = MakeButton("Start", Color.FromArgb(0, 120, 215));
        _cancel = MakeButton("Abbrechen", Color.FromArgb(70, 70, 74));
        _start.Click += (_, _) => _tcs?.TrySetResult(Selected);
        _cancel.Click += (_, _) => _tcs?.TrySetResult(null);
        buttons.Controls.AddRange(new Control[] { _start, _cancel });

        _status = new Label { Text = "Esc = Abbrechen", AutoSize = true, ForeColor = Color.FromArgb(180, 180, 180), Margin = new Padding(0, 6, 0, 0) };

        layout.Controls.AddRange(new Control[] { _title, _choices, buttons, _status });
        Controls.Add(layout);
        AcceptButton = _start;
    }

    static RadioButton MakeRadio(string text, bool chk) =>
        new() { Text = text, AutoSize = true, Checked = chk, ForeColor = Color.White, Margin = new Padding(0, 0, 14, 0), Cursor = Cursors.Hand };

    static Button MakeButton(string text, Color back)
    {
        var b = new Button { Text = text, AutoSize = true, FlatStyle = FlatStyle.Flat, BackColor = back, ForeColor = Color.White, Padding = new Padding(10, 2, 10, 2), Margin = new Padding(0, 0, 8, 0), Cursor = Cursors.Hand };
        b.FlatAppearance.BorderSize = 0;
        return b;
    }

    ScrollDirection Selected => _right.Checked ? ScrollDirection.Right : _left.Checked ? ScrollDirection.Left : ScrollDirection.Down;

    public Task<ScrollDirection?> WaitForStartAsync(CancellationToken ct)
    {
        _tcs = new TaskCompletionSource<ScrollDirection?>();
        ct.Register(() => _tcs.TrySetResult(null));
        return _tcs.Task;
    }

    public void SetRunning(ScrollDirection dir)
    {
        _choices.Visible = false;
        _start.Visible = false;
        _cancel.Visible = false;
        _title.Text = dir switch
        {
            ScrollDirection.Right => "Scrolle nach rechts …",
            ScrollDirection.Left => "Scrolle nach links …",
            _ => "Scrolle nach unten …",
        };
        _status.Text = "Esc oder Maus bewegen = Stopp";
        PlaceNearTarget();
    }

    public void SetStatus(string text) => _status.Text = $"{text}   ·   Esc oder Maus bewegen = Stopp";

    protected override void OnLoad(EventArgs e)
    {
        base.OnLoad(e);
        PlaceNearTarget();
    }

    /// <summary>Unterhalb, sonst oberhalb, sonst innen unten am Bereich platzieren.</summary>
    void PlaceNearTarget()
    {
        var screen = Screen.FromRectangle(_target).WorkingArea;
        int x = Math.Clamp(_target.X, screen.X, Math.Max(screen.X, screen.Right - Width));
        int y = _target.Bottom + 10;
        if (y + Height > screen.Bottom) y = _target.Top - Height - 10;
        if (y < screen.Top) y = _target.Bottom - Height - 10;
        Location = new Point(x, y);
    }

    protected override bool ShowWithoutActivation => true;

    protected override CreateParams CreateParams
    {
        get
        {
            var cp = base.CreateParams;
            cp.ExStyle |= Native.WS_EX_TOOLWINDOW;
            return cp;
        }
    }

    protected override void OnHandleCreated(EventArgs e)
    {
        base.OnHandleCreated(e);
        Native.SetWindowDisplayAffinity(Handle, Native.WDA_EXCLUDEFROMCAPTURE);
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        base.OnPaint(e);
        using var pen = new Pen(Color.FromArgb(90, 90, 96));
        e.Graphics.DrawRectangle(pen, 0, 0, Width - 1, Height - 1);
    }
}
