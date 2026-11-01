using System.Drawing.Drawing2D;

namespace ShotKit;

enum Tool { Select, Arrow, Line, Rectangle, Ellipse, Text, Highlight, Blur, Step, Pen, Crop }

static class Fonts
{
    public static readonly FontFamily UI = Create();

    static FontFamily Create()
    {
        try { return new FontFamily("Segoe UI"); }
        catch (ArgumentException) { return FontFamily.GenericSansSerif; }
    }
}

static class Geometry
{
    public static float Distance(PointF a, PointF b) => MathF.Sqrt((a.X - b.X) * (a.X - b.X) + (a.Y - b.Y) * (a.Y - b.Y));

    public static float DistanceToSegment(PointF p, PointF a, PointF b)
    {
        float dx = b.X - a.X, dy = b.Y - a.Y;
        float len2 = dx * dx + dy * dy;
        if (len2 < 0.0001f) return Distance(p, a);
        float t = Math.Clamp(((p.X - a.X) * dx + (p.Y - a.Y) * dy) / len2, 0, 1);
        return Distance(p, new PointF(a.X + t * dx, a.Y + t * dy));
    }

    public static RectangleF Normalize(PointF a, PointF b) =>
        RectangleF.FromLTRB(Math.Min(a.X, b.X), Math.Min(a.Y, b.Y), Math.Max(a.X, b.X), Math.Max(a.Y, b.Y));
}

abstract class Shape
{
    public Color Color = Color.Red;
    public float Width = 4f;
    public float FontSize = 28f;

    public abstract void Draw(Graphics g, Bitmap source);
    public abstract RectangleF Bounds { get; }
    public abstract void Offset(float dx, float dy);

    public virtual bool HitTest(PointF p, float tol)
    {
        var b = Bounds;
        b.Inflate(tol, tol);
        return b.Contains(p);
    }

    public virtual PointF[] Handles => Array.Empty<PointF>();
    public virtual void SetHandle(int index, PointF p) { }
    public virtual bool IsEmpty => false;
    public virtual Shape Clone() => (Shape)MemberwiseClone();

    protected static Pen MakePen(Color c, float w) =>
        new(c, w) { StartCap = LineCap.Round, EndCap = LineCap.Round, LineJoin = LineJoin.Round };

    protected bool OutlineHit(GraphicsPath path, PointF p, float tol)
    {
        using var pen = new Pen(Color.Black, Width + 2 * tol);
        return path.IsOutlineVisible(p, pen);
    }

    protected static Color Contrast(Color c) =>
        (c.R * 0.299 + c.G * 0.587 + c.B * 0.114) > 150 ? Color.FromArgb(40, 40, 40) : Color.White;
}

abstract class TwoPointShape : Shape
{
    public PointF P1, P2;
    public RectangleF Rect => Geometry.Normalize(P1, P2);
    public override RectangleF Bounds => Rect;
    public override bool IsEmpty => Math.Abs(P1.X - P2.X) < 3 && Math.Abs(P1.Y - P2.Y) < 3;

    public override void Offset(float dx, float dy)
    {
        P1 = new PointF(P1.X + dx, P1.Y + dy);
        P2 = new PointF(P2.X + dx, P2.Y + dy);
    }
}

class LineShape : TwoPointShape
{
    public override void Draw(Graphics g, Bitmap source)
    {
        using var pen = MakePen(Color, Width);
        g.DrawLine(pen, P1, P2);
    }

    public override bool HitTest(PointF p, float tol) => Geometry.DistanceToSegment(p, P1, P2) <= tol + Width / 2;
    public override PointF[] Handles => new[] { P1, P2 };
    public override void SetHandle(int i, PointF p) { if (i == 0) P1 = p; else P2 = p; }
}

sealed class ArrowShape : LineShape
{
    public override void Draw(Graphics g, Bitmap source)
    {
        float dx = P2.X - P1.X, dy = P2.Y - P1.Y;
        float len = MathF.Sqrt(dx * dx + dy * dy);
        if (len < 1) return;
        float ux = dx / len, uy = dy / len;
        float head = Math.Min(len, Math.Max(12f, Width * 3.2f));
        float half = head * 0.55f;

        var basePt = new PointF(P2.X - ux * head, P2.Y - uy * head);
        var left = new PointF(basePt.X - uy * half, basePt.Y + ux * half);
        var right = new PointF(basePt.X + uy * half, basePt.Y - ux * half);

        if (len > head)
        {
            using var pen = MakePen(Color, Width);
            pen.EndCap = LineCap.Flat;
            g.DrawLine(pen, P1, new PointF(P2.X - ux * head * 0.8f, P2.Y - uy * head * 0.8f));
        }
        var tri = new[] { P2, left, right };
        using var brush = new SolidBrush(Color);
        using var edge = new Pen(Color, 1.5f) { LineJoin = LineJoin.Round };
        g.FillPolygon(brush, tri);
        g.DrawPolygon(edge, tri);
    }
}

abstract class BoxShape : TwoPointShape
{
    public override PointF[] Handles
    {
        get
        {
            var r = Rect;
            return new[] { new PointF(r.Left, r.Top), new PointF(r.Right, r.Top),
                           new PointF(r.Right, r.Bottom), new PointF(r.Left, r.Bottom) };
        }
    }

    public override void SetHandle(int i, PointF p)
    {
        var r = Rect;
        float l = r.Left, t = r.Top, ri = r.Right, b = r.Bottom;
        switch (i)
        {
            case 0: l = p.X; t = p.Y; break;
            case 1: ri = p.X; t = p.Y; break;
            case 2: ri = p.X; b = p.Y; break;
            case 3: l = p.X; b = p.Y; break;
        }
        P1 = new PointF(l, t);
        P2 = new PointF(ri, b);
    }
}

sealed class RectShape : BoxShape
{
    public override void Draw(Graphics g, Bitmap source)
    {
        var r = Rect;
        using var pen = MakePen(Color, Width);
        g.DrawRectangle(pen, r.X, r.Y, r.Width, r.Height);
    }

    public override bool HitTest(PointF p, float tol)
    {
        using var path = new GraphicsPath();
        path.AddRectangle(Rect);
        return OutlineHit(path, p, tol);
    }
}

sealed class EllipseShape : BoxShape
{
    public override void Draw(Graphics g, Bitmap source)
    {
        using var pen = MakePen(Color, Width);
        g.DrawEllipse(pen, Rect);
    }

    public override bool HitTest(PointF p, float tol)
    {
        using var path = new GraphicsPath();
        path.AddEllipse(Rect);
        return OutlineHit(path, p, tol);
    }
}

sealed class HighlightShape : BoxShape
{
    public override void Draw(Graphics g, Bitmap source)
    {
        using var brush = new SolidBrush(Color.FromArgb(105, Color));
        g.FillRectangle(brush, Rect);
    }
}

sealed class BlurShape : BoxShape
{
    public override void Draw(Graphics g, Bitmap source)
    {
        var r = Rectangle.Round(Rect);
        r.Intersect(new Rectangle(0, 0, source.Width, source.Height));
        if (r.Width < 1 || r.Height < 1) return;

        int block = Math.Max(6, (int)(Width * 3));
        int sw = Math.Max(1, r.Width / block), sh = Math.Max(1, r.Height / block);
        using var small = new Bitmap(sw, sh);
        using (var sg = Graphics.FromImage(small))
        {
            sg.InterpolationMode = InterpolationMode.HighQualityBilinear;
            sg.DrawImage(source, new Rectangle(0, 0, sw, sh), r, GraphicsUnit.Pixel);
        }
        var state = g.Save();
        g.InterpolationMode = InterpolationMode.NearestNeighbor;
        g.PixelOffsetMode = PixelOffsetMode.Half;
        g.DrawImage(small, r, new Rectangle(0, 0, sw, sh), GraphicsUnit.Pixel);
        g.Restore(state);
    }
}

sealed class FreehandShape : Shape
{
    public List<PointF> Points = new();

    public override bool IsEmpty => Points.Count < 2;

    public override RectangleF Bounds
    {
        get
        {
            if (Points.Count == 0) return RectangleF.Empty;
            float l = Points.Min(p => p.X), t = Points.Min(p => p.Y), r = Points.Max(p => p.X), b = Points.Max(p => p.Y);
            return RectangleF.FromLTRB(l, t, r, b);
        }
    }

    public override void Draw(Graphics g, Bitmap source)
    {
        if (Points.Count < 2) return;
        using var pen = MakePen(Color, Width);
        g.DrawLines(pen, Points.ToArray());
    }

    public override bool HitTest(PointF p, float tol)
    {
        if (Points.Count < 2) return false;
        using var path = new GraphicsPath();
        path.AddLines(Points.ToArray());
        return OutlineHit(path, p, tol);
    }

    public override void Offset(float dx, float dy)
    {
        for (int i = 0; i < Points.Count; i++) Points[i] = new PointF(Points[i].X + dx, Points[i].Y + dy);
    }

    public override Shape Clone()
    {
        var c = (FreehandShape)base.Clone();
        c.Points = new List<PointF>(Points);
        return c;
    }
}

sealed class TextShape : Shape
{
    public PointF Location;
    public string Text = "";

    GraphicsPath BuildPath()
    {
        var path = new GraphicsPath();
        path.AddString(Text, Fonts.UI, (int)FontStyle.Bold, FontSize, Location, StringFormat.GenericTypographic);
        return path;
    }

    public override RectangleF Bounds
    {
        get
        {
            using var path = BuildPath();
            var b = path.GetBounds();
            return b.IsEmpty ? new RectangleF(Location, new SizeF(10, 10)) : b;
        }
    }

    public override void Draw(Graphics g, Bitmap source)
    {
        using var path = BuildPath();
        using var outline = new Pen(Contrast(Color), Math.Max(3f, FontSize / 7f)) { LineJoin = LineJoin.Round };
        using var brush = new SolidBrush(Color);
        g.DrawPath(outline, path);
        g.FillPath(brush, path);
    }

    public override void Offset(float dx, float dy) => Location = new PointF(Location.X + dx, Location.Y + dy);
}

sealed class StepShape : Shape
{
    public PointF Center;
    public int Number;

    float Radius => 10f + Width * 2.2f;

    public override RectangleF Bounds =>
        new(Center.X - Radius, Center.Y - Radius, Radius * 2, Radius * 2);

    public override void Draw(Graphics g, Bitmap source)
    {
        var r = Bounds;
        using var brush = new SolidBrush(Color);
        using var ring = new Pen(Color.White, Math.Max(2f, Radius / 8f));
        g.FillEllipse(brush, r);
        g.DrawEllipse(ring, r);

        string text = Number.ToString();
        float size = Radius * (text.Length > 1 ? 0.85f : 1.05f);
        using var font = new Font(Fonts.UI, size, FontStyle.Bold, GraphicsUnit.Pixel);
        using var sf = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center };
        using var textBrush = new SolidBrush(Contrast(Color));
        g.DrawString(text, font, textBrush, r, sf);
    }

    public override void Offset(float dx, float dy) => Center = new PointF(Center.X + dx, Center.Y + dy);
}

static class Effects
{
    /// <summary>Rahmen und/oder Schlagschatten um das Bild; offset = neue Position des Originalbilds.</summary>
    public static Bitmap Apply(Bitmap src, int border, Color borderColor, bool shadow, out Point offset)
    {
        const int Blur = 10;
        int m = shadow ? Blur + 8 : 0;
        int w = src.Width + 2 * border, h = src.Height + 2 * border;
        var result = new Bitmap(w + 2 * m, h + 2 * m, System.Drawing.Imaging.PixelFormat.Format32bppArgb);
        using var g = Graphics.FromImage(result);
        g.Clear(Color.Transparent);

        var content = new Rectangle(m, shadow ? m - 4 : 0, w, h);
        if (shadow)
        {
            var s = content;
            s.Offset(0, 5);
            using var layer = new SolidBrush(Color.FromArgb(4, 0, 0, 0));
            for (int k = Blur; k >= -Blur; k--)
            {
                var r = s;
                r.Inflate(k, k);
                g.FillRectangle(layer, r);
            }
        }
        if (border > 0)
        {
            using var b = new SolidBrush(borderColor);
            g.FillRectangle(b, content);
        }
        offset = new Point(content.X + border, content.Y + border);
        g.DrawImage(src, new Rectangle(offset, src.Size));
        return result;
    }
}
