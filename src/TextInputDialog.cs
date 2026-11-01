namespace ShotKit;

static class TextInputDialog
{
    /// <summary>Mehrzeilige Texteingabe. Rückgabe null = abgebrochen.</summary>
    public static string Ask(IWin32Window owner, string title, string initial)
    {
        using var form = new Form
        {
            Text = title,
            FormBorderStyle = FormBorderStyle.FixedDialog,
            StartPosition = FormStartPosition.CenterParent,
            MinimizeBox = false,
            MaximizeBox = false,
            ShowInTaskbar = false,
            AutoScaleMode = AutoScaleMode.Dpi,
            AutoScaleDimensions = new SizeF(96f, 96f),
            ClientSize = new Size(440, 200),
            Font = new Font("Segoe UI", 9f),
        };
        var box = new TextBox
        {
            Multiline = true,
            AcceptsReturn = true,
            ScrollBars = ScrollBars.Vertical,
            Text = initial.Replace("\n", "\r\n"),
            Font = new Font("Segoe UI", 12f),
            Location = new Point(12, 12),
            Size = new Size(416, 132),
        };
        var hint = new Label
        {
            Text = "Strg+Enter = OK",
            ForeColor = SystemColors.GrayText,
            AutoSize = true,
            Location = new Point(12, 162),
        };
        var ok = new Button { Text = "OK", DialogResult = DialogResult.OK, Location = new Point(262, 156), Size = new Size(80, 30) };
        var cancel = new Button { Text = "Abbrechen", DialogResult = DialogResult.Cancel, Location = new Point(348, 156), Size = new Size(80, 30) };
        box.KeyDown += (_, e) =>
        {
            if (e.Control && e.KeyCode == Keys.Enter) { e.SuppressKeyPress = true; form.DialogResult = DialogResult.OK; }
        };
        form.CancelButton = cancel;
        form.Controls.AddRange(new Control[] { box, hint, ok, cancel });
        form.Shown += (_, _) => { box.Focus(); box.SelectAll(); };

        return form.ShowDialog(owner) == DialogResult.OK ? box.Text.Replace("\r\n", "\n") : null;
    }
}
