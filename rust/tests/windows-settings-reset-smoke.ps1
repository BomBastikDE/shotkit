[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $TimeoutSeconds = 30
)

# Real app startup/reset/save coverage. No captures, clipboard access or registry
# writes. Native tray selection uses the existing output-matrix pattern, so this
# fixture deliberately has NO owner/local-session opt-in.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
. (Join-Path $PSScriptRoot 'shared-log-reader.ps1')
$fixtureClock = [Diagnostics.Stopwatch]::StartNew()
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 is required.' }
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'EPHEMERAL_CI_ONLY: settings reset UI smoke requires an interactive GitHub Actions Windows runner.'
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
if ([IO.Path]::GetExtension($ExePath) -ine '.exe') { throw 'A ShotKit executable is required.' }
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitSettingsResetSmoke {
    public sealed class WindowInfo { public IntPtr Handle; public string ClassName; public string Title; public bool Visible; }
    public struct Point { public int X; public int Y; public Point(int x, int y) { X=x; Y=y; } }
    public static class Native {
        private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
        private delegate bool EnumChildProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll", SetLastError=true)] static extern bool CloseDesktop(IntPtr desktop);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder text, uint length, out uint needed);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
        public static bool WindowVisible(IntPtr hwnd) { return IsWindowVisible(hwnd); }
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
        [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out UIntPtr result);
        public static bool UiResponsive(IntPtr hwnd) { UIntPtr result; return SendMessageTimeoutW(hwnd, 0, IntPtr.Zero, IntPtr.Zero, 2, 5000, out result)!=IntPtr.Zero; }
        public static bool CloseDialog(IntPtr hwnd) { return PostMessageW(hwnd,0x0010,IntPtr.Zero,IntPtr.Zero); }
        public struct Rect { public int Left, Top, Right, Bottom; }
        [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
        [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr m, int pos);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern uint GetMenuStringW(IntPtr m, uint pos, StringBuilder text, int max, uint flags);
        [DllImport("user32.dll")] static extern IntPtr GetSubMenu(IntPtr m, int pos);
        [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr m, uint id, uint flags);
        [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint pos, out Rect r);
        public static IntPtr PopupMenu(IntPtr popup) {
            UIntPtr result;
            if(SendMessageTimeoutW(popup,0x01e1,IntPtr.Zero,IntPtr.Zero,2,2000,out result)==IntPtr.Zero)
                throw new InvalidOperationException("Popup menu query failed or timed out.");
            return new IntPtr(unchecked((long)result.ToUInt64()));
        }
        static int FindItem(IntPtr m, string expected, uint? id) {
            int found=-1, count=GetMenuItemCount(m);
            if(count<0) throw new InvalidOperationException("Cannot inspect menu.");
            for(int i=0;i<count;i++) {
                var s=new StringBuilder(256); GetMenuStringW(m,(uint)i,s,s.Capacity,0x400);
                if(s.ToString()==expected && (!id.HasValue || GetMenuItemID(m,i)==id.Value)) {
                    if(found>=0) throw new InvalidOperationException("Duplicate menu entry: "+expected);
                    found=i;
                }
            }
            if(found<0) throw new InvalidOperationException("Missing menu entry: "+expected);
            return found;
        }
        static void MoveToItem(IntPtr popup, IntPtr m, int pos, int expectedPid) {
            uint pid; GetWindowThreadProcessId(popup,out pid);
            if(pid!=(uint)expectedPid || !IsWindow(popup) || !IsWindowVisible(popup) || PopupMenu(popup)!=m)
                throw new InvalidOperationException("Popup no longer identifies the expected visible menu.");
            Rect r; if(!GetMenuItemRect(IntPtr.Zero,m,(uint)pos,out r) || r.Right<=r.Left || r.Bottom<=r.Top)
                throw new InvalidOperationException("Menu entry has no visible screen rectangle.");
            int x=r.Left+(r.Right-r.Left)/2, y=r.Top+(r.Bottom-r.Top)/2;
            if(!SetCursorPos(x,y)) throw new InvalidOperationException("Cannot move cursor to menu entry.");
            if(WindowFromPoint(new Point(x,y))!=popup)
                throw new InvalidOperationException("Menu entry is obscured or no longer belongs to the expected popup.");
        }
        public static IntPtr OpenSettingsSubmenu(IntPtr popup, int expectedPid) {
            IntPtr m=PopupMenu(popup); if(m==IntPtr.Zero) throw new InvalidOperationException("No root menu.");
            int pos=FindItem(m,"Einstellungen",null); IntPtr sub=GetSubMenu(m,pos);
            if(sub==IntPtr.Zero) throw new InvalidOperationException("No settings submenu.");
            MoveToItem(popup,m,pos,expectedPid); return sub;
        }
        public static void AssertSettingChecked(IntPtr popup, IntPtr expectedMenu, uint id, string label, bool expectedChecked, int expectedPid) {
            uint pid; GetWindowThreadProcessId(popup,out pid);
            if(pid!=(uint)expectedPid || !IsWindow(popup) || !IsWindowVisible(popup) || PopupMenu(popup)!=expectedMenu)
                throw new InvalidOperationException("Settings popup no longer belongs to the expected process/menu.");
            int pos=FindItem(expectedMenu,label,id);
            uint state=GetMenuState(expectedMenu,id,0);
            if(state==0xffffffff || (state & 3)!=0 || ((state & 8)!=0)!=expectedChecked)
                throw new InvalidOperationException("Unexpected live menu checked/enabled state for "+label+": "+state);
            Rect rect;
            if(!GetMenuItemRect(IntPtr.Zero,expectedMenu,(uint)pos,out rect) || rect.Right<=rect.Left || rect.Bottom<=rect.Top)
                throw new InvalidOperationException("Verified live preference is not displayed in the settings submenu.");
        }
        public static string ClickSetting(IntPtr popup, IntPtr expectedMenu, uint id, string label, bool checkedBefore, int expectedPid) {
            IntPtr m=PopupMenu(popup); if(m!=expectedMenu) throw new InvalidOperationException("Wrong submenu.");
            int pos=FindItem(m,label,id); uint state=GetMenuState(m,id,0);
            if(state==0xffffffff || ((state & 8)!=0)!=checkedBefore)
                throw new InvalidOperationException("Unexpected checked state for "+label+": "+state);
            MoveToItem(popup,m,pos,expectedPid);
            Rect r; Point cursor;
            bool hasRect=GetMenuItemRect(IntPtr.Zero,m,(uint)pos,out r), hasCursor=GetCursorPos(out cursor);
            string diagnostic=String.Format("id={0} position={1} state={2} popup={3} rectAvailable={4} rect=({5},{6},{7},{8}) cursorAvailable={9} cursor=({10},{11}) windowAtCursor={12} foreground={13}",
                id,pos,state,popup,hasRect,r.Left,r.Top,r.Right,r.Bottom,hasCursor,cursor.X,cursor.Y,
                hasCursor?WindowFromPoint(cursor):IntPtr.Zero,GetForegroundWindow());
            try { mouse_event(2,0,0,0,UIntPtr.Zero); }
            finally { mouse_event(4,0,0,0,UIntPtr.Zero); }
            return diagnostic;
        }
        public static string MenuAfterClick(IntPtr root, IntPtr child) {
            Point cursor; bool hasCursor=GetCursorPos(out cursor);
            return String.Format("rootExists={0} rootVisible={1} childExists={2} childVisible={3} cursorAvailable={4} cursor=({5},{6}) windowAtCursor={7} foreground={8}",
                IsWindow(root),IsWindowVisible(root),IsWindow(child),IsWindowVisible(child),hasCursor,cursor.X,cursor.Y,
                hasCursor?WindowFromPoint(cursor):IntPtr.Zero,GetForegroundWindow());
        }
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        public static string InputDesktopName() {
            IntPtr d=OpenInputDesktop(0,false,1); if(d==IntPtr.Zero)return null;
            try { var s=new StringBuilder(256); uint n; return GetUserObjectInformationW(d,2,s,(uint)(s.Capacity*2),out n)?s.ToString():null; }
            finally { CloseDesktop(d); }
        }
        static string Text(IntPtr h) { var s=new StringBuilder(4096); GetWindowTextW(h,s,s.Capacity); return s.ToString(); }
        public static string WindowText(IntPtr h) {
            var s=new StringBuilder(Text(h)); EnumChildWindows(h,delegate(IntPtr c,IntPtr d){string t=Text(c);if(!String.IsNullOrWhiteSpace(t))s.Append(" ").Append(t);return true;},IntPtr.Zero);return s.ToString();
        }
        public static WindowInfo[] Windows(int pid) {
            var list=new List<WindowInfo>(); EnumProc cb=delegate(IntPtr h,IntPtr d){uint p;GetWindowThreadProcessId(h,out p);if(pid!=0 && p!=(uint)pid)return true;
                var c=new StringBuilder(256);GetClassNameW(h,c,c.Capacity);list.Add(new WindowInfo{Handle=h,ClassName=c.ToString(),Title=Text(h),Visible=IsWindowVisible(h)});return true;};
            EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return list.ToArray();
        }


        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern Microsoft.Win32.SafeHandles.SafeFileHandle CreateFileW(string path, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
        [DllImport("kernel32.dll", SetLastError=true)]
        static extern bool SetFileInformationByHandle(Microsoft.Win32.SafeHandles.SafeFileHandle file, int infoClass, ref byte info, uint size);
        public static Microsoft.Win32.SafeHandles.SafeFileHandle HoldRoot(string path) {
            // DELETE access + no FILE_SHARE_DELETE pins this exact generated
            // directory against rename/replacement for the whole fixture.
            var h=CreateFileW(path,0x00010080,3,IntPtr.Zero,3,0x02200000,IntPtr.Zero);
            if(h.IsInvalid) { int e=Marshal.GetLastWin32Error();h.Dispose();throw new System.ComponentModel.Win32Exception(e,"Cannot pin generated root"); }
            try { Attributes(h); return h; } catch { h.Dispose(); throw; }
        }
        [StructLayout(LayoutKind.Sequential)] struct TagInfo { public uint Attributes, Tag; }
        [DllImport("kernel32.dll", SetLastError=true)]
        static extern bool GetFileInformationByHandleEx(Microsoft.Win32.SafeHandles.SafeFileHandle file, int infoClass, out TagInfo info, uint size);
        public static uint Attributes(Microsoft.Win32.SafeHandles.SafeFileHandle entry) {
            TagInfo info;
            if(!GetFileInformationByHandleEx(entry,9,out info,8))
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(),"Cannot inspect held cleanup object");
            if((info.Attributes & 0x400)!=0) throw new InvalidOperationException("Reparse point in cleanup; refusing traversal");
            return info.Attributes;
        }
        public static void DeleteHeldRoot(Microsoft.Win32.SafeHandles.SafeFileHandle root) {
            byte remove=1;
            if(!SetFileInformationByHandle(root,4,ref remove,1))
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(),"Cannot delete exact held root");
        }
        public sealed class ControlInfo {
            public IntPtr Handle, Parent, Owner;
            public uint Pid, ThreadId, Style;
            public int Id;
            public string ClassName, Text;
            public bool Exists, Visible, Enabled;
        }
        [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr hwnd);
        [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr hwnd, uint command);
        [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr dialog, int id);
        [DllImport("user32.dll")] static extern int GetWindowLongW(IntPtr hwnd, int index);
        [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr hwnd);
        [DllImport("user32.dll")] static extern bool IsChild(IntPtr parent, IntPtr child);
        public static ControlInfo InspectControl(IntPtr hwnd) {
            uint pid; uint thread=GetWindowThreadProcessId(hwnd,out pid);
            var cls=new StringBuilder(256); GetClassNameW(hwnd,cls,cls.Capacity);
            return new ControlInfo {
                Handle=hwnd, Parent=GetParent(hwnd), Owner=GetWindow(hwnd,4), Pid=pid, ThreadId=thread, Style=unchecked((uint)GetWindowLongW(hwnd,-16)), Id=GetDlgCtrlID(hwnd),
                ClassName=cls.ToString(), Text=Text(hwnd), Exists=IsWindow(hwnd),
                Visible=IsWindowVisible(hwnd), Enabled=IsWindowEnabled(hwnd)
            };
        }
        public static ControlInfo[] Children(IntPtr dialog) {
            // Include hidden controls and every class for failure diagnostics.
            var list=new List<ControlInfo>(); EnumChildProc cb=delegate(IntPtr h,IntPtr d){
                list.Add(InspectControl(h)); return true;
            };
            EnumChildWindows(dialog,cb,IntPtr.Zero); GC.KeepAlive(cb); return list.ToArray();
        }
        public sealed class OkDialogInfo {
            public ControlInfo Dialog, Owner, Button;
            public int DefaultId;
            public IntPtr DefaultHandle;
        }
        public static OkDialogInfo RequireOkDialog(IntPtr dialog, uint expectedPid, string expectedTitle, uint timeoutMs) {
            var box=InspectControl(dialog);
            if(!box.Exists || !box.Visible || !box.Enabled || box.Pid!=expectedPid ||
               box.ClassName!="#32770" || box.Text!=expectedTitle || box.Owner==IntPtr.Zero || box.ThreadId==0)
                throw new InvalidOperationException("Expected owned enabled native OK dialog is not ready.");
            var owner=InspectControl(box.Owner);
            if(!owner.Exists || owner.Enabled || owner.Pid!=expectedPid ||
               owner.ClassName!="ShotKitRustPrototype" || owner.ThreadId!=box.ThreadId)
                throw new InvalidOperationException("OK dialog must disable its own primary owner on the same UI thread.");
            var visibleButtons=new List<ControlInfo>();
            foreach(var candidate in Children(dialog))
                if(candidate.Visible && candidate.ClassName=="Button") visibleButtons.Add(candidate);
            if(visibleButtons.Count!=1)
                throw new InvalidOperationException("OK-only dialog must have exactly one visible native Button; count="+visibleButtons.Count);
            var button=visibleButtons[0];
            // GetDlgCtrlID describes a child control, not MessageBoxW's result.
            // CI 42333349379 observed the sole OK control with IDCANCEL (2).
            // Explicitly admit only IDOK (1) or IDCANCEL (2), together with the
            // complete OK-only/modal/default contract; never accept an arbitrary ID.
            if(!button.Exists || !button.Enabled || button.Pid!=expectedPid || button.ThreadId!=box.ThreadId ||
               button.Parent!=dialog || !IsChild(dialog,button.Handle) ||
               (button.Id!=1 && button.Id!=2) || button.Text.Replace("&", "").Trim()!="OK" ||
               (button.Style & 0x0f)!=1) // BS_TYPEMASK / BS_DEFPUSHBUTTON
                throw new InvalidOperationException("Sole visible control must be an enabled owned default OK pushbutton with control ID 1 or 2.");
            int defaultId=DefaultButtonWithTimeout(dialog,timeoutMs);
            IntPtr defaultHandle=GetDlgItem(dialog,defaultId);
            // CI 42334705405 observed DM_GETDEFID=1 with the sole styled
            // default OK control at ID 2. Check both observed native values;
            // GetDlgItem(defaultId) is diagnostic, not an HWND equivalence proof.
            if(defaultId!=1 || (button.Id!=1 && button.Id!=2))
                throw new InvalidOperationException("Unexpected OK dialog default/control IDs; defaultId="+defaultId+" controlId="+button.Id);
            return new OkDialogInfo { Dialog=box, Owner=owner, Button=button, DefaultId=defaultId, DefaultHandle=defaultHandle };
        }
        public static void ClickOwnedOkButton(IntPtr dialog, IntPtr button, uint expectedPid, string expectedTitle) {
            // Re-enumerate ALL visible buttons and recheck the exact OK-only,
            // owner-modal and default contract directly before BM_CLICK.
            var actual=RequireOkDialog(dialog,expectedPid,expectedTitle,500);
            if(actual.Button.Handle!=button)
                throw new InvalidOperationException("The unique default OK button HWND changed before BM_CLICK.");
            // Native checks and input remain separate Win32 operations. The UI
            // can change between them; closure and app/file postconditions are
            // required as well. A child ID is not a MessageBoxW return value.
            Click(button);
        }
        public static void ClickOwnedButton(IntPtr dialog, IntPtr button, int id, uint expectedPid, string expectedTitle) {
            // Revalidate the exact native ID and HWND immediately before BM_CLICK.
            // Never substitute another ID, caption match, WM_COMMAND or WM_CLOSE.
            var parent=InspectControl(dialog); var child=InspectControl(button);
            if(!parent.Exists || !parent.Visible || !parent.Enabled || parent.Pid!=expectedPid ||
               parent.ClassName!="#32770" || parent.Text!=expectedTitle ||
               !child.Exists || !child.Visible || !child.Enabled || child.Pid!=expectedPid ||
               child.ClassName!="Button" || child.Id!=id || !IsChild(dialog,button))
                throw new InvalidOperationException("Expected owned visible enabled native dialog/button changed before BM_CLICK.");
            int matchingVisibleButtons=0; IntPtr uniqueHandle=IntPtr.Zero;
            foreach(var candidate in Children(dialog)) {
                if(candidate.Visible && candidate.ClassName=="Button" && candidate.Id==id) {
                    matchingVisibleButtons++; uniqueHandle=candidate.Handle;
                }
            }
            if(matchingVisibleButtons!=1 || uniqueHandle!=button)
                throw new InvalidOperationException("Expected exactly one matching visible native button with the selected HWND immediately before BM_CLICK; count="+matchingVisibleButtons);
            // Final enumeration and BM_CLICK are separate Win32 operations.
            // The UI thread can still change controls between these calls;
            // this guard does not make observation and input atomic. The app's
            // dialog result and filesystem/state postconditions remain required.
            Click(button);
        }
        public static void Click(IntPtr button) {
            // API success is SendMessageTimeoutW's nonzero return. Its out
            // result belongs to BM_CLICK and is NOT MessageBoxW's return value.
            UIntPtr result;
            if(SendMessageTimeoutW(button,0x00f5,IntPtr.Zero,IntPtr.Zero,2,3000,out result)==IntPtr.Zero)
                throw new InvalidOperationException("Button click timed out.");
        }
        public static int DefaultButton(IntPtr dialog) {
            return DefaultButtonWithTimeout(dialog,3000);
        }
        static int DefaultButtonWithTimeout(IntPtr dialog, uint timeoutMs) {
            UIntPtr result;
            if(SendMessageTimeoutW(dialog,0x0400,IntPtr.Zero,IntPtr.Zero,2,timeoutMs,out result)==IntPtr.Zero)
                throw new InvalidOperationException("DM_GETDEFID timed out.");
            uint value=unchecked((uint)result.ToUInt64());
            if((value>>16)!=0x534b) throw new InvalidOperationException("Dialog has no default button.");
            return (int)(value&0xffff);
        }
    }
}
'@

if ([ShotKitSettingsResetSmoke.Native]::InputDesktopName() -cne 'Default') {
    throw 'INTERACTIVE_DEFAULT_DESKTOP_REQUIRED'
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Settings-Reset-Smoke-' + [Guid]::NewGuid().ToString('N'))
$rootCreated = $false
$rootHandle = $null
$processHandle = $null
$processStartTicks = 0
$ownedProcessId = 0
$process = $null
$failure = $null
$cleanupFailure = $null
$uiCase = '<not-started>'
$uiRun = '<not-started>'
$interactionSequence = 0
# Unknown member makes a complete, unambiguous-v1 JSON snapshot rejected.
# Retain whitespace/UTF-8 bytes to catch parse-and-reserialize "backups".
$rejected = [Text.Encoding]::UTF8.GetBytes("{`r`n  `"version`": 1, `"unknown_reset_probe`": `"original-ä`"`r`n}`r`n")
$pending = [Text.Encoding]::UTF8.GetBytes('interrupted-reset-smoke-marker')

function Assert-True([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw "ASSERTION_FAILED: $Message" }
}
function Convert-ControlDiagnostic($Control) {
    return [ordered]@{
        hwnd = ('0x{0:X}' -f $Control.Handle.ToInt64())
        parent = ('0x{0:X}' -f $Control.Parent.ToInt64())
        owner = ('0x{0:X}' -f $Control.Owner.ToInt64())
        pid = $Control.Pid; threadId = $Control.ThreadId; class = $Control.ClassName; id = $Control.Id
        style = ('0x{0:X}' -f $Control.Style)
        exists = $Control.Exists; visible = $Control.Visible; enabled = $Control.Enabled
        text = $Control.Text
    }
}
function Write-UiSnapshot([string] $Stage, $Dialog = $null, [int] $ExpectedId = 0, $Details = $null) {
    $entry = [ordered]@{
        utc = [DateTime]::UtcNow.ToString('o'); elapsedMs = $fixtureClock.ElapsedMilliseconds
        case = $script:uiCase; run = $script:uiRun; interaction = $script:interactionSequence
        stage = $Stage; ownedPid = $script:ownedProcessId; processStartTicks = $script:processStartTicks
        expectedButtonId = $ExpectedId; details = $Details
    }
    if ($null -ne $Dialog) {
        try {
            $entry.dialog = Convert-ControlDiagnostic ([ShotKitSettingsResetSmoke.Native]::InspectControl($Dialog.Handle))
            $entry.children = @([ShotKitSettingsResetSmoke.Native]::Children($Dialog.Handle) | ForEach-Object { Convert-ControlDiagnostic $_ })
        } catch { $entry.snapshotError = $_.Exception.Message }
    }
    # Host output stays out of helper return values and survives temp cleanup.
    Write-Host ('SETTINGS_RESET_UI_DIAGNOSTIC: ' + ($entry | ConvertTo-Json -Depth 6 -Compress))
}
function Assert-CleanRunner {
    $names = @([IO.Path]::GetFileNameWithoutExtension($ExePath), 'ShotKit', 'shotkit-rust-prototype') | Select-Object -Unique
    foreach ($name in $names) {
        if (@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0) { throw "PREEXISTING_PROCESS: '$name'; no existing process was touched." }
    }
    if (@([ShotKitSettingsResetSmoke.Native]::Windows(0) | Where-Object { $_.ClassName -in @('ShotKitRustPrototype', 'ShotKitRustEditor') }).Count -gt 0) {
        throw 'PREEXISTING_SHOTKIT_WINDOW: refusing to affect a renamed or older ShotKit instance.'
    }
}
function Wait-Value([scriptblock] $Probe, [string] $What) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($process -and $process.HasExited) { throw "PROCESS_EXITED waiting for $What (code $($process.ExitCode))" }
        $value = & $Probe
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    $windows = @([ShotKitSettingsResetSmoke.Native]::Windows($process.Id) | ForEach-Object { "$($_.ClassName):$($_.Title)" }) -join '; '
    throw "TIMEOUT waiting for $What; owned windows: $windows"
}
function Get-Dialog([string] $Title) {
    [ShotKitSettingsResetSmoke.Native]::Windows($process.Id) | Where-Object {
        $_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -ceq $Title
    } | Select-Object -First 1
}
function Wait-Dialog([string] $Title, [string] $Body) {
    Write-UiSnapshot 'dialog_wait_begin' -Details @{ expectedTitle = $Title; expectedBody = $Body }
    $dialog = Wait-Value { Get-Dialog $Title } "dialog '$Title'"
    Write-UiSnapshot 'dialog_observed' $dialog -Details @{ expectedTitle = $Title; expectedBody = $Body }
    Assert-True ([ShotKitSettingsResetSmoke.Native]::WindowText($dialog.Handle).Contains($Body)) "Expected dialog body '$Body'"
    $dialog | Add-Member -NotePropertyName ExpectedBody -NotePropertyValue $Body
    return $dialog
}
function Click-DialogButton($Dialog, [int] $Id) {
    Assert-True ($Id -in @(6, 7)) 'Exact-ID dialog helper is restricted to Yes/No (6/7); OK-only dialogs require their semantic contract'
    $script:interactionSequence++
    $readyClock = [Diagnostics.Stopwatch]::StartNew()
    $attempt = 0
    $button = $null
    $stableHandle = [IntPtr]::Zero
    $matches = @()
    Write-UiSnapshot 'button_wait_begin' $Dialog $Id -Details @{ readyLimitMs = 5000; pollMs = 50 }
    try {
        # A visible message box and its text do not establish that its expected
        # button is visible/enabled yet. Poll the SAME dialog and exact native
        # ID for at most five seconds; require a stable handle on two samples.
        while ($readyClock.ElapsedMilliseconds -lt 5000) {
            Assert-OwnedProcess
            Assert-True (-not $process.HasExited) 'Owned app exited before button readiness'
            $attempt++
            $dialogInfo = [ShotKitSettingsResetSmoke.Native]::InspectControl($Dialog.Handle)
            Assert-True ($dialogInfo.Exists -and $dialogInfo.Pid -eq $ownedProcessId -and
                $dialogInfo.ClassName -ceq '#32770' -and $dialogInfo.Text -ceq $Dialog.Title) 'Expected dialog identity changed during button readiness'
            $children = @([ShotKitSettingsResetSmoke.Native]::Children($Dialog.Handle))
            $matches = @($children | Where-Object { $_.Visible -and $_.ClassName -ceq 'Button' -and $_.Id -eq $Id })
            $ready = $dialogInfo.Visible -and $dialogInfo.Enabled -and $matches.Count -eq 1 -and
                $matches[0].Exists -and $matches[0].Enabled -and $matches[0].Pid -eq $ownedProcessId
            if ($ready) {
                if ($stableHandle -eq $matches[0].Handle) { $button = $matches[0]; break }
                $stableHandle = $matches[0].Handle
            } else { $stableHandle = [IntPtr]::Zero }
            Start-Sleep -Milliseconds 50
        }
        Assert-True ($null -ne $button -and $matches.Count -eq 1) "BUTTON_READY_TIMEOUT: Exactly one visible enabled Button with native ID $Id required; attempts=$attempt elapsedMs=$($readyClock.ElapsedMilliseconds) visibleMatches=$($matches.Count)"
        Assert-True ([ShotKitSettingsResetSmoke.Native]::WindowText($Dialog.Handle).Contains($Dialog.ExpectedBody)) 'Expected dialog body changed before button click'
        Write-UiSnapshot 'button_click_begin' $Dialog $Id -Details @{ attempts = $attempt; readyElapsedMs = $readyClock.ElapsedMilliseconds; selectedHwnd = ('0x{0:X}' -f $button.Handle.ToInt64()) }
        [ShotKitSettingsResetSmoke.Native]::ClickOwnedButton($Dialog.Handle, $button.Handle, $Id, $ownedProcessId, $Dialog.Title)
        [void](Wait-Value { -not [ShotKitSettingsResetSmoke.Native]::IsWindow($Dialog.Handle) } 'dialog dismissal')
        Write-UiSnapshot 'button_click_complete' -ExpectedId $Id -Details @{ title = $Dialog.Title; attempts = $attempt; interactionElapsedMs = $readyClock.ElapsedMilliseconds }
    } catch {
        Write-UiSnapshot 'button_interaction_failed' $Dialog $Id -Details @{ attempts = $attempt; interactionElapsedMs = $readyClock.ElapsedMilliseconds; error = $_.Exception.Message; scriptStack = $_.ScriptStackTrace }
        throw
    }
}
function Click-OkDialog($Dialog) {
    $script:interactionSequence++
    $readyClock = [Diagnostics.Stopwatch]::StartNew()
    $attempt = 0
    $selected = $null
    $stableControlId = 0
    $stableHandle = [IntPtr]::Zero
    $lastReason = '<not-sampled>'
    Write-UiSnapshot 'ok_button_wait_begin' $Dialog -Details @{ readyLimitMs = 5000; permittedControlIds = @(1, 2); requiredCaption = 'OK' }
    try {
        while ($readyClock.ElapsedMilliseconds -lt 5000) {
            Assert-OwnedProcess
            Assert-True (-not $process.HasExited) 'Owned app exited before OK button readiness'
            $attempt++
            $remainingMs = [Math]::Max(1, [Math]::Min(500, 5000 - $readyClock.ElapsedMilliseconds))
            $candidate = $null
            try {
                $candidate = [ShotKitSettingsResetSmoke.Native]::RequireOkDialog($Dialog.Handle, $ownedProcessId, $Dialog.Title, [uint32]$remainingMs)
            } catch { $lastReason = $_.Exception.Message }
            if ($null -ne $candidate) {
                $lastReason = '<contract met>'
                if ($stableHandle -eq $candidate.Button.Handle -and $stableControlId -eq $candidate.Button.Id) { $selected = $candidate; break }
                $stableHandle = $candidate.Button.Handle
                $stableControlId = $candidate.Button.Id
            } else { $stableHandle = [IntPtr]::Zero; $stableControlId = 0 }
            Start-Sleep -Milliseconds 50
        }
        Assert-True ($null -ne $selected) "OK_BUTTON_READY_TIMEOUT: exact OK-only/modal/default contract not met; attempts=$attempt elapsedMs=$($readyClock.ElapsedMilliseconds) reason=$lastReason"
        Assert-True ([ShotKitSettingsResetSmoke.Native]::WindowText($Dialog.Handle).Contains($Dialog.ExpectedBody)) 'Expected OK dialog body changed before click'
        Write-UiSnapshot 'ok_button_click_begin' $Dialog $selected.Button.Id -Details @{
            attempts = $attempt; readyElapsedMs = $readyClock.ElapsedMilliseconds
            selectedHwnd = ('0x{0:X}' -f $selected.Button.Handle.ToInt64())
            nativeDefaultId = $selected.DefaultId; nativeDefaultHwnd = ('0x{0:X}' -f $selected.DefaultHandle.ToInt64())
            modalOwner = (Convert-ControlDiagnostic $selected.Owner)
        }
        [ShotKitSettingsResetSmoke.Native]::ClickOwnedOkButton($Dialog.Handle, $selected.Button.Handle, $ownedProcessId, $Dialog.Title)
        [void](Wait-Value { -not [ShotKitSettingsResetSmoke.Native]::IsWindow($Dialog.Handle) } 'actual OK dialog destruction')
        # A following startup dialog may immediately disable the same owner.
        # Terminal acknowledgements separately require the primary to become
        # enabled; all paths retain their next-dialog/app/file assertions.
        Write-UiSnapshot 'ok_button_click_complete' -ExpectedId $selected.Button.Id -Details @{ title = $Dialog.Title; interactionElapsedMs = $readyClock.ElapsedMilliseconds }
    } catch {
        Write-UiSnapshot 'ok_button_interaction_failed' $Dialog -Details @{ attempts = $attempt; interactionElapsedMs = $readyClock.ElapsedMilliseconds; lastReadinessFailure = $lastReason; error = $_.Exception.Message; scriptStack = $_.ScriptStackTrace }
        throw
    }
}
function Assert-PrimaryEnabled {
    [void](Wait-Value {
        $current = [ShotKitSettingsResetSmoke.Native]::InspectControl($primary.Handle)
        $current.Exists -and $current.Pid -eq $ownedProcessId -and $current.ClassName -ceq 'ShotKitRustPrototype' -and $current.Enabled
    } 'own primary enabled after terminal modal acknowledgement')
}
function Assert-Bytes([string] $Path, [byte[]] $Expected) {
    Assert-True ([IO.File]::Exists($Path)) "Expected file '$([IO.Path]::GetFileName($Path))'"
    Assert-True ([Convert]::ToBase64String([IO.File]::ReadAllBytes($Path)) -ceq [Convert]::ToBase64String($Expected)) "Original bytes changed in '$([IO.Path]::GetFileName($Path))'"
}
function Assert-Defaults([string] $Path, [bool] $OpenEditor = $true) {
    $s = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    Assert-True ($s.version -eq 1 -and $null -eq $s.save_folder -and $s.open_editor -ceq $OpenEditor -and
        $s.copy_to_clipboard -ceq $false -and $s.exclude_from_clipboard_history -ceq $true -and
        $s.auto_save -ceq $false -and $s.scroll_direction -ceq 'down') 'Usable safe default settings required'
}
function Read-Events {
    return @(Read-SharedLines -Path $script:diagnostics)
}
function Assert-Event([string] $Event) {
    Assert-True ((Read-Events) -ccontains $Event) "Missing app diagnostic '$Event'"
}
function Start-CaseProcess([string] $CaseRoot, [string] $RunName) {
    $script:uiCase = [IO.Path]::GetFileName($CaseRoot)
    $script:uiRun = $RunName
    $script:ownedProcessId = 0
    $script:processStartTicks = 0
    Write-UiSnapshot 'process_start_begin'
    Assert-CleanRunner
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $ExePath
    $start.WorkingDirectory = $CaseRoot
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.EnvironmentVariables['APPDATA'] = Join-Path $CaseRoot 'appdata'
    $start.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $CaseRoot 'localappdata'
    $tempPath = Join-Path $CaseRoot 'temp'
    [void][IO.Directory]::CreateDirectory($tempPath)
    $start.EnvironmentVariables['TEMP'] = $tempPath
    $start.EnvironmentVariables['TMP'] = $tempPath
    $script:diagnostics = Join-Path $tempPath ($RunName + '.log')
    $start.EnvironmentVariables['SHOTKIT_UIA_DIAGNOSTICS_PATH'] = $script:diagnostics
    $start.ArgumentList.Add((Join-Path $root 'fixture.png'))
    $script:process = [Diagnostics.Process]::new()
    $script:process.StartInfo = $start
    Assert-True ($script:process.Start()) 'App startup failed'
    # Keep the original OS handle open so later exit/kill operations refer to
    # the created process object even if Windows reuses its numeric PID.
    $script:processHandle = $script:process.SafeHandle
    $script:ownedProcessId = $script:process.Id
    $script:processStartTicks = $script:process.StartTime.ToUniversalTime().Ticks
    Write-UiSnapshot 'process_started'
}
function Complete-Startup {
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { [ShotKitSettingsResetSmoke.Native]::Windows($process.Id) }
    $script:primary = Wait-Value {
        [ShotKitSettingsResetSmoke.Native]::Windows($process.Id) | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    } 'owned primary window'
    $editor = Wait-Value {
        [ShotKitSettingsResetSmoke.Native]::Windows($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' } | Select-Object -First 1
    } 'working editor for imported local PNG'
    Assert-PrimaryEnabled
    Assert-True ([ShotKitSettingsResetSmoke.Native]::UiResponsive($editor.Handle)) 'Editor must respond after settings load'
}
function Assert-OwnedProcess {
    Assert-True ($null -ne $processHandle -and -not $processHandle.IsClosed -and -not $processHandle.IsInvalid) 'Original process handle is unavailable'
    Assert-True ($process.Id -eq $ownedProcessId -and $process.StartTime.ToUniversalTime().Ticks -eq $processStartTicks) 'Process PID/start identity changed'
}
function Close-CaseProcess {
    Write-UiSnapshot 'process_close_begin'
    Assert-OwnedProcess
    Assert-True ([ShotKitSettingsResetSmoke.Native]::PostMessageW($primary.Handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) 'App close post failed'
    Assert-True ($process.WaitForExit($TimeoutSeconds * 1000)) 'App did not exit normally'
    Assert-True ($process.ExitCode -eq 0) 'App exited unsuccessfully'
    $process.Dispose()
    $script:process = $null
    $script:processHandle = $null
}
function Open-SettingsMenu {
    Write-UiSnapshot 'tray_menu_open_begin'
    Assert-OwnedProcess
    # Reuse the output-matrix smoke's validated real tray menu path. Menu
    # construction reads CI autostart status; callers select only preference ID
    # 101, or inspect it and cancel without selecting any command.
    Assert-True ([ShotKitSettingsResetSmoke.Native]::PostMessageW($primary.Handle, 0x8001, [IntPtr]1, [IntPtr]0x0205)) 'Tray callback post failed'
    $popup = Wait-Value {
        [ShotKitSettingsResetSmoke.Native]::Windows($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq '#32768' } | Select-Object -First 1
    } 'owned tray popup'
    $submenu = [ShotKitSettingsResetSmoke.Native]::OpenSettingsSubmenu($popup.Handle, $process.Id)
    $child = Wait-Value {
        [ShotKitSettingsResetSmoke.Native]::Windows($process.Id) | Where-Object {
            $_.Visible -and $_.ClassName -eq '#32768' -and $_.Handle -ne $popup.Handle -and
            [ShotKitSettingsResetSmoke.Native]::PopupMenu($_.Handle) -eq $submenu
        } | Select-Object -First 1
    } 'owned settings submenu'
    return [pscustomobject]@{ Root = $popup; Child = $child; Menu = $submenu }
}
function Toggle-OpenEditor {
    $settingsMenu = Open-SettingsMenu
    $popup = $settingsMenu.Root
    $child = $settingsMenu.Child
    $submenu = $settingsMenu.Menu
    Write-UiSnapshot 'tray_setting_click_begin' -ExpectedId 101
    [void][ShotKitSettingsResetSmoke.Native]::ClickSetting($child.Handle, $submenu, 101, 'Nach Aufnahme Editor öffnen', $true, $process.Id)
    [void](Wait-Value { -not [ShotKitSettingsResetSmoke.Native]::WindowVisible($popup.Handle) } 'tray dismissal')
}
function Assert-LoadedOpenEditor([bool] $Expected) {
    # Read the app's newly constructed, visible native submenu. Its checkmark
    # comes from active_settings(), so unchanged disk bytes alone cannot pass.
    $settingsMenu = Open-SettingsMenu
    Write-UiSnapshot 'tray_setting_inspect_begin' -ExpectedId 101 -Details @{ expectedChecked = $Expected }
    [ShotKitSettingsResetSmoke.Native]::AssertSettingChecked($settingsMenu.Child.Handle, $settingsMenu.Menu, 101, 'Nach Aufnahme Editor öffnen', $Expected, $process.Id)
    # Cancel only the owned menu; no preference command is selected or toggled.
    Write-UiSnapshot 'tray_menu_cancel_begin'
    Assert-True ([ShotKitSettingsResetSmoke.Native]::PostMessageW($primary.Handle, 0x001F, [IntPtr]::Zero, [IntPtr]::Zero)) 'Owned menu cancellation post failed'
    [void](Wait-Value {
        -not [ShotKitSettingsResetSmoke.Native]::WindowVisible($settingsMenu.Root.Handle) -and
        -not [ShotKitSettingsResetSmoke.Native]::WindowVisible($settingsMenu.Child.Handle)
    } 'owned settings menu cancellation')
    [void](Wait-Value { (Read-Events) -ccontains 'tray_track_popup_choice=0' } 'cancelled menu result')
    Assert-True ([ShotKitSettingsResetSmoke.Native]::UiResponsive($primary.Handle)) 'App must remain responsive after checkmark inspection'
    Assert-True (@(Read-Events | Where-Object { $_ -match '^settings_save_stage=' }).Count -eq 0) 'Read-only checkmark inspection must not trigger a settings save'
}
function Assert-BlockedSave {
    Toggle-OpenEditor
    $warning = Wait-Dialog 'ShotKit' 'Einstellungen können nicht gespeichert werden'
    Click-OkDialog $warning
    Assert-PrimaryEnabled
    Assert-Event 'settings_save_stage=attempt'
    Assert-Event 'settings_save_stage=blocked reason=settings_not_writable'
    Assert-True (-not ((Read-Events) -ccontains 'settings_save_stage=success')) 'Blocked state must never report save success'
}
function New-Case([string] $Name) {
    $caseRoot = Join-Path $root $Name
    [void][IO.Directory]::CreateDirectory((Join-Path $caseRoot 'appdata/ShotKitRust'))
    [void][IO.Directory]::CreateDirectory((Join-Path $caseRoot 'localappdata'))
    return $caseRoot
}
function Assert-NoRecoveryArtifacts([string] $SettingsDir) {
    $files = @([IO.Directory]::GetFileSystemEntries($SettingsDir))
    Assert-True ($files.Count -eq 1 -and [IO.Path]::GetFileName($files[0]) -ceq 'settings.json') 'Cancellation must leave only the original settings file'
}
# Refuse traversal through any unexpected junction/symlink. If cleanup is unsafe
# or the owned process cannot be stopped, retain evidence and fail the smoke.
function Remove-SafeTree([string] $Path) {
    $full = [IO.Path]::GetFullPath($Path)
    Assert-True ($full -ceq $root -or $full.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) 'Cleanup escaped the generated test root'
    Assert-True ($null -ne $rootHandle -and -not $rootHandle.IsClosed -and -not $rootHandle.IsInvalid) 'Original generated root handle is unavailable'
    $entryHandle = $null
    try {
        # Every ancestor stays pinned while enumerating its children. Open each
        # leaf itself (OPEN_REPARSE_POINT), reject reparse attributes on that
        # held object, and delete through the same handle. There is no gap
        # between checking a directory and following a replaceable path.
        $entryHandle = if ($full -ceq $root) { $rootHandle } else { [ShotKitSettingsResetSmoke.Native]::HoldRoot($full) }
        $attrs = [ShotKitSettingsResetSmoke.Native]::Attributes($entryHandle)
        if (($attrs -band [IO.FileAttributes]::Directory) -ne 0) {
            foreach ($entry in [IO.Directory]::GetFileSystemEntries($full)) { Remove-SafeTree $entry }
        }
        [ShotKitSettingsResetSmoke.Native]::DeleteHeldRoot($entryHandle)
    } finally {
        if ($null -ne $entryHandle -and $full -cne $root) { $entryHandle.Dispose() }
    }
}

try {
    Assert-CleanRunner
    Assert-True (-not (Test-Path -LiteralPath $root)) 'Fresh test root already exists'
    [void][IO.Directory]::CreateDirectory($root)
    $rootCreated = $true
    $rootHandle = [ShotKitSettingsResetSmoke.Native]::HoldRoot($root)
    # Generate a valid PNG imported via argv; never capture the desktop.
    Add-Type -AssemblyName System.Drawing
    $image = [Drawing.Bitmap]::new(8, 8)
    try {
        $image.SetPixel(0, 0, [Drawing.Color]::Red)
        $image.Save((Join-Path $root 'fixture.png'), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $image.Dispose() }

    $caseRoot = New-Case 'default-no'
    $settingsDir = Join-Path $caseRoot 'appdata/ShotKitRust'
    $settingsPath = Join-Path $settingsDir 'settings.json'
    [IO.File]::WriteAllBytes($settingsPath, $rejected)
    Start-CaseProcess $caseRoot 'first'
    $prompt = Wait-Dialog 'Einstellungen zurücksetzen?' 'Bei Nein bleiben die Datei unverändert'
    Write-UiSnapshot 'default_no_inspect_begin' $prompt 7
    Assert-True ([ShotKitSettingsResetSmoke.Native]::DefaultButton($prompt.Handle) -eq 7) 'Native default button must be No (IDNO)'
    # Enter is delivered only to this PID-owned real message box. Do not click
    # No explicitly: this must exercise the Windows default-button behavior.
    $script:interactionSequence++
    Write-UiSnapshot 'default_no_enter_begin' $prompt 7
    Assert-True ([ShotKitSettingsResetSmoke.Native]::PostMessageW($prompt.Handle, 0x0100, [IntPtr]13, [IntPtr]0)) 'Enter keydown post failed'
    Assert-True ([ShotKitSettingsResetSmoke.Native]::PostMessageW($prompt.Handle, 0x0101, [IntPtr]13, [IntPtr]0)) 'Enter keyup post failed'
    $warning = Wait-Dialog 'ShotKit' 'Speichern der Einstellungen ist deaktiviert'
    Click-OkDialog $warning
    Complete-Startup
    Assert-Event 'settings_reset_stage=declined'
    Assert-True (-not ((Read-Events) -ccontains 'settings_reset_stage=approved')) 'Default Enter must not approve reset'
    Assert-BlockedSave
    Close-CaseProcess
    Assert-Bytes $settingsPath $rejected
    Assert-NoRecoveryArtifacts $settingsDir

    $caseRoot = New-Case 'explicit-yes'
    $settingsDir = Join-Path $caseRoot 'appdata/ShotKitRust'
    $settingsPath = Join-Path $settingsDir 'settings.json'
    [IO.File]::WriteAllBytes($settingsPath, $rejected)
    Start-CaseProcess $caseRoot 'first'
    $prompt = Wait-Dialog 'Einstellungen zurücksetzen?' 'ausdrücklich auf Standardwerte zurücksetzen'
    Click-DialogButton $prompt 6 # IDYES, independent of OS button localization.
    $success = Wait-Dialog 'Einstellungen zurückgesetzt' 'Das unveränderte Original bleibt erhalten'
    $backups = @([IO.Directory]::GetFiles($settingsDir, 'settings.json.recovery-*.bak'))
    Assert-True ($backups.Count -eq 1) 'One retained original backup required'
    $backup = $backups[0]
    Assert-Bytes $backup $rejected
    Assert-True ([IO.File]::Exists($backup + '.complete')) 'Completion receipt required'
    Assert-True (-not [IO.File]::Exists($settingsPath + '.reset-pending')) 'Successful reset left a pending intent'
    Assert-True (@([IO.Directory]::GetFileSystemEntries($settingsDir)).Count -eq 3) 'Only defaults, original backup and receipt may remain'
    Assert-Defaults $settingsPath
    $receiptBytes = [IO.File]::ReadAllBytes($backup + '.complete')
    Click-OkDialog $success
    Complete-Startup
    Assert-Event 'settings_reset_stage=approved'
    Assert-Event 'settings_reset_stage=success'
    Toggle-OpenEditor
    [void](Wait-Value { (Read-Events) -ccontains 'settings_save_stage=success' } 'real preference save after recovery')
    Assert-Defaults $settingsPath $false
    Close-CaseProcess
    Assert-Bytes $backup $rejected
    Assert-Bytes ($backup + '.complete') $receiptBytes
    # Restart proves that ordinary startup accepts the actual receipt/backup
    # pair and reads the saved preference, without a second reset or warning.
    $savedSettingsBytes = [IO.File]::ReadAllBytes($settingsPath)
    Start-CaseProcess $caseRoot 'restart'
    Complete-Startup
    Assert-LoadedOpenEditor $false
    Assert-Bytes $settingsPath $savedSettingsBytes
    Assert-True (@(Read-Events | Where-Object { $_ -match '^settings_(load_stage=failed|reset_stage=)' }).Count -eq 0) 'Clean restart must not reject the receipt or reset again'
    Close-CaseProcess
    Assert-Defaults $settingsPath $false
    Assert-Bytes $backup $rejected
    Assert-Bytes ($backup + '.complete') $receiptBytes

    $caseRoot = New-Case 'interrupted-missing-target'
    $settingsDir = Join-Path $caseRoot 'appdata/ShotKitRust'
    $settingsPath = Join-Path $settingsDir 'settings.json'
    [IO.File]::WriteAllBytes(($settingsPath + '.reset-pending'), $pending)
    Start-CaseProcess $caseRoot 'first'
    $warning = Wait-Dialog 'ShotKit' 'Speichern der Einstellungen ist deaktiviert'
    Click-OkDialog $warning
    Complete-Startup
    Assert-True (@(Read-Events | Where-Object { $_ -match '^settings_load_stage=failed ' }).Count -eq 1) 'Interrupted reset must fail startup load'
    Assert-True (@(Read-Events | Where-Object { $_ -match '^settings_reset_stage=' }).Count -eq 0) 'Pending state must not offer or perform another reset'
    Assert-BlockedSave
    Close-CaseProcess
    Assert-True (-not [IO.File]::Exists($settingsPath)) 'Pending reset must not become writable first-run defaults'
    Assert-Bytes ($settingsPath + '.reset-pending') $pending
    Assert-True (@([IO.Directory]::GetFileSystemEntries($settingsDir)).Count -eq 1) 'Interrupted state created unexpected settings artifacts'
    Write-Output 'PASS: real default-No Enter cancellation preserved exact rejected bytes and blocked saving; explicit Yes retained exact backup and receipt, loaded usable defaults, saved and restarted with the live preference checkmark preserved; missing target with pending reset stayed read-only.'
} catch {
    $failure = $_
    Write-UiSnapshot 'fixture_failed' -Details @{ error = $_.Exception.Message; scriptStack = $_.ScriptStackTrace }
}
finally {
    $allExited = $true
    if ($null -ne $process) {
        try {
            Assert-OwnedProcess
            if (-not $process.HasExited) { $process.Kill(); Assert-True ($process.WaitForExit(5000)) 'Owned app survived cleanup kill' }
            $allExited = $process.HasExited
        } catch { $allExited = $false; $cleanupFailure = $_ }
        $process.Dispose()
    }
    if ($rootCreated -and $allExited -and $null -ne $rootHandle) {
        try { Remove-SafeTree $root } catch { $cleanupFailure = $_ }
    } elseif ($rootCreated -and $null -eq $rootHandle) {
        $cleanupFailure = 'Generated root could not be pinned; refusing path-based deletion.'
    }
    if ($null -ne $rootHandle) { $rootHandle.Dispose() }
    if ($rootCreated -and $allExited -and $null -eq $cleanupFailure -and (Test-Path -LiteralPath $root)) {
        $cleanupFailure = 'Generated test root still exists after held-object cleanup.'
    }
}
if ($null -ne $cleanupFailure) { throw "CLEANUP_FAILED: $cleanupFailure. Temporary evidence retained at '$root'." }
if ($null -ne $failure) { throw $failure }
