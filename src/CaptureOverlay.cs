using System.Drawing.Drawing2D;
using System.Drawing.Imaging;

namespace ShotKit;

/// <summary>
/// Vollbild-Overlay über alle Monitore: eingefrorenes Bildschirmfoto, Bereich aufziehen
/// oder Fenster per Klick wählen, Lupe mit Pixelkoordinaten.
/// </summary>
sealed class CaptureOverlay : Form
{
    const int MagPixels = 15, MagZoom = 8, MagSize = MagPixels * MagZoom;
    static readonly Color Accent = Color.FromArgb(255, 0, 150, 255);

    readonly Bitmap _shot, _dim;
    readonly Rectangle _virtual;
    readonly List<WindowInfo> _windows;
    readonly Font _font = new("Segoe UI", 12f, FontStyle.Bold, GraphicsUnit.Pixel);
    readonly Font _hintFont = new("Segoe UI", 14f, GraphicsUnit.Pixel);

    Point _cursor;
    bool _dragging;
    Point _start;
    Rectangle _selection, _hover;
    List<Rectangle> _lastDirty = new();

    public const string DefaultHint = "Ziehen: Bereich  ·  Klick: Fenster  ·  Strg: Element  ·  Pfeiltasten: 1 px  ·  Esc: Abbrechen";
    readonly string _hint;
    readonly bool _preferInner;

    public Bitmap Result { get; private set; }
    /// <summary>Gewählter Bereich in Bildschirmkoordinaten.</summary>
    public Rectangle SelectedRect { get; private set; }

    /// <param name="preferInner">true: Klick wählt standardmäßig das Inhaltselement (z. B. Webseite ohne Browserleisten), Strg das ganze Fenster.</param>
    public CaptureOverlay(Bitmap shot, Rectangle virtualScreen, List<WindowInfo> windows, string hint = DefaultHint, bool preferInner = false)
    {
        _hint = hint;
        _preferInner = preferInner;
        _shot = shot;
        _virtual = virtualScreen;
        _windows = windows;

        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        ShowInTaskbar = false;
        TopMost = true;
        AutoScaleMode = AutoScaleMode.None;
        KeyPreview = true;
        Cursor = Cursors.Cross;
        SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer |
                 ControlStyles.UserPaint | ControlStyles.Opaque, true);
        Bounds = virtualScreen;

        _dim = new Bitmap(shot.Width, shot.Height, PixelFormat.Format32bppPArgb);
        using var g = Graphics.FromImage(_dim);
        g.DrawImage(shot, new Rectangle(0, 0, shot.Width, shot.Height));
        using var veil = new SolidBrush(Color.FromArgb(120, 0, 0, 0));
        g.FillRectangle(veil, 0, 0, shot.Width, shot.Height);
    }

    protected override void OnShown(EventArgs e)
    {
        base.OnShown(e);
        Bounds = _virtual;   // nach evtl. DPI-Anpassung bei Handle-Erstellung erneut setzen
        Activate();
        _cursor = PointToClient(Cursor.Position);
        UpdateHover();
        Invalidate();
    }

    protected override void WndProc(ref Message m)
    {
        // Overlay spannt über Monitore mit verschiedener Skalierung -> keine automatische Größenanpassung
        if (m.Msg == Native.WM_DPICHANGED) { m.Result = IntPtr.Zero; return; }
        base.WndProc(ref m);
    }

    Rectangle ActiveRect => _dragging ? _selection : _hover;

    // ---------- Eingabe ----------

    protected override void OnMouseDown(MouseEventArgs e)
    {
        if (e.Button == MouseButtons.Right) { DialogResult = DialogResult.Cancel; return; }
        if (e.Button != MouseButtons.Left) return;
        _dragging = true;
        _start = e.Location;
        _selection = Rectangle.Empty;
        RefreshDirty();
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        _cursor = e.Location;
        if (_dragging)
            _selection = Rectangle.FromLTRB(Math.Min(_start.X, e.X), Math.Min(_start.Y, e.Y),
                                            Math.Max(_start.X, e.X) + 1, Math.Max(_start.Y, e.Y) + 1);
        else
            UpdateHover();
        RefreshDirty();
    }

    protected override void OnMouseUp(MouseEventArgs e)
    {
        if (e.Button != MouseButtons.Left || !_dragging) return;
        _dragging = false;
        var r = _selection.Width > 4 && _selection.Height > 4 ? _selection : _hover;
        if (r.Width < 2 || r.Height < 2) { UpdateHover(); RefreshDirty(); return; }
        Finish(r);
    }

    protected override void OnKeyDown(KeyEventArgs e)
    {
        switch (e.KeyCode)
        {
            case Keys.Escape:
                DialogResult = DialogResult.Cancel;
                break;
            case Keys.Enter when !_hover.IsEmpty:
                Finish(_hover);
                break;
            case Keys.ControlKey:
                UpdateHover();
                RefreshDirty();
                break;
            case Keys.Left: case Keys.Right: case Keys.Up: case Keys.Down:
                int step = e.Shift ? 10 : 1;
                var p = Cursor.Position;
                p.Offset(e.KeyCode == Keys.Left ? -step : e.KeyCode == Keys.Right ? step : 0,
                         e.KeyCode == Keys.Up ? -step : e.KeyCode == Keys.Down ? step : 0);
                Cursor.Position = p;   // löst MouseMove aus
                break;
        }
    }

    protected override void OnKeyUp(KeyEventArgs e)
    {
        if (e.KeyCode == Keys.ControlKey) { UpdateHover(); RefreshDirty(); }
    }

    void Finish(Rectangle r)
    {
        r.Intersect(new Rectangle(0, 0, _shot.Width, _shot.Height));
        if (r.Width < 1 || r.Height < 1) return;
        Result = ScreenCapture.Copy(_shot, r);
        SelectedRect = new Rectangle(r.X + _virtual.X, r.Y + _virtual.Y, r.Width, r.Height);
        DialogResult = DialogResult.OK;
    }

    /// <summary>Fenster unter dem Cursor; mit Strg das kleinste Unterelement.</summary>
    void UpdateHover()
    {
        _hover = Rectangle.Empty;
        var p = new Point(_cursor.X + _virtual.X, _cursor.Y + _virtual.Y);
        foreach (var w in _windows)
        {
            if (!w.Bounds.Contains(p)) continue;
            var r = w.Bounds;
            bool ctrl = (ModifierKeys & Keys.Control) != 0;
            if (ctrl != _preferInner)
            {
                foreach (var c in w.Children)
                    if (c.Contains(p) && (long)c.Width * c.Height < (long)r.Width * r.Height) r = c;
            }
            r.Offset(-_virtual.X, -_virtual.Y);
            r.Intersect(ClientRectangle);
            _hover = r;
            return;
        }
    }

    // ---------- Zeichnen ----------

    void RefreshDirty()
    {
        var now = DirtyRects();
        foreach (var r in _lastDirty) Invalidate(r);
        foreach (var r in now) Invalidate(r);
        _lastDirty = now;
    }

    List<Rectangle> DirtyRects()
    {
        var list = new List<Rectangle>();
        var a = ActiveRect;
        if (!a.IsEmpty)
        {
            a.Inflate(3, 3);
            list.Add(a);
            list.Add(LabelRect(ActiveRect));
        }
        if (!_dragging)
        {
            list.Add(new Rectangle(0, _cursor.Y - 1, ClientSize.Width, 3));
            list.Add(new Rectangle(_cursor.X - 1, 0, 3, ClientSize.Height));
        }
        var m = MagnifierRect();
        m.Inflate(4, 4);
        m.Height += 28;
        list.Add(m);
        return list;
    }

    protected override void OnPaintBackground(PaintEventArgs e) { }

    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        var clip = e.ClipRectangle;
        g.DrawImage(_dim, clip, clip, GraphicsUnit.Pixel);

        DrawHint(g);

        var r = ActiveRect;
        if (r.Width > 0 && r.Height > 0)
        {
            var c = Rectangle.Intersect(r, clip);
            if (!c.IsEmpty) g.DrawImage(_shot, c, c, GraphicsUnit.Pixel);
            using var pen = new Pen(Accent, 2);
            g.DrawRectangle(pen, r.X, r.Y, r.Width - 1, r.Height - 1);
            DrawLabel(g, r, $"{r.Width} × {r.Height}");
        }

        if (!_dragging)
        {
            using var pen = new Pen(Color.FromArgb(170, Accent), 1);
            g.DrawLine(pen, 0, _cursor.Y, ClientSize.Width, _cursor.Y);
            g.DrawLine(pen, _cursor.X, 0, _cursor.X, ClientSize.Height);
        }

        DrawMagnifier(g);
    }

    Rectangle LabelRect(Rectangle r)
    {
        const int w = 130, h = 24;
        int y = r.Top - h - 4;
        if (y < 0) y = r.Top + 4;
        return new Rectangle(r.Left, y, w, h);
    }

    void DrawLabel(Graphics g, Rectangle r, string text)
    {
        var lr = LabelRect(r);
        var size = TextRenderer.MeasureText(text, _font);
        var box = new Rectangle(lr.X, lr.Y, Math.Min(lr.Width, size.Width + 10), lr.Height);
        using var bg = new SolidBrush(Color.FromArgb(220, 20, 20, 20));
        g.FillRectangle(bg, box);
        TextRenderer.DrawText(g, text, _font, box, Color.White,
            TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);
    }

    Rectangle MagnifierRect()
    {
        int x = _cursor.X + 24, y = _cursor.Y + 24;
        var screen = Screen.FromPoint(new Point(_cursor.X + _virtual.X, _cursor.Y + _virtual.Y)).Bounds;
        screen.Offset(-_virtual.X, -_virtual.Y);
        if (x + MagSize > screen.Right) x = _cursor.X - 24 - MagSize;
        if (y + MagSize + 28 > screen.Bottom) y = _cursor.Y - 24 - MagSize - 28;
        return new Rectangle(x, y, MagSize, MagSize);
    }

    void DrawMagnifier(Graphics g)
    {
        var dst = MagnifierRect();
        var src = new Rectangle(_cursor.X - MagPixels / 2, _cursor.Y - MagPixels / 2, MagPixels, MagPixels);

        g.FillRectangle(Brushes.Black, dst);
        var state = g.Save();
        g.InterpolationMode = InterpolationMode.NearestNeighbor;
        g.PixelOffsetMode = PixelOffsetMode.Half;
        g.DrawImage(_shot, dst, src, GraphicsUnit.Pixel);
        g.Restore(state);

        var center = new Rectangle(dst.X + MagPixels / 2 * MagZoom, dst.Y + MagPixels / 2 * MagZoom, MagZoom, MagZoom);
        using (var pen = new Pen(Color.FromArgb(200, 255, 255, 255), 1))
            g.DrawRectangle(pen, center);
        using (var border = new Pen(Accent, 2))
            g.DrawRectangle(border, dst);

        var sp = new Point(_cursor.X + _virtual.X, _cursor.Y + _virtual.Y);
        Color px = Color.Empty;
        if (_cursor.X >= 0 && _cursor.Y >= 0 && _cursor.X < _shot.Width && _cursor.Y < _shot.Height)
            px = _shot.GetPixel(_cursor.X, _cursor.Y);
        string info = px.IsEmpty ? $"{sp.X}, {sp.Y}" : $"{sp.X}, {sp.Y}   #{px.R:X2}{px.G:X2}{px.B:X2}";
        var box = new Rectangle(dst.X, dst.Bottom + 2, dst.Width, 24);
        using var bg = new SolidBrush(Color.FromArgb(220, 20, 20, 20));
        g.FillRectangle(bg, box);
        TextRenderer.DrawText(g, info, _font, box, Color.White,
            TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);
    }

    void DrawHint(Graphics g)
    {
        var primary = Screen.PrimaryScreen!.Bounds;
        primary.Offset(-_virtual.X, -_virtual.Y);
        var size = TextRenderer.MeasureText(_hint, _hintFont);
        var box = new Rectangle(primary.X + (primary.Width - size.Width - 24) / 2, primary.Y + 16, size.Width + 24, size.Height + 12);
        using var bg = new SolidBrush(Color.FromArgb(200, 20, 20, 20));
        g.FillRectangle(bg, box);
        TextRenderer.DrawText(g, _hint, _hintFont, box, Color.White,
            TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing)
        {
            _dim.Dispose();
            _font.Dispose();
            _hintFont.Dispose();
        }
        base.Dispose(disposing);
    }
}
