using System.Diagnostics;

namespace ShotKit;

static class Program
{
    [STAThread]
    static void Main(string[] args)
    {
        using var mutex = new Mutex(true, "ShotKit.SingleInstance.7f3c2a91", out bool firstInstance);
        if (!firstInstance)
        {
            MessageBox.Show("ShotKit läuft bereits – siehe Symbol im Infobereich der Taskleiste.", "ShotKit",
                MessageBoxButtons.OK, MessageBoxIcon.Information);
            return;
        }
        ApplicationConfiguration.Initialize();
        Application.Run(new TrayApp(args));
    }
}

sealed class HotkeyWindow : NativeWindow, IDisposable
{
    readonly List<int> _ids = new();
    public event Action<int> Pressed;

    public HotkeyWindow() => CreateHandle(new CreateParams());

    public bool Register(int id, uint modifiers, Keys key)
    {
        if (!Native.RegisterHotKey(Handle, id, modifiers | Native.MOD_NOREPEAT, (uint)key)) return false;
        _ids.Add(id);
        return true;
    }

    public void Unregister(int id)
    {
        if (_ids.Remove(id)) Native.UnregisterHotKey(Handle, id);
    }

    protected override void WndProc(ref Message m)
    {
        if (m.Msg == Native.WM_HOTKEY) Pressed?.Invoke(m.WParam.ToInt32());
        base.WndProc(ref m);
    }

    public void Dispose()
    {
        foreach (int id in _ids) Native.UnregisterHotKey(Handle, id);
        _ids.Clear();
        DestroyHandle();
    }
}

sealed class TrayApp : ApplicationContext
{
    enum Mode { Region = 1, Monitor = 2, Window = 3, AllScreens = 4, Scroll = 5 }
    const int EscapeHotkeyId = 99;
    CancellationTokenSource _scrollCts;

    public static TrayApp Instance { get; private set; }
    public Settings Settings { get; }

    readonly NotifyIcon _tray;
    readonly HotkeyWindow _hotkeys = new();
    bool _busy;

    public TrayApp(string[] args)
    {
        Instance = this;
        Settings = Settings.Load();

        _tray = new NotifyIcon
        {
            Icon = Icons.AppIcon(),
            Text = "ShotKit – Druck: Bereich aufnehmen",
            ContextMenuStrip = BuildMenu(),
            Visible = true,
        };
        _tray.MouseClick += (_, e) => { if (e.Button == MouseButtons.Left) _ = CaptureRegionAsync(delayMs: 200); };

        _hotkeys.Pressed += id =>
        {
            if (id == EscapeHotkeyId) { _scrollCts?.Cancel(); return; }
            _ = (Mode)(id % 10) switch
            {
                Mode.Region => CaptureRegionAsync(),
                Mode.Monitor => CaptureScreenAsync(allScreens: false),
                Mode.AllScreens => CaptureScreenAsync(allScreens: true),
                Mode.Window => CaptureActiveWindowAsync(),
                Mode.Scroll => CaptureScrollingAsync(),
                _ => Task.CompletedTask,
            };
        };

        var failed = new List<string>();
        void Reg(Mode mode, uint mods, Keys key, string label, int offset = 0)
        {
            if (!_hotkeys.Register((int)mode + offset, mods, key)) failed.Add(label);
        }
        Reg(Mode.Region, 0, Keys.PrintScreen, "Druck");
        Reg(Mode.Monitor, Native.MOD_CONTROL, Keys.PrintScreen, "Strg+Druck");
        Reg(Mode.Window, Native.MOD_SHIFT, Keys.PrintScreen, "Umschalt+Druck");
        Reg(Mode.AllScreens, Native.MOD_CONTROL | Native.MOD_SHIFT, Keys.PrintScreen, "Strg+Umschalt+Druck");
        Reg(Mode.Scroll, Native.MOD_CONTROL | Native.MOD_ALT, Keys.PrintScreen, "Strg+Alt+Druck");
        // Ausweich-Tastenkürzel, falls Druck von Windows/Snipping Tool belegt ist
        Reg(Mode.Region, Native.MOD_CONTROL | Native.MOD_SHIFT, Keys.D1, "Strg+Umschalt+1", 10);
        Reg(Mode.Monitor, Native.MOD_CONTROL | Native.MOD_SHIFT, Keys.D2, "Strg+Umschalt+2", 10);
        Reg(Mode.Window, Native.MOD_CONTROL | Native.MOD_SHIFT, Keys.D3, "Strg+Umschalt+3", 10);
        Reg(Mode.Scroll, Native.MOD_CONTROL | Native.MOD_SHIFT, Keys.D4, "Strg+Umschalt+4", 10);

        if (failed.Count > 0)
            _tray.ShowBalloonTip(6000, "ShotKit – Tastenkürzel belegt",
                $"Nicht verfügbar: {string.Join(", ", failed)}.\nDiese Kürzel werden von einem anderen Programm genutzt.",
                ToolTipIcon.Warning);
        else
            _tray.ShowBalloonTip(3000, "ShotKit ist bereit",
                "Druck = Bereich/Fenster · Strg+Druck = Bildschirm · Umschalt+Druck = aktives Fenster",
                ToolTipIcon.Info);

        foreach (var a in args.Where(File.Exists)) OpenInEditor(a);
    }

    ContextMenuStrip BuildMenu()
    {
        var menu = new ContextMenuStrip();
        menu.Items.Add(new ToolStripMenuItem("Bereich / Fenster aufnehmen", null, (_, _) => _ = CaptureRegionAsync(delayMs: 200)) { ShortcutKeyDisplayString = "Druck", Font = new Font(menu.Font, FontStyle.Bold) });
        menu.Items.Add(new ToolStripMenuItem("Bildschirm aufnehmen", null, (_, _) => _ = CaptureScreenAsync(false, 200)) { ShortcutKeyDisplayString = "Strg+Druck" });
        menu.Items.Add(new ToolStripMenuItem("Alle Bildschirme aufnehmen", null, (_, _) => _ = CaptureScreenAsync(true, 200)) { ShortcutKeyDisplayString = "Strg+Umschalt+Druck" });
        menu.Items.Add(new ToolStripMenuItem("Aktives Fenster aufnehmen", null, (_, _) => _ = CaptureActiveWindowAsync()) { ShortcutKeyDisplayString = "Umschalt+Druck" });
        var delayed = new ToolStripMenuItem("Verzögert aufnehmen");
        foreach (int s in new[] { 3, 5, 10 })
            delayed.DropDownItems.Add($"Bereich in {s} Sekunden", null, (_, _) =>
            {
                _tray.ShowBalloonTip(1500, "ShotKit", $"Aufnahme in {s} Sekunden …", ToolTipIcon.None);
                _ = CaptureRegionAsync(delayMs: s * 1000);
            });
        menu.Items.Add(new ToolStripMenuItem("Scroll-Aufnahme (lange Seiten) …", null, (_, _) => _ = CaptureScrollingAsync(200)) { ShortcutKeyDisplayString = "Strg+Alt+Druck" });
        menu.Items.Add(delayed);
        menu.Items.Add(new ToolStripSeparator());

        menu.Items.Add("Bild öffnen …", null, (_, _) =>
        {
            using var dlg = new OpenFileDialog { Filter = ImageIO.OpenFilter, InitialDirectory = Settings.SaveFolder };
            if (dlg.ShowDialog() == DialogResult.OK) OpenInEditor(dlg.FileName);
        });
        menu.Items.Add("Bild aus Zwischenablage bearbeiten", null, (_, _) =>
        {
            if (!Clipboard.ContainsImage()) { _tray.ShowBalloonTip(2000, "ShotKit", "Die Zwischenablage enthält kein Bild.", ToolTipIcon.Info); return; }
            using var img = Clipboard.GetImage();
            ShowEditor(new Bitmap(img), null);
        });
        menu.Items.Add("Speicherordner öffnen", null, (_, _) => OpenFolder(null));
        menu.Items.Add(new ToolStripSeparator());

        var settings = new ToolStripMenuItem("Einstellungen");
        settings.DropDownItems.Add(Toggle("Nach Aufnahme Editor öffnen", () => Settings.OpenEditor, v => Settings.OpenEditor = v));
        settings.DropDownItems.Add(Toggle("Nach Aufnahme in Zwischenablage kopieren", () => Settings.CopyToClipboard, v => Settings.CopyToClipboard = v));
        settings.DropDownItems.Add(Toggle("Aufnahmen automatisch speichern", () => Settings.AutoSave, v => Settings.AutoSave = v));
        settings.DropDownItems.Add("Speicherordner ändern …", null, (_, _) => ChangeFolder());
        settings.DropDownItems.Add(new ToolStripSeparator());
        settings.DropDownItems.Add(Toggle("Mit Windows starten", () => SafeAutostart(), v =>
        {
            try { Autostart.Enabled = v; }
            catch (Exception ex) { MessageBox.Show(ex.Message, "ShotKit", MessageBoxButtons.OK, MessageBoxIcon.Error); }
        }, persist: false));
        menu.Items.Add(settings);
        menu.Items.Add(new ToolStripSeparator());
        menu.Items.Add("Beenden", null, (_, _) => Quit());
        return menu;
    }

    static bool SafeAutostart()
    {
        try { return Autostart.Enabled; } catch { return false; }
    }

    ToolStripMenuItem Toggle(string text, Func<bool> get, Action<bool> set, bool persist = true)
    {
        var item = new ToolStripMenuItem(text) { Checked = get() };
        item.Click += (_, _) =>
        {
            set(!get());
            item.Checked = get();
            if (persist) Settings.Save();
        };
        return item;
    }

    void ChangeFolder()
    {
        using var dlg = new FolderBrowserDialog { SelectedPath = Settings.SaveFolder, Description = "Ordner für Aufnahmen", UseDescriptionForTitle = true };
        if (dlg.ShowDialog() != DialogResult.OK) return;
        Settings.SaveFolder = dlg.SelectedPath;
        Settings.Save();
    }

    public void OpenFolder(string selectFile)
    {
        try
        {
            if (selectFile != null && File.Exists(selectFile))
            {
                Process.Start("explorer.exe", $"/select,\"{selectFile}\"");
                return;
            }
            Directory.CreateDirectory(Settings.SaveFolder);
            Process.Start(new ProcessStartInfo(Settings.SaveFolder) { UseShellExecute = true });
        }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "ShotKit", MessageBoxButtons.OK, MessageBoxIcon.Error);
        }
    }

    // ---------- Aufnahme ----------

    Task RunCapture(Form hide, int delayMs, Func<Bitmap> capture) => RunCaptureAsync(hide, delayMs, () => Task.FromResult(capture()));

    async Task RunCaptureAsync(Form hide, int delayMs, Func<Task<Bitmap>> capture)
    {
        if (_busy) return;
        _busy = true;
        Bitmap result = null;
        try
        {
            if (hide != null) { hide.Hide(); delayMs = Math.Max(delayMs, 250); }
            if (delayMs > 0) await Task.Delay(delayMs);
            result = await capture();
        }
        catch (Exception ex)
        {
            _tray.ShowBalloonTip(4000, "ShotKit – Aufnahme fehlgeschlagen", ex.Message, ToolTipIcon.Error);
        }
        finally
        {
            if (hide != null && !hide.IsDisposed) hide.Show();
            _busy = false;
        }
        if (result != null) HandleCapture(result);
    }

    public Task CaptureRegionAsync(Form hide = null, int delayMs = 0) => RunCapture(hide, delayMs, () =>
    {
        var windows = ScreenCapture.GetWindows();
        var vs = SystemInformation.VirtualScreen;
        using var shot = ScreenCapture.Capture(vs);
        using var overlay = new CaptureOverlay(shot, vs, windows);
        return overlay.ShowDialog() == DialogResult.OK ? overlay.Result : null;
    });

    const string ScrollHint = "Scroll-Aufnahme: Klick wählt den Inhaltsbereich (Webseite/Dokument ohne Leisten)  ·  oder Bereich ziehen  ·  Strg: ganzes Fenster  ·  Esc: Abbrechen";

    /// <summary>Bereich wählen → Richtung ankreuzen → Start; ShotKit scrollt und setzt das Bild zusammen.</summary>
    public Task CaptureScrollingAsync(int delayMs = 0) => RunCaptureAsync(null, delayMs, async () =>
    {
        Rectangle rect;
        var windows = ScreenCapture.GetWindows();
        var vs = SystemInformation.VirtualScreen;
        using (var shot = ScreenCapture.Capture(vs))
        using (var overlay = new CaptureOverlay(shot, vs, windows, ScrollHint, preferInner: true))
        {
            if (overlay.ShowDialog() != DialogResult.OK) return null;
            overlay.Result.Dispose();
            rect = overlay.SelectedRect;
        }
        if (rect.Width < 100 || rect.Height < 100)
        {
            _tray.ShowBalloonTip(3000, "ShotKit", "Der Bereich ist für eine Scroll-Aufnahme zu klein.", ToolTipIcon.Warning);
            return null;
        }

        _scrollCts = new CancellationTokenSource();
        bool escRegistered = _hotkeys.Register(EscapeHotkeyId, 0, Keys.Escape);
        try
        {
            var (image, info) = await ScrollCapture.RunAsync(rect, Settings.ScrollDirection, _scrollCts.Token,
                dir => { Settings.ScrollDirection = dir; Settings.Save(); });
            if (image != null && info != null)
                _tray.ShowBalloonTip(3000, $"Scroll-Aufnahme {image.Width} × {image.Height}", info, ToolTipIcon.Info);
            else if (image == null && info != null)
                _tray.ShowBalloonTip(4000, "ShotKit – Scroll-Aufnahme", info, ToolTipIcon.Warning);
            return image;
        }
        finally
        {
            if (escRegistered) _hotkeys.Unregister(EscapeHotkeyId);
            _scrollCts.Dispose();
            _scrollCts = null;
        }
    });

    Task CaptureScreenAsync(bool allScreens, int delayMs = 0) => RunCapture(null, delayMs, () =>
        ScreenCapture.Capture(allScreens ? SystemInformation.VirtualScreen : Screen.FromPoint(Cursor.Position).Bounds));

    Task CaptureActiveWindowAsync() => RunCapture(null, 0, () =>
    {
        var hWnd = Native.GetForegroundWindow();
        if (hWnd == IntPtr.Zero) return null;
        var r = Native.GetWindowBounds(hWnd);
        r.Intersect(SystemInformation.VirtualScreen);
        return r.Width > 0 && r.Height > 0 ? ScreenCapture.Capture(r) : null;
    });

    void HandleCapture(Bitmap bmp)
    {
        string path = null;
        if (Settings.AutoSave)
        {
            try
            {
                path = ImageIO.UniquePath(Settings.SaveFolder);
                ImageIO.Save(bmp, path);
            }
            catch (Exception ex)
            {
                path = null;
                _tray.ShowBalloonTip(4000, "ShotKit – Speichern fehlgeschlagen", ex.Message, ToolTipIcon.Error);
            }
        }
        bool copied = Settings.CopyToClipboard && ClipboardHelper.SetImage(bmp);

        if (Settings.OpenEditor)
        {
            ShowEditor(bmp, path);
            return;
        }
        string msg = (copied, path) switch
        {
            (true, not null) => $"In Zwischenablage kopiert und gespeichert:\n{path}",
            (true, null) => "In Zwischenablage kopiert.",
            (false, not null) => $"Gespeichert:\n{path}",
            _ => "Aufnahme erstellt (weder kopiert noch gespeichert – siehe Einstellungen).",
        };
        _tray.ShowBalloonTip(2500, $"Aufnahme {bmp.Width} × {bmp.Height}", msg, ToolTipIcon.Info);
        bmp.Dispose();
    }

    public void OpenInEditor(string path)
    {
        try { ShowEditor(ImageIO.Load(path), path); }
        catch (Exception ex) { MessageBox.Show($"Bild konnte nicht geöffnet werden:\n{ex.Message}", "ShotKit", MessageBoxButtons.OK, MessageBoxIcon.Error); }
    }

    static void ShowEditor(Bitmap bmp, string path)
    {
        var editor = new EditorForm(bmp, path);
        editor.Show();
        editor.Activate();
    }

    void Quit()
    {
        foreach (var f in Application.OpenForms.Cast<Form>().ToList()) f.Close();
        if (Application.OpenForms.Count > 0) return;   // Benutzer hat Schließen abgebrochen
        _tray.Visible = false;
        _tray.Dispose();
        _hotkeys.Dispose();
        ExitThread();
    }
}
