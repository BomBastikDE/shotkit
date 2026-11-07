param(
    [Parameter(Mandatory = $true)][string] $ExePath,
    [int] $TimeoutSeconds = 30
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'NO_INTERACTIVE_DESKTOP: multi-editor smoke requires an interactive Windows desktop.'
}
if ($TimeoutSeconds -lt 5 -or $TimeoutSeconds -gt 180) { throw 'TimeoutSeconds must be 5..180.' }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$name = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0) {
    throw 'PREEXISTING_PROCESS: refusing to modify an existing ShotKit process.'
}
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class ShotKitMultiSmoke {
    public struct Rect { public int Left, Top, Right, Bottom; }
    public sealed class Window { public IntPtr Handle; public string Class, Title; public bool Visible; }
    delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);

        // Narrow native identity checks for the expected history-loss message box.
        [DllImport("user32.dll", EntryPoint="GetWindow")] static extern IntPtr HistoryWindow(IntPtr h, uint command);
        [DllImport("user32.dll", EntryPoint="IsWindowEnabled")] public static extern bool HistoryEnabled(IntPtr h);
        [DllImport("user32.dll", EntryPoint="IsWindow")] public static extern bool HistoryExists(IntPtr h);
        [DllImport("user32.dll", EntryPoint="GetParent")] static extern IntPtr HistoryParent(IntPtr h);
        [DllImport("user32.dll", EntryPoint="GetDlgCtrlID")] static extern int HistoryId(IntPtr h);
        [DllImport("user32.dll", EntryPoint="PostMessageW")] public static extern bool HistoryPost(IntPtr h, uint m, IntPtr w, IntPtr l);
        static string HistoryText(IntPtr h, bool cls) {
            var text=new StringBuilder(1024);
            if(cls) GetClassNameW(h,text,text.Capacity); else GetWindowTextW(h,text,text.Capacity);
            return text.ToString();
        }
        static bool HistoryPid(IntPtr h, uint expected) { uint pid; GetWindowThreadProcessId(h,out pid); return pid==expected; }
        public static IntPtr HistoryYes(IntPtr dialog, IntPtr editor, uint pid, string expectedText) {
            if(!HistoryExists(dialog) || !HistoryPid(dialog,pid) || !HistoryPid(editor,pid) ||
                HistoryWindow(dialog,4)!=editor || HistoryEnabled(editor) || !HistoryEnabled(dialog) ||
                HistoryText(dialog,true)!="#32770" || HistoryText(dialog,false)!="Rückgängig-Verlauf")
                throw new InvalidOperationException("REDO_CONFIRMATION_OWNER_OR_MODALITY_MISMATCH");
            int messages=0, buttons=0, yesCount=0, noCount=0; IntPtr yes=IntPtr.Zero;
            EnumProc scan=(h,p)=>{
                string cls=HistoryText(h,true);
                if(cls=="Static" && HistoryText(h,false)==expectedText && HistoryParent(h)==dialog && HistoryPid(h,pid)) messages++;
                if(cls=="Button" && IsWindowVisible(h)) {
                    buttons++;
                    if(HistoryParent(h)!=dialog || !HistoryPid(h,pid) || !HistoryEnabled(h)) return true;
                    if(HistoryId(h)==6) { yesCount++; yes=h; }
                    if(HistoryId(h)==7) noCount++;
                }
                return true;
            };
            EnumChildWindows(dialog,scan,IntPtr.Zero); GC.KeepAlive(scan);
            return messages==1 && buttons==2 && yesCount==1 && noCount==1 ? yes : IntPtr.Zero;
        }
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr desktop);
    static Window Read(IntPtr h) {
        var c = new StringBuilder(256); var t = new StringBuilder(1024);
        GetClassNameW(h,c,c.Capacity); GetWindowTextW(h,t,t.Capacity);
        return new Window { Handle=h, Class=c.ToString(), Title=t.ToString(), Visible=IsWindowVisible(h) };
    }
    public static Window[] Windows(uint wanted) {
        var found = new List<Window>();
        EnumProc cb = (h,p) => { uint pid; GetWindowThreadProcessId(h,out pid); if(pid==wanted) found.Add(Read(h)); return true; };
        EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb); return found.ToArray();
    }
    public static Window[] Children(IntPtr parent) {
        var found = new List<Window>();
        EnumProc cb = (h,p) => { found.Add(Read(h)); return true; };
        EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return found.ToArray();
    }
}
'@
$desktop = [ShotKitMultiSmoke]::OpenInputDesktop(0, $false, 0x0100)
if ($desktop -eq [IntPtr]::Zero) { throw 'NO_INTERACTIVE_DESKTOP: input desktop unavailable.' }
[void][ShotKitMultiSmoke]::CloseDesktop($desktop)
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Multi-' + [Guid]::NewGuid().ToString('N'))
$process = $null
$startTicks = $null
function Get-Windows { @([ShotKitMultiSmoke]::Windows([uint32]$process.Id)) }
function Get-Editors { @(Get-Windows | Where-Object { $_.Class -eq 'ShotKitRustEditor' -and $_.Visible }) }
function Wait-Value([scriptblock] $Probe, [string] $Description) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($process.HasExited) { throw "PROCESS_EXITED: $Description" }
        $value = & $Probe
        if ($value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    $dialogs = (Get-Windows | Where-Object { $_.Class -eq '#32770' } | ForEach-Object { $_.Title }) -join ', '
    throw "TIMEOUT: $Description; dialogs: $dialogs"
}
function Post([IntPtr] $Handle, [uint32] $Message, [long] $W = 0, [long] $L = 0) {
    if (-not [ShotKitMultiSmoke]::IsWindow($Handle)) { throw "Invalid owned HWND $Handle" }
    if (-not [ShotKitMultiSmoke]::PostMessageW($Handle,$Message,[IntPtr]$W,[IntPtr]$L)) { throw 'PostMessage failed.' }
}
function Get-Editor([IntPtr] $Handle) { Get-Editors | Where-Object { $_.Handle -eq $Handle } | Select-Object -First 1 }
function Get-Dialog([string] $Title) { Get-Windows | Where-Object { $_.Visible -and $_.Class -eq '#32770' -and $_.Title -like $Title } | Select-Object -First 1 }
function Answer([string] $Title, [string[]] $Labels) {
    $dialog = Wait-Value { Get-Dialog $Title } "dialog $Title"
    $button = Wait-Value {
        [ShotKitMultiSmoke]::Children($dialog.Handle) | Where-Object {
            $_.Visible -and $_.Class -eq 'Button' -and $Labels -contains (($_.Title -replace '&','').Trim())
        } | Select-Object -First 1
    } "owned answer button $($Labels -join '/')"
    Post $button.Handle 0x00f5
    [void](Wait-Value { -not [ShotKitMultiSmoke]::IsWindow($dialog.Handle) } "dialog $Title closed")
}
function Dirty([IntPtr] $Handle) {
    $window = Get-Editor $Handle
    return $null -ne $window -and $window.Title.StartsWith('* ')
}
function Confirm-RedoDiscard([int]$ExpectedRedo,[IntPtr]$Owner) {
    $expectedText="Für diese Bearbeitung werden 0 ältere Rückgängig-Schritte und $ExpectedRedo Wiederholen-Schritte entfernt. Möchten Sie fortfahren?"
    $dialog=Wait-Value {
        $dialogs=@(Get-Windows | Where-Object {$_.Visible -and $_.Class -eq '#32770'})
        if(-not $dialogs.Count){return $null}
        if($dialogs.Count -ne 1 -or $dialogs[0].Title -cne 'Rückgängig-Verlauf'){throw 'UNEXPECTED_REDO_CONFIRMATION_DIALOG'}
        return $dialogs[0]
    } 'expected owned Redo-discard confirmation'
    $yes=Wait-Value {
        $button=[ShotKitMultiSmoke]::HistoryYes($dialog.Handle,$Owner,[uint32]$process.Id,$expectedText)
        if($button -ne [IntPtr]::Zero){return $button}
    } 'exact history-loss text/counts and native IDYES/IDNO'
    if(-not [ShotKitMultiSmoke]::HistoryPost($yes,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'REDO_CONFIRMATION_YES_FAILED'}
    [void](Wait-Value { -not [ShotKitMultiSmoke]::HistoryExists($dialog.Handle) -and [ShotKitMultiSmoke]::HistoryEnabled($Owner) } 'history-loss confirmation completed')
}
function Draw-Rectangle([IntPtr] $Handle,[int] $ExpectedRedoDiscard=0) {
    $client = New-Object ShotKitMultiSmoke+Rect
    if (-not [ShotKitMultiSmoke]::GetClientRect($Handle, [ref]$client)) { throw 'GetClientRect failed.' }
    $bar = [ShotKitMultiSmoke]::Children($Handle) | Where-Object { $_.Class -eq 'ShotKitRustEditorToolbar' } | Select-Object -First 1
    if ($null -eq $bar) { throw 'Document toolbar missing.' }
    $barRect = New-Object ShotKitMultiSmoke+Rect
    if (-not [ShotKitMultiSmoke]::GetWindowRect($bar.Handle,[ref]$barRect)) { throw 'Toolbar rectangle unavailable.' }
    $height = $barRect.Bottom - $barRect.Top
    $x = [int](($client.Right - $client.Left) / 2)
    $y = [int](($client.Bottom - $height) / 2 + $height)
    Post $Handle 0x0111 0x5104 # rectangle tool
    Post $Handle 0x0201 1 (($y -shl 16) -bor ($x - 50))
    Post $Handle 0x0200 1 ((($y + 40) -shl 16) -bor ($x + 50))
    Post $Handle 0x0202 0 ((($y + 40) -shl 16) -bor ($x + 50))
    if($ExpectedRedoDiscard -gt 0){Confirm-RedoDiscard $ExpectedRedoDiscard $Handle}
    [void](Wait-Value { Dirty $Handle } 'document dirty after rectangle')
}
function Pixels([string] $Path) {
    $bitmap = [Drawing.Bitmap]::new($Path)
    try {
        $bytes = New-Object IO.MemoryStream
        $writer = [IO.BinaryWriter]::new($bytes)
        try {
            $writer.Write($bitmap.Width); $writer.Write($bitmap.Height)
            for ($y=0; $y -lt $bitmap.Height; $y++) { for ($x=0; $x -lt $bitmap.Width; $x++) { $writer.Write($bitmap.GetPixel($x,$y).ToArgb()) } }
            return [Convert]::ToBase64String($bytes.ToArray())
        } finally { $writer.Dispose(); $bytes.Dispose() }
    } finally { $bitmap.Dispose() }
}
try {
    [void](New-Item -ItemType Directory -Path $testRoot)
    $paths = @((Join-Path $testRoot 'a.png'), (Join-Path $testRoot 'b.png'))
    foreach ($path in $paths) {
        $bitmap = [Drawing.Bitmap]::new(200,160)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try { $graphics.Clear([Drawing.Color]::FromArgb(255,20,80,120)); $bitmap.Save($path,[Drawing.Imaging.ImageFormat]::Png) }
        finally { $graphics.Dispose(); $bitmap.Dispose() }
    }
    $original = Pixels $paths[0]
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName=$ExePath; $start.WorkingDirectory=$testRoot; $start.UseShellExecute=$false
    $start.Arguments='"' + ($paths -join '" "') + '"'
    $start.EnvironmentVariables['APPDATA'] = Join-Path $testRoot 'appdata'
    $start.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $testRoot 'localappdata'
    $process = [Diagnostics.Process]::Start($start)
    $startTicks=$process.StartTime.ToUniversalTime().Ticks
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { Get-Windows }
    [void](Wait-Value { (Get-Editors).Count -eq 2 } 'both startup paths opened simultaneously')
    $a = Get-Editors | Where-Object { $_.Title.Contains($paths[0]) } | Select-Object -First 1
    $b = Get-Editors | Where-Object { $_.Title.Contains($paths[1]) } | Select-Object -First 1
    if ($null -eq $a -or $null -eq $b -or $a.Handle -eq $b.Handle) { throw 'Independent document identities missing.' }
    $hostWindow = Get-Windows | Where-Object { $_.Class -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    if ($null -eq $hostWindow -or $hostWindow.Visible) { throw 'Hidden persistent tray/IPC host missing.' }
    Draw-Rectangle $a.Handle
    if (Dirty $b.Handle) { throw 'Editing A changed B dirty state.' }
    Post $b.Handle 0x0111 0x5110 # save clean B, independent from A
    Start-Sleep -Milliseconds 300
    if ((Pixels $paths[0]) -ne $original -or (Pixels $paths[1]) -ne $original) { throw 'Saving B changed A or B image pixels unexpectedly.' }
    Draw-Rectangle $b.Handle
    Post $b.Handle 0x0111 0x5110
    [void](Wait-Value { -not (Dirty $b.Handle) } 'B saved independently')
    if (-not (Dirty $a.Handle) -or (Pixels $paths[0]) -ne $original -or (Pixels $paths[1]) -eq $original) { throw 'Document save isolation failed.' }
    Post $b.Handle 0x0111 0x5112 # B undo
    [void](Wait-Value { Dirty $b.Handle } 'B undo dirtied B')
    Post $b.Handle 0x0111 0x5110
    [void](Wait-Value { -not (Dirty $b.Handle) } 'B undo saved')
    if ((Pixels $paths[1]) -ne $original -or -not (Dirty $a.Handle)) { throw 'Independent B undo history failed.' }
    # Force an I/O failure exclusively inside this test's temporary directory.
    $backup = $paths[0] + '.backup'
    Move-Item -LiteralPath $paths[0] -Destination $backup
    [void](New-Item -ItemType Directory -Path $paths[0])
    Post $a.Handle 0x0010
    Answer 'ShotKit' @('Yes','Ja')
    Answer 'ShotKit' @('OK')
    if (-not [ShotKitMultiSmoke]::IsWindow($a.Handle) -or -not (Dirty $a.Handle)) { throw 'Failed save closed or cleaned dirty A.' }
    Remove-Item -LiteralPath $paths[0] -Force
    Move-Item -LiteralPath $backup -Destination $paths[0]
    Post $a.Handle 0x0010
    Answer 'ShotKit' @('Cancel','Abbrechen')
    if ((Get-Editors).Count -ne 2) { throw 'Cancelled close removed an editor.' }
    Post $hostWindow.Handle 0x0010 # application quit must confirm A
    Answer 'ShotKit' @('Cancel','Abbrechen')
    if ($process.HasExited -or (Get-Editors).Count -ne 2) { throw 'Cancelled quit removed a document or process.' }
    Post $a.Handle 0x0111 0x5110
    [void](Wait-Value { -not (Dirty $a.Handle) } 'A save after failed/cancelled close')
    if ((Pixels $paths[0]) -eq $original -or (Pixels $paths[1]) -ne $original) { throw 'A save exported wrong document.' }
    Post $a.Handle 0x0010
    [void](Wait-Value { (Get-Editors).Count -eq 1 } 'clean A close preserved B')
    Draw-Rectangle $b.Handle 1 # B has one undone rectangle, even after saving
    Post $b.Handle 0x0010
    Answer 'ShotKit' @('No','Nein')
    [void](Wait-Value { (Get-Editors).Count -eq 0 } 'discard B closed only B')
    if ($process.HasExited) { throw 'Closing last editor terminated tray/IPC host.' }
    Post $hostWindow.Handle 0x8004 # a second launch without files must offer a visible entry
    $empty = Wait-Value { Get-Editors | Select-Object -First 1 } 'visible empty editor after activation'
    if ($empty.Title -notlike '*Bild öffnen oder einfügen*' -or (Dirty $empty.Handle)) { throw 'Empty activation editor has wrong title or dirty state.' }
    $emptyToolbar = [ShotKitMultiSmoke]::Children($empty.Handle) | Where-Object { $_.Class -eq 'ShotKitRustEditorToolbar' }
    if ($null -eq $emptyToolbar) { throw 'Empty activation editor has no toolbar.' }
    Post $empty.Handle 0x0010
    [void](Wait-Value { (Get-Editors).Count -eq 0 } 'empty editor closed without save prompt')
    Post $hostWindow.Handle 0x0010
    if (-not $process.WaitForExit($TimeoutSeconds * 1000) -or $process.ExitCode -ne 0) { throw 'Clean application quit failed.' }
    Write-Output 'PASS: two startup documents, independent dirty/save/undo, failed-save retention, close/quit cancellation, per-window close, persistent tray host and visible empty activation.'
}
finally {
    if ($null -ne $process) {
        if (-not $process.HasExited) {
            if ($process.StartTime.ToUniversalTime().Ticks -ne $startTicks -or
                -not [StringComparer]::OrdinalIgnoreCase.Equals($process.MainModule.FileName,$ExePath)) { throw 'Refusing cleanup: process identity changed.' }
            $process.Kill()
            if (-not $process.WaitForExit(5000)) { throw "Cleanup failed; retaining $testRoot" }
        }
        $process.Dispose()
    }
    if (Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force }
}
