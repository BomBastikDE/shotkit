using System.Drawing.Drawing2D;
using System.Drawing.Imaging;

namespace ShotKit;

/// <summary>Zeichenfläche des Editors: Bild + bearbeitbare Objekte, Undo/Redo, Zoom.</summary>
sealed class EditorCanvas : Control
{
    sealed record Snapshot(Bitmap Image, List<Shape> Shapes);
    enum Drag { None, Create, Move, Handle, Crop }

    readonly Stack<Snapshot> _undo = new(), _redo = new();
    Tool _tool = Tool.Arrow;
    Drag _drag;
    Shape _creating;
    PointF _dragStart, _lastPoint;
    int _handle;
    bool _undoPushed;
    RectangleF _crop;

    public Bitmap Image { get; private set; }
    public List<Shape> Shapes { get; private set; } = new();
    public Shape Selected { get; private set; }
    public float Zoom { get; private set; } = 1f;
    public bool Dirty { get; set; }
    public bool CanUndo => _undo.Count > 0;
    public bool CanRedo => _redo.Count > 0;

    public Color CurrentColor = Color.FromArgb(230, 40, 40);
    public float CurrentWidth = 4f;
    public float CurrentFontSize = 28f;

    /// <summary>Undo-Stand, Dirty-Flag, Werkzeug oder Bildgröße haben sich geändert.</summary>
    public event EventHandler StateChanged;

    public EditorCanvas(Bitmap image)
    {
        Image = image;
        SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer |
                 ControlStyles.UserPaint | ControlStyles.Selectable, true);
        Cursor = Cursors.Cross;
        UpdateSize();
    }

    public Tool Tool
    {
        get => _tool;
        set
        {
            _tool = value;
            _crop = RectangleF.Empty;
            if (value != Tool.Select) Selected = null;
            Cursor = value == Tool.Select ? Cursors.Default : Cursors.Cross;
            Invalidate();
            OnStateChanged();
        }
    }

    void OnStateChanged() => StateChanged?.Invoke(this, EventArgs.Empty);

    // ---------- Zoom ----------

    public void SetZoom(float zoom)
    {
        float max = Math.Min(8f, 32000f / Math.Max(Image.Width, Image.Height));
        Zoom = Math.Clamp(zoom, Math.Min(0.02f, max), max);
        UpdateSize();
        Invalidate();
        OnStateChanged();
    }

    void UpdateSize() =>
        Size = new Size((int)Math.Ceiling(Image.Width * Zoom), (int)Math.Ceiling(Image.Height * Zoom));

    PointF ToImage(Point p) => new(p.X / Zoom, p.Y / Zoom);

    // ---------- Undo/Redo ----------

    Snapshot Current() => new(Image, Shapes.Select(s => s.Clone()).ToList());

    public void PushUndo()
    {
        _undo.Push(Current());
        _redo.Clear();
        Dirty = true;
        OnStateChanged();
    }

    public void Undo()
    {
        if (_undo.Count == 0) return;
        _redo.Push(Current());
        Restore(_undo.Pop());
    }

    public void Redo()
    {
        if (_redo.Count == 0) return;
        _undo.Push(Current());
        Restore(_redo.Pop());
    }

    void Restore(Snapshot s)
    {
        Image = s.Image;
        Shapes = s.Shapes;
        Selected = null;
        Dirty = true;
        UpdateSize();
        Invalidate();
        OnStateChanged();
    }

    // ---------- Operationen ----------

    public void SelectShape(Shape s)
    {
        Selected = s;
        Invalidate();
    }

    public void DeleteSelected()
    {
        if (Selected == null) return;
        PushUndo();
        Shapes.Remove(Selected);
        Selected = null;
        Invalidate();
    }

    public void NudgeSelected(float dx, float dy)
    {
        if (Selected == null) return;
        PushUndo();
        Selected.Offset(dx, dy);
        Invalidate();
    }

    public void ApplyToSelected(Action<Shape> change)
    {
        if (Selected == null) return;
        PushUndo();
        change(Selected);
        Invalidate();
    }

    public void ApplyCrop(Rectangle r)
    {
        PushUndo();
        var cropped = ScreenCapture.Copy(Image, r);
        foreach (var s in Shapes) s.Offset(-r.X, -r.Y);
        Image = cropped;
        Selected = null;
        UpdateSize();
        Invalidate();
        OnStateChanged();
    }

    public void ApplyEffect(int border, bool shadow)
    {
        PushUndo();
        var result = Effects.Apply(Image, border, CurrentColor, shadow, out var offset);
        foreach (var s in Shapes) s.Offset(offset.X, offset.Y);
        Image = result;
        UpdateSize();
        Invalidate();
        OnStateChanged();
    }

    public void ReplaceImage(Bitmap image)
    {
        PushUndo();
        Image = image;
        Shapes = new List<Shape>();
        Selected = null;
        UpdateSize();
        Invalidate();
        OnStateChanged();
    }

    /// <summary>Bild inkl. aller Objekte zu einer Bitmap zusammenführen.</summary>
    public Bitmap Flatten()
    {
        var bmp = new Bitmap(Image.Width, Image.Height, PixelFormat.Format32bppArgb);
        using var g = Graphics.FromImage(bmp);
        g.DrawImage(Image, new Rectangle(0, 0, Image.Width, Image.Height));
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = System.Drawing.Text.TextRenderingHint.AntiAlias;
        foreach (var s in Shapes) s.Draw(g, Image);
        return bmp;
    }

    // ---------- Maus ----------

    Shape HitShape(PointF p)
    {
        float tol = 5 / Zoom;
        for (int i = Shapes.Count - 1; i >= 0; i--)
            if (Shapes[i].HitTest(p, tol)) return Shapes[i];
        return null;
    }

    int HitHandle(PointF p)
    {
        if (Selected == null) return -1;
        var hs = Selected.Handles;
        for (int i = 0; i < hs.Length; i++)
            if (Geometry.Distance(hs[i], p) <= 8 / Zoom) return i;
        return -1;
    }

    protected override void OnMouseDown(MouseEventArgs e)
    {
        Focus();
        if (e.Button != MouseButtons.Left) return;
        var p = ToImage(e.Location);
        _dragStart = _lastPoint = p;
        _undoPushed = false;

        int h = HitHandle(p);
        if (h >= 0) { _drag = Drag.Handle; _handle = h; return; }

        switch (_tool)
        {
            case Tool.Select:
                SelectShape(HitShape(p));
                if (Selected != null) _drag = Drag.Move;
                break;
            case Tool.Text:
                if (HitShape(p) is TextShape t) EditText(t);
                else CreateText(p);
                break;
            case Tool.Step:
                PushUndo();
                int next = Shapes.OfType<StepShape>().Select(s => s.Number).DefaultIfEmpty(0).Max() + 1;
                Shapes.Add(new StepShape { Center = p, Number = next, Color = CurrentColor, Width = CurrentWidth });
                Invalidate();
                break;
            case Tool.Crop:
                _drag = Drag.Crop;
                _crop = RectangleF.Empty;
                break;
            default:
                _creating = NewShape(p);
                _drag = Drag.Create;
                break;
        }
    }

    Shape NewShape(PointF p)
    {
        Shape s = _tool switch
        {
            Tool.Arrow => new ArrowShape(),
            Tool.Line => new LineShape(),
            Tool.Rectangle => new RectShape(),
            Tool.Ellipse => new EllipseShape(),
            Tool.Highlight => new HighlightShape(),
            Tool.Blur => new BlurShape(),
            Tool.Pen => new FreehandShape { Points = { p } },
            _ => throw new InvalidOperationException()
        };
        if (s is TwoPointShape tp) tp.P1 = tp.P2 = p;
        s.Color = CurrentColor;
        s.Width = CurrentWidth;
        s.FontSize = CurrentFontSize;
        return s;
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        var p = ToImage(e.Location);
        bool shift = (ModifierKeys & Keys.Shift) != 0;

        switch (_drag)
        {
            case Drag.Create:
                if (_creating is TwoPointShape tp) tp.P2 = shift ? Constrain(tp, p) : p;
                else if (_creating is FreehandShape f) f.Points.Add(p);
                Invalidate();
                break;
            case Drag.Move:
                if (!_undoPushed) { PushUndo(); _undoPushed = true; }
                Selected.Offset(p.X - _lastPoint.X, p.Y - _lastPoint.Y);
                _lastPoint = p;
                Invalidate();
                break;
            case Drag.Handle:
                if (!_undoPushed) { PushUndo(); _undoPushed = true; }
                Selected.SetHandle(_handle, p);
                Invalidate();
                break;
            case Drag.Crop:
                var r = Geometry.Normalize(_dragStart, p);
                r.Intersect(new RectangleF(0, 0, Image.Width, Image.Height));
                _crop = r;
                Invalidate();
                break;
            default:
                if (HitHandle(p) >= 0) Cursor = Cursors.SizeAll;
                else if (_tool == Tool.Select) Cursor = HitShape(p) != null ? Cursors.SizeAll : Cursors.Default;
                else Cursor = Cursors.Cross;
                break;
        }
    }

    static PointF Constrain(TwoPointShape s, PointF p)
    {
        float dx = p.X - s.P1.X, dy = p.Y - s.P1.Y;
        if (s is BoxShape)
        {
            float d = Math.Max(Math.Abs(dx), Math.Abs(dy));
            return new PointF(s.P1.X + Math.Sign(dx) * d, s.P1.Y + Math.Sign(dy) * d);
        }
        double angle = Math.Round(Math.Atan2(dy, dx) / (Math.PI / 4)) * (Math.PI / 4);
        float len = MathF.Sqrt(dx * dx + dy * dy);
        return new PointF(s.P1.X + (float)Math.Cos(angle) * len, s.P1.Y + (float)Math.Sin(angle) * len);
    }

    protected override void OnMouseUp(MouseEventArgs e)
    {
        switch (_drag)
        {
            case Drag.Create:
                if (_creating != null && !_creating.IsEmpty)
                {
                    PushUndo();
                    Shapes.Add(_creating);
                    Selected = _creating;
                }
                _creating = null;
                break;
            case Drag.Crop:
                var r = Rectangle.Round(_crop);
                r.Intersect(new Rectangle(0, 0, Image.Width, Image.Height));
                _crop = RectangleF.Empty;
                _drag = Drag.None;
                if (r.Width > 2 && r.Height > 2)
                {
                    ApplyCrop(r);
                    Tool = Tool.Select;
                }
                break;
        }
        _drag = Drag.None;
        Invalidate();
        OnStateChanged();
    }

    protected override void OnMouseDoubleClick(MouseEventArgs e)
    {
        if (_tool == Tool.Select && HitShape(ToImage(e.Location)) is TextShape t) EditText(t);
    }

    protected override void OnMouseWheel(MouseEventArgs e)
    {
        if ((ModifierKeys & Keys.Control) != 0)
        {
            SetZoom(Zoom * (e.Delta > 0 ? 1.25f : 0.8f));
            if (e is HandledMouseEventArgs h) h.Handled = true;
            return;
        }
        base.OnMouseWheel(e);
    }

    void CreateText(PointF p)
    {
        string text = TextInputDialog.Ask(FindForm(), "Text einfügen", "");
        if (string.IsNullOrWhiteSpace(text)) return;
        PushUndo();
        var t = new TextShape { Location = p, Text = text, Color = CurrentColor, Width = CurrentWidth, FontSize = CurrentFontSize };
        Shapes.Add(t);
        Selected = t;
        Invalidate();
    }

    void EditText(TextShape t)
    {
        string text = TextInputDialog.Ask(FindForm(), "Text bearbeiten", t.Text);
        if (text == null || text == t.Text) return;
        PushUndo();
        if (string.IsNullOrWhiteSpace(text)) { Shapes.Remove(t); Selected = null; }
        else t.Text = text;
        Invalidate();
    }

    // ---------- Zeichnen ----------

    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        using (var checker = new HatchBrush(HatchStyle.LargeCheckerBoard, Color.FromArgb(228, 228, 228), Color.White))
            g.FillRectangle(checker, ClientRectangle);

        var state = g.Save();
        g.ScaleTransform(Zoom, Zoom);
        g.InterpolationMode = Zoom < 1 ? InterpolationMode.HighQualityBicubic : InterpolationMode.NearestNeighbor;
        g.PixelOffsetMode = PixelOffsetMode.Half;
        g.DrawImage(Image, new Rectangle(0, 0, Image.Width, Image.Height));

        g.PixelOffsetMode = PixelOffsetMode.Default;
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = System.Drawing.Text.TextRenderingHint.AntiAlias;
        foreach (var s in Shapes) s.Draw(g, Image);
        _creating?.Draw(g, Image);
        g.Restore(state);

        if (_drag == Drag.Crop && !_crop.IsEmpty) DrawCrop(g);
        if (Selected != null) DrawSelection(g);
    }

    RectangleF ToScreen(RectangleF r) => new(r.X * Zoom, r.Y * Zoom, r.Width * Zoom, r.Height * Zoom);

    void DrawCrop(Graphics g)
    {
        var r = ToScreen(_crop);
        using var region = new Region(ClientRectangle);
        region.Exclude(r);
        using var veil = new SolidBrush(Color.FromArgb(140, 0, 0, 0));
        g.FillRegion(veil, region);
        using var pen = new Pen(Color.White, 1) { DashStyle = DashStyle.Dash };
        g.DrawRectangle(pen, r.X, r.Y, r.Width, r.Height);
        var label = $"{(int)_crop.Width} × {(int)_crop.Height}";
        using var font = new Font(Fonts.UI, 12f, FontStyle.Bold, GraphicsUnit.Pixel);
        g.DrawString(label, font, Brushes.White, r.X + 4, r.Y + 4);
    }

    void DrawSelection(Graphics g)
    {
        var b = ToScreen(Selected.Bounds);
        b.Inflate(4, 4);
        g.SmoothingMode = SmoothingMode.None;
        using (var pen = new Pen(Color.FromArgb(0, 120, 215), 1) { DashStyle = DashStyle.Dash })
            g.DrawRectangle(pen, b.X, b.Y, b.Width, b.Height);

        foreach (var h in Selected.Handles)
        {
            var hr = new RectangleF(h.X * Zoom - 5, h.Y * Zoom - 5, 10, 10);
            g.FillRectangle(Brushes.White, hr);
            using var pen = new Pen(Color.FromArgb(0, 120, 215), 1.5f);
            g.DrawRectangle(pen, hr.X, hr.Y, hr.Width, hr.Height);
        }
    }
}
