using System.Drawing.Imaging;

namespace ShotKit;

sealed record WindowInfo(Rectangle Bounds, List<Rectangle> Children);

static class ScreenCapture
{
    public static Bitmap Capture(Rectangle screenRect)
    {
        var bmp = new Bitmap(screenRect.Width, screenRect.Height, PixelFormat.Format32bppArgb);
        using var g = Graphics.FromImage(bmp);
        g.CopyFromScreen(screenRect.Location, Point.Empty, screenRect.Size, CopyPixelOperation.SourceCopy);
        return bmp;
    }

    public static Bitmap Copy(Bitmap src, Rectangle r)
    {
        var bmp = new Bitmap(r.Width, r.Height, PixelFormat.Format32bppArgb);
        using var g = Graphics.FromImage(bmp);
        g.DrawImage(src, new Rectangle(0, 0, r.Width, r.Height), r, GraphicsUnit.Pixel);
        return bmp;
    }

    /// <summary>Alle sichtbaren Top-Level-Fenster in Z-Reihenfolge (oberstes zuerst), inkl. Unterelementen.</summary>
    public static List<WindowInfo> GetWindows()
    {
        var list = new List<WindowInfo>();
        uint ownPid = (uint)Environment.ProcessId;

        Native.EnumWindows((hWnd, _) =>
        {
            if (!Native.IsWindowVisible(hWnd) || Native.IsIconic(hWnd) || Native.IsCloaked(hWnd))
                return true;
            Native.GetWindowThreadProcessId(hWnd, out uint pid);
            if (pid == ownPid) return true;

            var bounds = Native.GetWindowBounds(hWnd);
            if (bounds.Width < 10 || bounds.Height < 10) return true;

            var children = new List<Rectangle>();
            Native.EnumChildWindows(hWnd, (child, _) =>
            {
                if (Native.IsWindowVisible(child) && Native.GetWindowRect(child, out var cr))
                {
                    var r = cr.ToRectangle();
                    r.Intersect(bounds);
                    if (r.Width > 8 && r.Height > 8) children.Add(r);
                }
                return true;
            }, IntPtr.Zero);

            list.Add(new WindowInfo(bounds, children));
            return true;
        }, IntPtr.Zero);

        return list;
    }
}

static class ImageIO
{
    public static readonly string SaveFilter =
        "PNG-Bild|*.png|JPEG-Bild|*.jpg|BMP-Bild|*.bmp|GIF-Bild|*.gif";
    public static readonly string OpenFilter =
        "Bilder|*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.tif;*.tiff|Alle Dateien|*.*";

    public static void Save(Bitmap bmp, string path)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        switch (Path.GetExtension(path).ToLowerInvariant())
        {
            case ".jpg":
            case ".jpeg":
                using (var flat = OnWhite(bmp))
                {
                    var codec = ImageCodecInfo.GetImageEncoders().First(c => c.FormatID == ImageFormat.Jpeg.Guid);
                    using var ps = new EncoderParameters(1);
                    ps.Param[0] = new EncoderParameter(System.Drawing.Imaging.Encoder.Quality, 92L);
                    flat.Save(path, codec, ps);
                }
                break;
            case ".bmp":
                using (var flat = OnWhite(bmp)) flat.Save(path, ImageFormat.Bmp);
                break;
            case ".gif":
                bmp.Save(path, ImageFormat.Gif);
                break;
            default:
                bmp.Save(path, ImageFormat.Png);
                break;
        }
    }

    public static Bitmap Load(string path)
    {
        using var fs = File.OpenRead(path);
        using var img = Image.FromStream(fs);
        var bmp = new Bitmap(img.Width, img.Height, PixelFormat.Format32bppArgb);
        using var g = Graphics.FromImage(bmp);
        g.DrawImage(img, new Rectangle(0, 0, img.Width, img.Height));
        return bmp;
    }

    public static Bitmap OnWhite(Bitmap src)
    {
        var bmp = new Bitmap(src.Width, src.Height, PixelFormat.Format24bppRgb);
        using var g = Graphics.FromImage(bmp);
        g.Clear(Color.White);
        g.DrawImage(src, new Rectangle(0, 0, src.Width, src.Height));
        return bmp;
    }

    public static string UniquePath(string folder, string extension = ".png")
    {
        string baseName = $"ShotKit_{DateTime.Now:yyyy-MM-dd_HH-mm-ss}";
        string path = Path.Combine(folder, baseName + extension);
        for (int i = 2; File.Exists(path); i++)
            path = Path.Combine(folder, $"{baseName}_{i}{extension}");
        return path;
    }
}

static class ClipboardHelper
{
    public static bool SetImage(Bitmap bmp)
    {
        try
        {
            using var flat = ImageIO.OnWhite(bmp);
            using var png = new MemoryStream();
            bmp.Save(png, ImageFormat.Png);
            var data = new DataObject();
            data.SetData(DataFormats.Bitmap, true, flat);
            data.SetData("PNG", false, png);   // erhält Transparenz in Office, Paint.NET, Browsern …
            Clipboard.SetDataObject(data, true, 10, 100);
            return true;
        }
        catch
        {
            return false;
        }
    }
}
