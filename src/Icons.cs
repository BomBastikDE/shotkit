using System.Drawing.Drawing2D;

namespace ShotKit;

/// <summary>Zur Laufzeit gezeichnete Symbole – keine externen Ressourcen nötig.</summary>
static class Icons
{
    static Icon _app;

    public static Icon AppIcon()
    {
        if (_app != null) return _app;
        using var bmp = new Bitmap(64, 64);
        using (var g = Graphics.FromImage(bmp))
        {
            g.SmoothingMode = SmoothingMode.AntiAlias;
            using var path = RoundedRect(new RectangleF(2, 2, 60, 60), 14);
            using var fill = new LinearGradientBrush(new Rectangle(0, 0, 64, 64),
                Color.FromArgb(255, 92, 72), Color.FromArgb(214, 36, 58), 90f);
            g.FillPath(fill, path);
            using var pen = new Pen(Color.White, 6) { StartCap = LineCap.Round, EndCap = LineCap.Round, LineJoin = LineJoin.Round };
            g.DrawLines(pen, new PointF[] { new(14, 26), new(14, 14), new(26, 14) });
            g.DrawLines(pen, new PointF[] { new(38, 14), new(50, 14), new(50, 26) });
            g.DrawLines(pen, new PointF[] { new(50, 38), new(50, 50), new(38, 50) });
            g.DrawLines(pen, new PointF[] { new(26, 50), new(14, 50), new(14, 38) });
            g.FillEllipse(Brushes.White, 25, 25, 14, 14);
        }
        IntPtr h = bmp.GetHicon();
        _app = (Icon)Icon.FromHandle(h).Clone();
        Native.DestroyIcon(h);
        return _app;
    }

    public static GraphicsPath RoundedRect(RectangleF r, float radius)
    {
        var p = new GraphicsPath();
        float d = radius * 2;
        p.AddArc(r.X, r.Y, d, d, 180, 90);
        p.AddArc(r.Right - d, r.Y, d, d, 270, 90);
        p.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
        p.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
        p.CloseFigure();
        return p;
    }

    static Graphics Begin(Bitmap bmp, int px)
    {
        var g = Graphics.FromImage(bmp);
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = System.Drawing.Text.TextRenderingHint.AntiAlias;
        g.ScaleTransform(px / 20f, px / 20f);
        return g;
    }

    public static Bitmap ForTool(Tool tool, int px)
    {
        var bmp = new Bitmap(px, px);
        using var g = Begin(bmp, px);
        var ink = Color.FromArgb(50, 50, 50);
        using var pen = new Pen(ink, 1.8f) { StartCap = LineCap.Round, EndCap = LineCap.Round, LineJoin = LineJoin.Round };
        using var brush = new SolidBrush(ink);
        using var center = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center };

        switch (tool)
        {
            case Tool.Select:
                g.FillPolygon(brush, new PointF[] { new(5, 2), new(5, 16.5f), new(8.6f, 13), new(11, 18.5f), new(13.2f, 17.5f), new(10.8f, 12), new(15.5f, 12) });
                break;
            case Tool.Arrow:
                g.DrawLine(pen, 3.5f, 16.5f, 12, 8);
                g.FillPolygon(brush, new PointF[] { new(17, 3), new(8.5f, 6), new(14, 11.5f) });
                break;
            case Tool.Line:
                g.DrawLine(pen, 4, 16, 16, 4);
                break;
            case Tool.Rectangle:
                g.DrawRectangle(pen, 3, 5, 14, 10);
                break;
            case Tool.Ellipse:
                g.DrawEllipse(pen, 2.5f, 4, 15, 12);
                break;
            case Tool.Text:
                using (var f = new Font(Fonts.UI, 16, FontStyle.Bold, GraphicsUnit.Pixel))
                    g.DrawString("T", f, brush, new RectangleF(0, 0, 20, 20), center);
                break;
            case Tool.Highlight:
                using (var y = new SolidBrush(Color.FromArgb(220, 255, 214, 0)))
                    g.FillRectangle(y, 2, 11, 16, 6);
                g.FillPolygon(brush, new PointF[] { new(7, 10), new(13, 3), new(16, 6), new(10, 12) });
                break;
            case Tool.Blur:
                for (int x = 0; x < 3; x++)
                    for (int yy = 0; yy < 3; yy++)
                    {
                        int v = (x + yy) % 2 == 0 ? 70 : 170;
                        using var b = new SolidBrush(Color.FromArgb(v, v, v));
                        g.FillRectangle(b, 2.5f + x * 5, 2.5f + yy * 5, 5, 5);
                    }
                break;
            case Tool.Step:
                using (var red = new SolidBrush(Color.FromArgb(230, 40, 40)))
                    g.FillEllipse(red, 2, 2, 16, 16);
                using (var f = new Font(Fonts.UI, 11, FontStyle.Bold, GraphicsUnit.Pixel))
                    g.DrawString("1", f, Brushes.White, new RectangleF(0, 0.5f, 20, 20), center);
                break;
            case Tool.Pen:
                g.DrawBezier(pen, 3, 15, 7, 1, 11, 21, 17, 5);
                break;
            case Tool.Crop:
                g.DrawLines(pen, new PointF[] { new(6, 2), new(6, 14), new(18, 14) });
                g.DrawLines(pen, new PointF[] { new(2, 6), new(14, 6), new(14, 18) });
                break;
        }
        return bmp;
    }

    public static Bitmap UndoRedo(bool redo, int px)
    {
        var bmp = new Bitmap(px, px);
        using var g = Begin(bmp, px);
        if (redo)
        {
            g.TranslateTransform(20, 0);
            g.ScaleTransform(-1, 1);
        }
        using var pen = new Pen(Color.FromArgb(50, 50, 50), 1.8f) { StartCap = LineCap.Round, EndCap = LineCap.Round };
        g.DrawBezier(pen, 16, 16, 17, 6, 10, 5, 5, 8);
        using var brush = new SolidBrush(Color.FromArgb(50, 50, 50));
        g.FillPolygon(brush, new PointF[] { new(2, 9), new(8, 4.5f), new(8.5f, 11.5f) });
        return bmp;
    }

    public static Bitmap Swatch(Color c, int px, bool rounded = true)
    {
        var bmp = new Bitmap(px, px);
        using var g = Begin(bmp, px);
        using var path = RoundedRect(new RectangleF(2, 2, 16, 16), rounded ? 3.5f : 0.5f);
        using var b = new SolidBrush(c);
        g.FillPath(b, path);
        using var pen = new Pen(Color.FromArgb(120, 0, 0, 0), 1f);
        g.DrawPath(pen, path);
        return bmp;
    }

    /// <summary>Symbole der Aktionsleiste: "capture", "copy", "save".</summary>
    public static Bitmap Action(string kind, int px)
    {
        var bmp = new Bitmap(px, px);
        using var g = Begin(bmp, px);
        var ink = Color.FromArgb(50, 50, 50);
        using var pen = new Pen(ink, 1.6f) { StartCap = LineCap.Round, EndCap = LineCap.Round, LineJoin = LineJoin.Round };
        switch (kind)
        {
            case "capture":
                g.DrawLines(pen, new PointF[] { new(3, 7), new(3, 3), new(7, 3) });
                g.DrawLines(pen, new PointF[] { new(13, 3), new(17, 3), new(17, 7) });
                g.DrawLines(pen, new PointF[] { new(17, 13), new(17, 17), new(13, 17) });
                g.DrawLines(pen, new PointF[] { new(7, 17), new(3, 17), new(3, 13) });
                g.DrawLine(pen, 10, 7, 10, 13);
                g.DrawLine(pen, 7, 10, 13, 10);
                break;
            case "copy":
                using (var back = RoundedRect(new RectangleF(3, 3, 9, 9), 1.5f)) g.DrawPath(pen, back);
                using (var front = RoundedRect(new RectangleF(8, 8, 9, 9), 1.5f)) g.DrawPath(pen, front);
                break;
            default:
                g.DrawLines(pen, new PointF[] { new(4, 3), new(13, 3), new(16, 6), new(16, 17), new(4, 17), new(4, 3) });
                g.DrawRectangle(pen, 7, 3, 6, 4);
                g.DrawRectangle(pen, 7, 12, 6, 5);
                break;
        }
        return bmp;
    }
}
