namespace ShotKit;

sealed class EditorForm : Form
{
    const int Pad = 24;

    static readonly (Tool tool, string name, Keys key)[] ToolDefs =
    {
        (Tool.Select, "Auswählen / Verschieben", Keys.V),
        (Tool.Arrow, "Pfeil", Keys.A),
        (Tool.Line, "Linie", Keys.L),
        (Tool.Rectangle, "Rechteck", Keys.R),
        (Tool.Ellipse, "Ellipse", Keys.E),
        (Tool.Text, "Text", Keys.T),
        (Tool.Highlight, "Textmarker", Keys.M),
        (Tool.Blur, "Verpixeln", Keys.B),
        (Tool.Step, "Schritt-Nummer", Keys.N),
        (Tool.Pen, "Freihand", Keys.P),
        (Tool.Crop, "Zuschneiden", Keys.C),
    };

    static readonly Dictionary<Tool, string> Hints = new()
    {
        [Tool.Select] = "Objekt anklicken zum Auswählen, ziehen zum Verschieben · Entf löscht · Doppelklick auf Text = bearbeiten",
        [Tool.Arrow] = "Ziehen zum Zeichnen · Umschalt = 45°-Schritte",
        [Tool.Line] = "Ziehen zum Zeichnen · Umschalt = 45°-Schritte",
        [Tool.Rectangle] = "Ziehen zum Zeichnen · Umschalt = Quadrat",
        [Tool.Ellipse] = "Ziehen zum Zeichnen · Umschalt = Kreis",
        [Tool.Text] = "Klicken, um Text zu platzieren · vorhandenen Text anklicken zum Bearbeiten",
        [Tool.Highlight] = "Bereich aufziehen, um ihn farbig hervorzuheben",
        [Tool.Blur] = "Bereich aufziehen, um ihn unkenntlich zu machen (Stärke = Linienbreite)",
        [Tool.Step] = "Klicken setzt fortlaufende Nummern",
        [Tool.Pen] = "Freihand zeichnen",
        [Tool.Crop] = "Bereich aufziehen, auf den zugeschnitten wird",
    };

    static readonly Color[] Palette =
    {
        Color.FromArgb(230, 40, 40), Color.FromArgb(255, 140, 0), Color.FromArgb(255, 214, 0),
        Color.FromArgb(40, 170, 70), Color.FromArgb(0, 120, 215), Color.FromArgb(140, 60, 200),
        Color.Black, Color.White,
    };

    readonly EditorCanvas _canvas;
    readonly Panel _host;
    static readonly int[] WidthValues = { 1, 2, 3, 4, 6, 8, 12, 16 };
    static readonly int[] FontValues = { 14, 18, 24, 28, 36, 48, 64, 96 };

    readonly ToolStrip _bar, _actions;
    readonly Panel _top;
    readonly List<ToolStripItem> _palItems, _whItems, _viewItems;
    readonly ToolStripDropDownButton _more;
    readonly List<ToolStripItem> _morePal = new(), _moreWh = new(), _moreView = new();
    readonly ToolStripMenuItem _moreUndo, _moreRedo;
    readonly ToolStripButton _capture, _copy, _saveBtn;
    bool _inResponsive;
    readonly StatusStrip _status;
    readonly ToolStripStatusLabel _lblHint, _lblSize, _lblZoom;
    readonly Dictionary<Tool, ToolStripButton> _toolButtons = new();
    readonly ToolStripButton _colorButton, _undoButton, _redoButton;
    readonly ToolStripComboBox _widthBox, _fontBox;
    readonly int _iconPx;
    string _filePath;

    public EditorForm(Bitmap image, string filePath)
    {
        _filePath = filePath;
        Icon = Icons.AppIcon();
        KeyPreview = true;
        AutoScaleMode = AutoScaleMode.Dpi;
        Font = new Font("Segoe UI", 9f);
        _iconPx = (int)Math.Round(20 * DeviceDpi / 96.0);

        var wa = Screen.FromPoint(Cursor.Position).WorkingArea;
        StartPosition = FormStartPosition.Manual;
        Size = new Size(Math.Min(wa.Width * 9 / 10, Math.Max(1000, image.Width + 120)),
                        Math.Min(wa.Height * 9 / 10, Math.Max(650, image.Height + 200)));
        Location = new Point(wa.X + (wa.Width - Width) / 2, wa.Y + (wa.Height - Height) / 2);
        MinimumSize = new Size(700, 400);

        _canvas = new EditorCanvas(image);
        _host = new Panel { Dock = DockStyle.Fill, AutoScroll = true, BackColor = Color.FromArgb(52, 52, 56) };
        _host.Controls.Add(_canvas);
        _host.Resize += (_, _) => LayoutCanvas();
        _canvas.SizeChanged += (_, _) => LayoutCanvas();
        _canvas.StateChanged += (_, _) => UpdateUi();

        // ---------- Werkzeugleiste ----------
        _bar = new ToolStrip
        {
            GripStyle = ToolStripGripStyle.Hidden,
            ImageScalingSize = new Size(_iconPx, _iconPx),
            Padding = new Padding(6, 4, 6, 4),
            RenderMode = ToolStripRenderMode.System,
        };

        foreach (var (tool, name, key) in ToolDefs)
        {
            var b = new ToolStripButton(Icons.ForTool(tool, _iconPx))
            {
                DisplayStyle = ToolStripItemDisplayStyle.Image,
                ToolTipText = $"{name} ({key})",
            };
            b.Click += (_, _) => SetTool(tool);
            _toolButtons[tool] = b;
            _bar.Items.Add(b);
        }
        _bar.Items.Add(new ToolStripSeparator());

        _colorButton = new ToolStripButton(Icons.Swatch(_canvas.CurrentColor, _iconPx))
        {
            DisplayStyle = ToolStripItemDisplayStyle.Image,
            ToolTipText = "Farbe wählen …",
        };
        _colorButton.Click += (_, _) => PickColor();
        _bar.Items.Add(_colorButton);
        int palStart = _bar.Items.Count;
        foreach (var c in Palette)
        {
            var b = new ToolStripButton(Icons.Swatch(c, _iconPx * 3 / 4, false))
            {
                DisplayStyle = ToolStripItemDisplayStyle.Image,
                ImageScaling = ToolStripItemImageScaling.None,
                ToolTipText = ColorTranslator.ToHtml(c),
                Margin = new Padding(0),
            };
            b.Click += (_, _) => SetColor(c);
            _bar.Items.Add(b);
        }
        _palItems = _bar.Items.Cast<ToolStripItem>().Skip(palStart).ToList();
        _bar.Items.Add(new ToolStripSeparator());

        int whStart = _bar.Items.Count;
        _bar.Items.Add(new ToolStripLabel("Breite"));
        _widthBox = MakeCombo(WidthValues, 4, "Linienbreite / Stärke");
        _widthBox.SelectedIndexChanged += (_, _) =>
        {
            float w = float.Parse((string)_widthBox.SelectedItem);
            _canvas.CurrentWidth = w;
            if (_canvas.Selected != null && _canvas.Selected.Width != w) _canvas.ApplyToSelected(s => s.Width = w);
        };
        _bar.Items.Add(_widthBox);

        _bar.Items.Add(new ToolStripLabel("Schrift"));
        _fontBox = MakeCombo(FontValues, 28, "Schriftgröße für Text");
        _fontBox.SelectedIndexChanged += (_, _) =>
        {
            float f = float.Parse((string)_fontBox.SelectedItem);
            _canvas.CurrentFontSize = f;
            if (_canvas.Selected is TextShape t && t.FontSize != f) _canvas.ApplyToSelected(s => s.FontSize = f);
        };
        _bar.Items.Add(_fontBox);
        _bar.Items.Add(new ToolStripSeparator());

        _undoButton = new ToolStripButton(Icons.UndoRedo(false, _iconPx)) { DisplayStyle = ToolStripItemDisplayStyle.Image, ToolTipText = "Rückgängig (Strg+Z)" };
        _undoButton.Click += (_, _) => _canvas.Undo();
        _redoButton = new ToolStripButton(Icons.UndoRedo(true, _iconPx)) { DisplayStyle = ToolStripItemDisplayStyle.Image, ToolTipText = "Wiederholen (Strg+Y)" };
        _redoButton.Click += (_, _) => _canvas.Redo();
        _bar.Items.Add(_undoButton);
        _bar.Items.Add(_redoButton);
        _bar.Items.Add(new ToolStripSeparator());
        _whItems = _bar.Items.Cast<ToolStripItem>().Skip(whStart).ToList();

        int viewStart = _bar.Items.Count;

        var effects = new ToolStripDropDownButton("Effekte") { ToolTipText = "Rahmen und Schatten (Rahmenfarbe = aktuelle Farbe)" };
        FillEffects(effects.DropDownItems);
        _bar.Items.Add(effects);

        var view = new ToolStripDropDownButton("Ansicht");
        FillView(view.DropDownItems);
        _bar.Items.Add(view);
        _viewItems = _bar.Items.Cast<ToolStripItem>().Skip(viewStart).ToList();

        // »-Menü: nimmt auf, was bei schmalem Fenster aus der Leiste verschwindet
        _more = new ToolStripDropDownButton("»") { ToolTipText = "Weitere Werkzeuge", DisplayStyle = ToolStripItemDisplayStyle.Text, ShowDropDownArrow = false, Available = false };
        _more.Font = new Font(Font.FontFamily, Font.Size + 3, FontStyle.Bold);
        var mEffects = new ToolStripMenuItem("Effekte");
        FillEffects(mEffects.DropDownItems);
        var mView = new ToolStripMenuItem("Ansicht");
        FillView(mView.DropDownItems);
        _moreView.AddRange(new ToolStripItem[] { mEffects, mView });
        var mWidth = new ToolStripMenuItem("Breite");
        foreach (var v in WidthValues) mWidth.DropDownItems.Add(v.ToString(), null, (_, _) => _widthBox.SelectedItem = v.ToString());
        var mFont = new ToolStripMenuItem("Schrift");
        foreach (var v in FontValues) mFont.DropDownItems.Add(v.ToString(), null, (_, _) => _fontBox.SelectedItem = v.ToString());
        _moreUndo = new ToolStripMenuItem("Rückgängig", null, (_, _) => _canvas.Undo()) { ShortcutKeyDisplayString = "Strg+Z" };
        _moreRedo = new ToolStripMenuItem("Wiederholen", null, (_, _) => _canvas.Redo()) { ShortcutKeyDisplayString = "Strg+Y" };
        _moreWh.AddRange(new ToolStripItem[] { mWidth, mFont, _moreUndo, _moreRedo });
        var mColor = new ToolStripMenuItem("Farbpalette");
        foreach (var c in Palette)
            mColor.DropDownItems.Add(new ToolStripMenuItem(ColorTranslator.ToHtml(c), Icons.Swatch(c, _iconPx * 3 / 4, false), (_, _) => SetColor(c)));
        _morePal.Add(mColor);
        _more.DropDownItems.AddRange(_moreView.Concat(_moreWh).Concat(_morePal).ToArray());
        _bar.Items.Add(_more);

        // feste Aktionsleiste rechts: Aufnahme, Kopieren, Datei sind immer sichtbar
        _actions = new ToolStrip
        {
            GripStyle = ToolStripGripStyle.Hidden,
            ImageScalingSize = new Size(_iconPx, _iconPx),
            Padding = new Padding(6, 4, 6, 4),
            RenderMode = ToolStripRenderMode.System,
            CanOverflow = false,
            Dock = DockStyle.Right,
        };
        // Dock=Right stellt die Leiste sonst automatisch auf senkrecht
        _actions.LayoutStyle = ToolStripLayoutStyle.HorizontalStackWithOverflow;

        _capture = new ToolStripButton("Aufnahme", Icons.Action("capture", _iconPx))
        {
            ToolTipText = "Neue Bereichsaufnahme (Strg+N)", DisplayStyle = ToolStripItemDisplayStyle.ImageAndText,
        };
        _capture.Click += (_, _) => _ = TrayApp.Instance.CaptureRegionAsync(this);
        _actions.Items.Add(_capture);
        _copy = new ToolStripButton("Kopieren", Icons.Action("copy", _iconPx))
        {
            ToolTipText = "In Zwischenablage kopieren (Strg+C)", DisplayStyle = ToolStripItemDisplayStyle.ImageAndText,
            Font = new Font(Font, FontStyle.Bold),
        };
        _copy.Click += (_, _) => CopyToClipboard();
        _actions.Items.Add(_copy);
        _saveBtn = new ToolStripButton("Speichern", Icons.Action("save", _iconPx))
        {
            ToolTipText = "Speichern (Strg+S)", DisplayStyle = ToolStripItemDisplayStyle.ImageAndText,
        };
        _saveBtn.Click += (_, _) => Save();
        _actions.Items.Add(_saveBtn);

        var file = new ToolStripDropDownButton("Datei");
        file.DropDownItems.Add(new ToolStripMenuItem("Speichern", null, (_, _) => Save()) { ShortcutKeyDisplayString = "Strg+S" });
        file.DropDownItems.Add(new ToolStripMenuItem("Speichern unter …", null, (_, _) => SaveAs()) { ShortcutKeyDisplayString = "Strg+Umschalt+S" });
        file.DropDownItems.Add(new ToolStripMenuItem("Öffnen …", null, (_, _) => OpenFile()) { ShortcutKeyDisplayString = "Strg+O" });
        file.DropDownItems.Add(new ToolStripSeparator());
        file.DropDownItems.Add("Bild aus Zwischenablage einfügen", null, (_, _) => PasteImage());
        file.DropDownItems.Add("Speicherordner öffnen", null, (_, _) => TrayApp.Instance.OpenFolder(_filePath));
        _actions.Items.Add(file);

        // linke Leiste füllt den Rest; passt etwas nicht mehr hinein, räumt ApplyResponsive es ins »-Menü
        _bar.Dock = DockStyle.Fill;
        _bar.CanOverflow = true;
        _top = new Panel { Dock = DockStyle.Top, Height = Math.Max(_bar.PreferredSize.Height, _actions.PreferredSize.Height) };
        _top.Controls.Add(_bar);
        _top.Controls.Add(_actions);
        _top.SizeChanged += (_, _) => ApplyResponsive();
        Load += (_, _) => ApplyResponsive();

        // ---------- Statusleiste ----------
        _status = new StatusStrip { SizingGrip = true };
        _lblHint = new ToolStripStatusLabel { Spring = true, TextAlign = ContentAlignment.MiddleLeft };
        _lblSize = new ToolStripStatusLabel { BorderSides = ToolStripStatusLabelBorderSides.Left };
        _lblZoom = new ToolStripStatusLabel { BorderSides = ToolStripStatusLabelBorderSides.Left, IsLink = true, ToolTipText = "Klicken: Originalgröße / Einpassen" };
        _lblZoom.Click += (_, _) => { if (Math.Abs(_canvas.Zoom - 1f) < 0.001f) ZoomToFit(); else _canvas.SetZoom(1f); };
        _status.Items.AddRange(new ToolStripItem[] { _lblHint, _lblSize, _lblZoom });

        Controls.Add(_host);
        Controls.Add(_top);
        Controls.Add(_status);

        SetTool(Tool.Arrow);
    }

    void FillEffects(ToolStripItemCollection items)
    {
        items.Add("Rahmen", null, (_, _) => _canvas.ApplyEffect((int)Math.Max(2, _canvas.CurrentWidth), false));
        items.Add("Schlagschatten", null, (_, _) => _canvas.ApplyEffect(0, true));
        items.Add("Rahmen + Schlagschatten", null, (_, _) => _canvas.ApplyEffect((int)Math.Max(2, _canvas.CurrentWidth), true));
    }

    void FillView(ToolStripItemCollection items)
    {
        items.Add(new ToolStripMenuItem("Vergrößern", null, (_, _) => _canvas.SetZoom(_canvas.Zoom * 1.25f)) { ShortcutKeyDisplayString = "Strg++" });
        items.Add(new ToolStripMenuItem("Verkleinern", null, (_, _) => _canvas.SetZoom(_canvas.Zoom * 0.8f)) { ShortcutKeyDisplayString = "Strg+-" });
        items.Add(new ToolStripMenuItem("Originalgröße", null, (_, _) => _canvas.SetZoom(1f)) { ShortcutKeyDisplayString = "Strg+0" });
        items.Add(new ToolStripMenuItem("Einpassen", null, (_, _) => ZoomToFit()) { ShortcutKeyDisplayString = "Strg+9" });
    }

    // Stufen beim Verkleinern: 0 alles · 1 Effekte/Ansicht im »-Menü · 2 + Breite/Schrift/Undo/Redo, Speichern nur im Datei-Menü
    // · 3 + Farbpalette eingeklappt, Aufnahme/Kopieren nur als Symbol. Aufnahme, Kopieren und Datei bleiben immer sichtbar.
    void ApplyResponsive()
    {
        if (_inResponsive || _top == null || _top.ClientSize.Width <= 0) return;
        _inResponsive = true;
        _top.SuspendLayout();
        try
        {
            int stage = 0;
            for (; stage < 3; stage++)
            {
                ApplyStage(stage);
                if (NeedWidth(_bar) + NeedWidth(_actions) <= _top.ClientSize.Width) break;
            }
            ApplyStage(stage);
        }
        finally
        {
            _top.ResumeLayout(true);
            _inResponsive = false;
        }
    }

    void ApplyStage(int stage)
    {
        foreach (var i in _viewItems) i.Available = stage < 1;
        foreach (var i in _whItems) i.Available = stage < 2;
        foreach (var i in _palItems) i.Available = stage < 3;
        foreach (var i in _moreView) i.Available = stage >= 1;
        foreach (var i in _moreWh) i.Available = stage >= 2;
        foreach (var i in _morePal) i.Available = stage >= 3;
        _more.Available = stage >= 1;
        _saveBtn.Available = stage < 2;
        var style = stage < 3 ? ToolStripItemDisplayStyle.ImageAndText : ToolStripItemDisplayStyle.Image;
        _capture.DisplayStyle = style;
        _copy.DisplayStyle = style;
    }

    static int NeedWidth(ToolStrip strip)
    {
        int w = strip.Padding.Horizontal + 8;
        foreach (ToolStripItem i in strip.Items)
            if (i.Available) w += i.GetPreferredSize(Size.Empty).Width + i.Margin.Horizontal;
        return w;
    }

    ToolStripComboBox MakeCombo(int[] values, int initial, string tip)
    {
        var box = new ToolStripComboBox { DropDownStyle = ComboBoxStyle.DropDownList, AutoSize = false, Width = 56, ToolTipText = tip };
        foreach (var v in values) box.Items.Add(v.ToString());
        box.SelectedItem = initial.ToString();
        return box;
    }

    protected override void OnShown(EventArgs e)
    {
        base.OnShown(e);
        ZoomToFit();
        Activate();
    }

    void ZoomToFit()
    {
        var cs = _host.ClientSize;
        float fitW = (cs.Width - 2f * Pad) / _canvas.Image.Width, fitH = (cs.Height - 2f * Pad) / _canvas.Image.Height;
        // lange Scroll-Aufnahmen auf Breite einpassen, sonst ganz
        float fit = _canvas.Image.Height > 2.5f * _canvas.Image.Width ? fitW : Math.Min(fitW, fitH);
        _canvas.SetZoom(Math.Clamp(fit, 0.02f, 1f));
    }

    void LayoutCanvas()
    {
        _host.AutoScrollMinSize = new Size(_canvas.Width + 2 * Pad, _canvas.Height + 2 * Pad);
        var cs = _host.ClientSize;
        int x = _canvas.Width + 2 * Pad < cs.Width ? (cs.Width - _canvas.Width) / 2 : Pad + _host.AutoScrollPosition.X;
        int y = _canvas.Height + 2 * Pad < cs.Height ? (cs.Height - _canvas.Height) / 2 : Pad + _host.AutoScrollPosition.Y;
        _canvas.Location = new Point(x, y);
    }

    void SetTool(Tool tool)
    {
        _canvas.Tool = tool;
        if (tool == Tool.Highlight && _canvas.CurrentColor == Palette[0]) SetColor(Palette[2]);
    }

    void SetColor(Color c)
    {
        _canvas.CurrentColor = c;
        var old = _colorButton.Image;
        _colorButton.Image = Icons.Swatch(c, _iconPx);
        old?.Dispose();
        if (_canvas.Selected != null && _canvas.Selected.Color != c) _canvas.ApplyToSelected(s => s.Color = c);
    }

    void PickColor()
    {
        using var dlg = new ColorDialog { Color = _canvas.CurrentColor, FullOpen = true, AnyColor = true };
        if (dlg.ShowDialog(this) == DialogResult.OK) SetColor(dlg.Color);
    }

    void UpdateUi()
    {
        foreach (var (tool, b) in _toolButtons) b.Checked = tool == _canvas.Tool;
        _undoButton.Enabled = _canvas.CanUndo;
        _redoButton.Enabled = _canvas.CanRedo;
        _moreUndo.Enabled = _canvas.CanUndo;
        _moreRedo.Enabled = _canvas.CanRedo;
        _lblHint.Text = Hints[_canvas.Tool];
        _lblSize.Text = $"{_canvas.Image.Width} × {_canvas.Image.Height} px";
        _lblZoom.Text = $"{_canvas.Zoom * 100:0} %";
        string name = _filePath != null ? Path.GetFileName(_filePath) : "Neue Aufnahme";
        Text = $"{name}{(_canvas.Dirty ? " •" : "")} – ShotKit";
    }

    // ---------- Datei / Zwischenablage ----------

    void CopyToClipboard()
    {
        using var flat = _canvas.Flatten();
        _lblHint.Text = ClipboardHelper.SetImage(flat)
            ? "✔ In die Zwischenablage kopiert"
            : "Zwischenablage ist gerade belegt – bitte erneut versuchen";
    }

    bool Save()
    {
        string path = _filePath;
        if (path == null || !IsWritableFormat(path))
            path = ImageIO.UniquePath(TrayApp.Instance.Settings.SaveFolder);
        return SaveTo(path);
    }

    bool SaveAs()
    {
        using var dlg = new SaveFileDialog
        {
            Filter = ImageIO.SaveFilter,
            InitialDirectory = _filePath != null ? Path.GetDirectoryName(_filePath) : TrayApp.Instance.Settings.SaveFolder,
            FileName = _filePath != null ? Path.GetFileNameWithoutExtension(_filePath) : Path.GetFileNameWithoutExtension(ImageIO.UniquePath(TrayApp.Instance.Settings.SaveFolder)),
            AddExtension = true,
            DefaultExt = "png",
        };
        return dlg.ShowDialog(this) == DialogResult.OK && SaveTo(dlg.FileName);
    }

    static bool IsWritableFormat(string path) =>
        Path.GetExtension(path).ToLowerInvariant() is ".png" or ".jpg" or ".jpeg" or ".bmp" or ".gif";

    bool SaveTo(string path)
    {
        try
        {
            using var flat = _canvas.Flatten();
            ImageIO.Save(flat, path);
            _filePath = path;
            _canvas.Dirty = false;
            UpdateUi();
            _lblHint.Text = $"✔ Gespeichert: {path}";
            return true;
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, $"Speichern fehlgeschlagen:\n{ex.Message}", "ShotKit", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return false;
        }
    }

    void OpenFile()
    {
        using var dlg = new OpenFileDialog { Filter = ImageIO.OpenFilter, InitialDirectory = TrayApp.Instance.Settings.SaveFolder };
        if (dlg.ShowDialog(this) != DialogResult.OK) return;
        TrayApp.Instance.OpenInEditor(dlg.FileName);
    }

    void PasteImage()
    {
        if (!Clipboard.ContainsImage()) { _lblHint.Text = "Die Zwischenablage enthält kein Bild"; return; }
        using var img = Clipboard.GetImage();
        _canvas.ReplaceImage(new Bitmap(img));
        ZoomToFit();
    }

    // ---------- Tastatur ----------

    protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
    {
        switch (keyData)
        {
            case Keys.Control | Keys.Z: _canvas.Undo(); return true;
            case Keys.Control | Keys.Y:
            case Keys.Control | Keys.Shift | Keys.Z: _canvas.Redo(); return true;
            case Keys.Control | Keys.C: CopyToClipboard(); return true;
            case Keys.Control | Keys.S: Save(); return true;
            case Keys.Control | Keys.Shift | Keys.S: SaveAs(); return true;
            case Keys.Control | Keys.O: OpenFile(); return true;
            case Keys.Control | Keys.V: PasteImage(); return true;
            case Keys.Control | Keys.N: _ = TrayApp.Instance.CaptureRegionAsync(this); return true;
            case Keys.Control | Keys.Oemplus:
            case Keys.Control | Keys.Add: _canvas.SetZoom(_canvas.Zoom * 1.25f); return true;
            case Keys.Control | Keys.OemMinus:
            case Keys.Control | Keys.Subtract: _canvas.SetZoom(_canvas.Zoom * 0.8f); return true;
            case Keys.Control | Keys.D0:
            case Keys.Control | Keys.NumPad0: _canvas.SetZoom(1f); return true;
            case Keys.Control | Keys.D9:
            case Keys.Control | Keys.NumPad9: ZoomToFit(); return true;
            case Keys.Delete:
            case Keys.Back: _canvas.DeleteSelected(); return true;
            case Keys.Escape:
                if (_canvas.Selected != null) _canvas.SelectShape(null);
                else if (_canvas.Tool != Tool.Select) SetTool(Tool.Select);
                return true;
        }

        var code = keyData & Keys.KeyCode;
        var mods = keyData & Keys.Modifiers;
        if (_canvas.Selected != null && (mods == Keys.None || mods == Keys.Shift) &&
            code is Keys.Left or Keys.Right or Keys.Up or Keys.Down)
        {
            float step = mods == Keys.Shift ? 10 : 1;
            _canvas.NudgeSelected(code == Keys.Left ? -step : code == Keys.Right ? step : 0,
                                  code == Keys.Up ? -step : code == Keys.Down ? step : 0);
            return true;
        }
        if (mods == Keys.None)
        {
            foreach (var (tool, _, key) in ToolDefs)
                if (key == code) { SetTool(tool); return true; }
        }
        return base.ProcessCmdKey(ref msg, keyData);
    }

    protected override void OnFormClosing(FormClosingEventArgs e)
    {
        if (_canvas.Dirty && e.CloseReason == CloseReason.UserClosing || _canvas.Dirty && e.CloseReason == CloseReason.ApplicationExitCall)
        {
            Activate();
            var answer = MessageBox.Show(this, "Änderungen speichern?", "ShotKit",
                MessageBoxButtons.YesNoCancel, MessageBoxIcon.Question);
            if (answer == DialogResult.Cancel || (answer == DialogResult.Yes && !Save()))
                e.Cancel = true;
        }
        base.OnFormClosing(e);
    }
}
