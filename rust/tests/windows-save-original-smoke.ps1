[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $StartupTimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'This smoke test requires an interactive ephemeral GitHub Actions Windows runner.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: refusing to use or change '$imageName'."
}

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitSaveSmoke {
    public sealed class Win { public IntPtr Handle; public int Pid; public string ClassName, Title; public bool Visible; }
    public sealed class Child { public IntPtr Handle; public int Id; public string ClassName, Text; public bool Visible; }
    public static class Native {
        delegate bool EnumProc(IntPtr h, IntPtr d);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr d);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr d);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
        [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
        [DllImport("user32.dll")] static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
        static extern IntPtr SendMessageText(IntPtr h, uint m, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
        static string Read(IntPtr h, bool cls) { var s=new StringBuilder(512); if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString(); }
        public static Win[] Windows(int wanted) { var a=new List<Win>(); EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)wanted)a.Add(new Win{Handle=h,Pid=(int)p,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
        public static Child[] Children(IntPtr parent) { var a=new List<Child>();EnumProc cb=(h,d)=>{a.Add(new Child{Handle=h,Id=GetDlgCtrlID(h),ClassName=Read(h,true),Text=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
        public static bool SetEditText(IntPtr h,string s) { IntPtr p=Marshal.StringToHGlobalUni(s);try{return SendMessageText(h,0x000C,IntPtr.Zero,p).ToInt64()!=0;}finally{Marshal.FreeHGlobal(p);} }
        public static void Click(IntPtr h) { SendMessageW(h,0x00F5,IntPtr.Zero,IntPtr.Zero); }
        public static void ControlDown() { keybd_event(0x11,0,0,UIntPtr.Zero); }
        public static void ControlUp() { keybd_event(0x11,0,0x0002,UIntPtr.Zero); }
        public static void ControlShiftS() {
            try {
                keybd_event(0x11,0,0,UIntPtr.Zero);
                keybd_event(0x10,0,0,UIntPtr.Zero);
                keybd_event(0x53,0,0,UIntPtr.Zero);
            }
            finally {
                keybd_event(0x53,0,0x0002,UIntPtr.Zero);
                keybd_event(0x10,0,0x0002,UIntPtr.Zero);
                keybd_event(0x11,0,0x0002,UIntPtr.Zero);
            }
        }
        public static void TapS() {
            try { keybd_event(0x53,0,0,UIntPtr.Zero); }
            finally { keybd_event(0x53,0,0x0002,UIntPtr.Zero); }
        }
    }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Save-Original-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA
$proc = $null; $failure = $null

function Owned-Windows { if ($null -eq $proc -or $proc.HasExited) { return @() }; @([ShotKitSaveSmoke.Native]::Windows($proc.Id)) }
function Wait-For([scriptblock]$Probe,[int]$Seconds,[string]$What) {
    $until=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $until) {
        if($null -ne $proc -and $proc.HasExited){throw "PROCESS_EXITED waiting for $What (code $($proc.ExitCode))."}
        $v=&$Probe; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100
    }
    throw "TIMEOUT waiting for $What."
}
function Button([IntPtr]$Parent,[string]$Label) {
    $b=[ShotKitSaveSmoke.Native]::Children($Parent) | Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and (($_.Text -replace '&','').Trim() -eq $Label)} | Select-Object -First 1
    if($null -eq $b){throw "BUTTON_NOT_FOUND: '$Label' in dialog '$([ShotKitSaveSmoke.Native]::Windows($proc.Id) | Where-Object {$_.Handle -eq $Parent} | Select-Object -ExpandProperty Title -First 1)'"}
    [ShotKitSaveSmoke.Native]::Click($b.Handle)
}
function Dialog-Button([IntPtr]$Parent,[int]$Id) {
    $b=[ShotKitSaveSmoke.Native]::Children($Parent) | Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and $_.Id -eq $Id} | Select-Object -First 1
    if($null -eq $b){throw "DIALOG_BUTTON_NOT_FOUND: control ID $Id"}
    [ShotKitSaveSmoke.Native]::Click($b.Handle)
}
function Hash-File([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash }
function Request-Save-Protection([IntPtr]$Main,[string]$Description) {
    [void][ShotKitSaveSmoke.Native]::SetForegroundWindow($Main)
    try {
        [ShotKitSaveSmoke.Native]::ControlDown()
        [ShotKitSaveSmoke.Native]::TapS()
        $prompt=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'Original schützen'} | Select-Object -First 1} 5 $Description
        return $prompt
    } finally {
        [ShotKitSaveSmoke.Native]::ControlUp()
    }
}
function New-Fixture([string]$Path,[string]$Format) {
    $bmp=[Drawing.Bitmap]::new(32,24,[Drawing.Imaging.PixelFormat]::Format24bppRgb)
    try {
        $g=[Drawing.Graphics]::FromImage($bmp)
        try {$g.Clear([Drawing.Color]::DarkCyan);$g.FillRectangle([Drawing.Brushes]::Orange,5,4,17,13)} finally {$g.Dispose()}
        $kind=if($Format -eq 'jpeg'){[Drawing.Imaging.ImageFormat]::Jpeg}else{[Drawing.Imaging.ImageFormat]::Gif}
        $bmp.Save($Path,$kind)
    } finally {$bmp.Dispose()}
}
function New-PngFixture([string]$Path,[Drawing.Color]$Background) {
    $bmp=[Drawing.Bitmap]::new(32,24,[Drawing.Imaging.PixelFormat]::Format24bppRgb)
    try {
        $g=[Drawing.Graphics]::FromImage($bmp)
        try {$g.Clear($Background);$g.FillRectangle([Drawing.Brushes]::Orange,5,4,17,13)} finally {$g.Dispose()}
        $bmp.Save($Path,[Drawing.Imaging.ImageFormat]::Png)
    } finally {$bmp.Dispose()}
}
function Save-As-Dialog([IntPtr]$Main,[string]$Description,[string]$FileName) {
    [void][ShotKitSaveSmoke.Native]::SetForegroundWindow($Main)
    [ShotKitSaveSmoke.Native]::ControlShiftS()
    $dialog=Wait-For {
        $windows=Owned-Windows
        $windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and @([ShotKitSaveSmoke.Native]::Children($_.Handle) | Where-Object {$_.ClassName -eq 'Edit'}).Count -gt 0} | Select-Object -First 1
    } 8 "$Description Save As dialog"
    $edit=[ShotKitSaveSmoke.Native]::Children($dialog.Handle) | Where-Object {$_.Visible -and $_.ClassName -eq 'Edit'} | Select-Object -First 1
    if($null -eq $edit){throw "$Description Save As filename control missing."}
    if(-not [ShotKitSaveSmoke.Native]::SetEditText($edit.Handle,$FileName)){throw "$Description could not set Save As filename."}
    Dialog-Button $dialog.Handle 1 # IDOK
}
function Run-Dotted-Png-Overwrite {
    $source=Join-Path $root 'dotted-name-source.png'
    $output=Join-Path $root 'Bericht v1.2.png'
    New-PngFixture $source ([Drawing.Color]::DarkCyan)
    New-PngFixture $output ([Drawing.Color]::DarkMagenta)
    $before=Hash-File $output
    $script:proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $source) -WorkingDirectory $root -PassThru
    $main=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'} | Select-Object -First 1} $StartupTimeoutSeconds 'dotted-name editor window'
    [void][ShotKitSaveSmoke.Native]::SetForegroundWindow($main.Handle)
    $welcome=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1} 12 'dotted-name startup notice'
    Button $welcome.Handle 'OK'
    $main=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'} | Select-Object -First 1} 10 'dotted-name loaded editor'

    Save-As-Dialog $main.Handle 'dotted-name decline' 'Bericht v1.2'
    $replace=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'Datei ersetzen'} | Select-Object -First 1} 8 'replacement confirmation after Save As closes'
    Dialog-Button $replace.Handle 7 # IDNO
    Start-Sleep -Milliseconds 250
    if((Hash-File $output) -cne $before){throw 'Existing Bericht v1.2.png changed after declining replacement.'}

    Save-As-Dialog $main.Handle 'dotted-name accept' 'Bericht v1.2'
    $replace=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'Datei ersetzen'} | Select-Object -First 1} 8 'second replacement confirmation after Save As closes'
    Dialog-Button $replace.Handle 6 # IDYES
    [void](Wait-For {(Hash-File $output) -cne $before} 10 'confirmed dotted-name PNG replacement')
    if((Get-Item -LiteralPath $output).Length -le 0){throw 'Confirmed Bericht v1.2.png replacement is empty.'}
    $image=[Drawing.Image]::FromFile($output)
    try {if($image.RawFormat.Guid -ne [Drawing.Imaging.ImageFormat]::Png.Guid){throw 'Confirmed dotted-name output is not a PNG.'}} finally {$image.Dispose()}
    Write-Output "PASS: Bericht v1.2 was saved as PNG; declining replacement preserved SHA256 $before and confirming replacement produced a valid PNG."
    $script:proc.Kill();[void]$script:proc.WaitForExit(5000);$script:proc=$null
}
function Run-Format([string]$Format) {
    $extension=if($Format -eq 'jpeg'){'jpg'}else{'gif'}
    $source=Join-Path $root "source.$extension"; $output=Join-Path $root "saved-as-$Format.png"
    New-Fixture $source $Format
    $before=Hash-File $source
    $script:proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $source) -WorkingDirectory $root -PassThru
    $main=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'} | Select-Object -First 1} $StartupTimeoutSeconds 'editor window'
    [void][ShotKitSaveSmoke.Native]::SetForegroundWindow($main.Handle)
    $welcome=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1} 12 'startup notice'
    Button $welcome.Handle 'OK'
    $main=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*'} | Select-Object -First 1} 10 'loaded editor'

    $protect=Request-Save-Protection $main.Handle "$Format overwrite-protection prompt"
    Dialog-Button $protect.Handle 7 # IDNO
    Start-Sleep -Milliseconds 250
    if((Hash-File $source) -cne $before){throw "$Format original changed after declining Save As."}

    $protect=Request-Save-Protection $main.Handle "$Format second overwrite-protection prompt"
    Dialog-Button $protect.Handle 6 # IDYES
    $dialog=Wait-For {
        $windows=Owned-Windows
        $windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and @([ShotKitSaveSmoke.Native]::Children($_.Handle) | Where-Object {$_.ClassName -eq 'Edit'}).Count -gt 0} | Select-Object -First 1
    } 8 "$Format Save As dialog"
    $edit=[ShotKitSaveSmoke.Native]::Children($dialog.Handle) | Where-Object {$_.Visible -and $_.ClassName -eq 'Edit'} | Select-Object -First 1
    if($null -eq $edit){throw "$Format Save As filename control missing."}
    if(-not [ShotKitSaveSmoke.Native]::SetEditText($edit.Handle,$output)){throw "$Format could not set Save As filename."}
    Dialog-Button $dialog.Handle 1 # IDOK
    [void](Wait-For {Test-Path -LiteralPath $output -PathType Leaf} 10 "$Format exported file")
    if((Get-Item -LiteralPath $output).Length -le 0){throw "$Format Save As output is empty."}
    if((Hash-File $source) -cne $before){throw "$Format original changed after Save As to another path."}
    Write-Output "PASS: $Format prompt cancellation and Save As preserved original SHA256 $before; export created at $output."
    $script:proc.Kill();[void]$script:proc.WaitForExit(5000);$script:proc=$null
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force | Out-Null
    $env:APPDATA=$appData; $env:LOCALAPPDATA=$localAppData
    Run-Format 'jpeg'
    Run-Format 'gif'
    Run-Dotted-Png-Overwrite
    Write-Output 'PASS: lossy JPEG and GIF originals remained byte-for-byte unchanged after both cancelling and exporting through Save As.'
}
catch { $failure=$_ }
finally {
    if($null -ne $proc -and -not $proc.HasExited){try{$proc.Kill();[void]$proc.WaitForExit(5000)}catch{}}
    if($oldAppData){$env:APPDATA=$oldAppData}else{Remove-Item Env:APPDATA -ErrorAction SilentlyContinue}
    if($oldLocalAppData){$env:LOCALAPPDATA=$oldLocalAppData}else{Remove-Item Env:LOCALAPPDATA -ErrorAction SilentlyContinue}
    if($null -eq $proc -or $proc.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}
