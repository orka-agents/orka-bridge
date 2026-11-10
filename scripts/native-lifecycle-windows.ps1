# CI-only integrated debug shell. No workspace selection, VM, credentials or injected Ready state.
param([Parameter(Mandatory)][string]$Executable, [Parameter(Mandatory)][string]$Output)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Hosted CI walk only; never run against an existing user profile' }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NativeWalk {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string name);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string name);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, UIntPtr w, string l, uint flags, uint timeout, out UIntPtr result);
}
'@
$Executable = (Resolve-Path $Executable).Path
$Output = [IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Force $Output | Out-Null
# dirs::data_dir uses SHGetKnownFolderPath(RoamingAppData), not an APPDATA override.
$data = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'ai.orka.bridge'
$selected = Join-Path $data 'setup-v1/local-vm'
$vmData = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'ai.orka.bridge/local-vm-v1'
if (Test-Path $data) { throw "Expected fresh hosted-CI application profile: $data (will not erase it)" }
if (!(Test-Path $env:BRIDGE_DEV_IMAGE_DIR -PathType Container) -or @(Get-ChildItem -Force $env:BRIDGE_DEV_IMAGE_DIR).Count) { throw 'Expected empty image source directory' }
if (Get-Process -Name orka-bridge -ErrorAction SilentlyContinue) { throw 'Bridge is already running' }
$desktop = [System.Windows.Automation.AutomationElement]::RootElement
$children = [System.Windows.Automation.TreeScope]::Descendants
function Wait-For([string]$Message, [scriptblock]$Predicate) {
    for ($i = 0; $i -lt 100; $i++) {
        $value = & $Predicate
        if ($value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    throw $Message
}
function Find-Control($Root, [string]$Name, $Type) {
    $conditions = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    if ($Type) { $conditions = [System.Windows.Automation.AndCondition]::new($conditions, [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $Type)) }
    $Root.FindFirst($children, $conditions)
}
$button = [System.Windows.Automation.ControlType]::Button
$menuItem = [System.Windows.Automation.ControlType]::MenuItem
function Press($Element) {
    $pattern = $null
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) { $pattern.Invoke(); return }
    $point = $Element.GetClickablePoint()
    [NativeWalk]::SetCursorPos([int]$point.X, [int]$point.Y) | Out-Null
    [NativeWalk]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    [NativeWalk]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
}
function Capture([string]$Name, [bool]$Menu = $false) {
    $r = if ($Menu) { [System.Windows.Forms.SystemInformation]::VirtualScreen } else { $script:window.Current.BoundingRectangle }
    $bitmap = [Drawing.Bitmap]::new([int]$r.Width, [int]$r.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$r.X, [int]$r.Y, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $script:themeOutput "$Name.png"), [Drawing.Imaging.ImageFormat]::Png)
        if (!$Menu) {
            $colour = $bitmap.GetPixel($bitmap.Width - 30, $bitmap.Height - 30)
            $brightness = ($colour.R + $colour.G + $colour.B) / 765.0
            if (($script:theme -eq 'light' -and $brightness -lt 0.9) -or ($script:theme -eq 'dark' -and $brightness -gt 0.2)) { throw 'Window screenshot did not follow requested theme (or desktop capture unavailable)' }
        }
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
function Start-Bridge {
    if ((Test-Path $selected) -or (Test-Path $vmData)) { throw 'Saved local selection or VM data appeared; refusing to resume a VM' }
    $script:app = Start-Process $Executable -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $script:themeOutput "app-$([Guid]::NewGuid().ToString('N')).out.log") -RedirectStandardError (Join-Path $script:themeOutput "app-$([Guid]::NewGuid().ToString('N')).err.log")
    $script:hwnd = Wait-For 'Main native window did not appear' { $script:app.Refresh(); if ($script:app.MainWindowHandle -ne [IntPtr]::Zero) { $script:app.MainWindowHandle } }
    $script:window = [System.Windows.Automation.AutomationElement]::FromHandle($script:hwnd)
    [NativeWalk]::SetForegroundWindow($script:hwnd) | Out-Null
}
function Find-TrayIcon($Root, [string]$Label) {
    $all = @($Root.FindAll($children, [System.Windows.Automation.Condition]::TrueCondition))
    $all | ForEach-Object { "Name=$($_.Current.Name); Type=$($_.Current.ControlType.ProgrammaticName); Offscreen=$($_.Current.IsOffscreen)" } | Set-Content (Join-Path $script:themeOutput "tray-$Label.txt")
    # Explorer may expose tooltip text with extra notification copy and a non-Button role.
    $matches = @($all | Where-Object { $_.Current.Name -like '*Bridge*' -and !$_.Current.IsOffscreen })
    if ($matches.Count -eq 1) { return $matches[0] }
    if ($matches.Count -gt 1) { throw 'More than one Bridge icon in native notification area' }
}
function Tray-Menu {
    $taskbar = [NativeWalk]::FindWindow('Shell_TrayWnd', $null)
    $notify = [NativeWalk]::FindWindowEx($taskbar, [IntPtr]::Zero, 'TrayNotifyWnd', $null)
    if ($notify -eq [IntPtr]::Zero) { throw 'Native notification area missing (interactive CI desktop required)' }
    $area = [System.Windows.Automation.AutomationElement]::FromHandle($notify)
    $tray = Find-TrayIcon $area 'visible'
    if (!$tray -or $tray.Current.IsOffscreen) {
        $chevron = Find-Control $area 'Show hidden icons' $button
        if (!$chevron) { $chevron = Find-Control $area 'Notification Chevron' $button }
        if (!$chevron) { $chevron = $area.FindFirst($children, [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::AutomationIdProperty, 'NotificationChevron')) }
        if ($chevron) { Press $chevron }
        $overflow = Wait-For 'Native notification overflow missing' { $h = [NativeWalk]::FindWindow('NotifyIconOverflowWindow', $null); if ($h -ne [IntPtr]::Zero) { [System.Windows.Automation.AutomationElement]::FromHandle($h) } }
        $tray = Wait-For 'Native Bridge notification icon missing' { Find-TrayIcon $overflow 'overflow' }
    }
    # Right-click the actual Explorer notification icon; do not invoke a product test command.
    $point = $tray.GetClickablePoint()
    [NativeWalk]::SetCursorPos([int]$point.X, [int]$point.Y) | Out-Null
    [NativeWalk]::mouse_event(8, 0, 0, 0, [UIntPtr]::Zero)
    [NativeWalk]::mouse_event(16, 0, 0, 0, [UIntPtr]::Zero)
    $open = Wait-For 'Open Bridge menu item missing' { Find-Control $desktop 'Open Bridge' $menuItem }
    $stop = Wait-For 'Stop workspace and quit menu item missing' { Find-Control $desktop 'Stop workspace and quit' $menuItem }
    if (!$open.Current.IsEnabled -or !$stop.Current.IsEnabled -or $open.Current.IsOffscreen -or $stop.Current.IsOffscreen) { throw 'Native tray commands disabled or not drawn' }
    return @($open, $stop)
}
function Retained {
    $script:app.Refresh()
    if ($script:app.HasExited -or !(Get-Process -Id $script:originalPid -ErrorAction SilentlyContinue)) { throw 'Original Bridge process lost' }
    if (![NativeWalk]::IsWindow($script:hwnd) -or ![NativeWalk]::IsWindowVisible($script:hwnd) -or [NativeWalk]::IsIconic($script:hwnd)) { throw 'Original window not visible/unminimized' }
    Wait-For 'Reopen did not focus original process' {
        [uint32]$foregroundPid = 0
        [NativeWalk]::GetWindowThreadProcessId([NativeWalk]::GetForegroundWindow(), [ref]$foregroundPid) | Out-Null
        $foregroundPid -eq $script:originalPid
    } | Out-Null
    # File is a view toggle, not persisted in the document: a webview remount loses this editor.
    $editor = Find-Control $script:window 'Agent file' ([System.Windows.Automation.ControlType]::Edit)
    if (!$editor -or $editor.Current.IsOffscreen) { throw 'File editor view lost on reopen' }
    $value = $editor.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value
    if (!$value.Contains($script:marker)) { throw 'Edited text lost on reopen' }
    if (Test-Path $selected) { throw 'Walk accidentally selected a workspace' }
}
function Wait-Exit {
    if (!$script:app.WaitForExit(15000)) { throw 'Native quit did not exit within 15 seconds' }
    if ($script:app.ExitCode -ne 0) { throw "Native quit failed with exit code $($script:app.ExitCode)" }
}
$personalize = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
$existed = Test-Path $personalize
$prior = @{}
foreach ($name in @('AppsUseLightTheme', 'SystemUsesLightTheme')) { $prior[$name] = Get-ItemPropertyValue $personalize $name -ErrorAction SilentlyContinue }
function Broadcast-Theme {
    $result = [UIntPtr]::Zero
    [NativeWalk]::SendMessageTimeout([IntPtr]0xffff, 0x1a, [UIntPtr]::Zero, 'ImmersiveColorSet', 2, 1000, [ref]$result) | Out-Null
}
try {
    if (!$existed) { New-Item $personalize -Force | Out-Null }
    foreach ($theme in @('light', 'dark')) {
        $script:theme = $theme
        $script:themeOutput = Join-Path $Output $theme
        New-Item -ItemType Directory -Force $script:themeOutput | Out-Null
        foreach ($name in $prior.Keys) { Set-ItemProperty $personalize $name ([int]($theme -eq 'light')) }
        Broadcast-Theme
        Start-Bridge
        $script:originalPid = $script:app.Id
        Press (Wait-For 'Designer button missing' { Find-Control $script:window 'Designer' $button })
        Press (Wait-For 'File button missing' { Find-Control $script:window 'File' $button })
        $editor = Wait-For 'Real File editor missing' { Find-Control $script:window 'Agent file' ([System.Windows.Automation.ControlType]::Edit) }
        $editor.SetFocus()
        $script:marker = " lifecycle-$([Guid]::NewGuid().ToString('N'))"
        [System.Windows.Forms.SendKeys]::SendWait('^{END}')
        [System.Windows.Forms.SendKeys]::SendWait($script:marker)
        $owned = Join-Path ([Environment]::GetFolderPath('UserProfile')) 'Orka/agents/web-reader/web-reader.agent.md'
        Wait-For 'Native editor change not saved' { (Test-Path $owned) -and [IO.File]::ReadAllText($owned).Contains($script:marker) } | Out-Null
        # SC_CLOSE follows the native titlebar Close route, unlike process termination.
        [NativeWalk]::PostMessage($script:hwnd, 0x112, [IntPtr]0xf060, [IntPtr]::Zero) | Out-Null
        Wait-For 'Native Close did not hide' { ![NativeWalk]::IsWindowVisible($script:hwnd) } | Out-Null
        if ($script:app.HasExited) { throw 'Close quit instead of hiding' }
        $commands = Tray-Menu
        Capture 'menu-hidden' $true
        Press $commands[0]
        Wait-For 'Open Bridge did not reopen' { [NativeWalk]::IsWindowVisible($script:hwnd) } | Out-Null
        Retained; Capture 'reopened-menu'
        [NativeWalk]::PostMessage($script:hwnd, 0x112, [IntPtr]0xf060, [IntPtr]::Zero) | Out-Null
        Wait-For 'Close before second launch did not hide' { ![NativeWalk]::IsWindowVisible($script:hwnd) } | Out-Null
        $second = Start-Process $Executable -NoNewWindow -PassThru
        if (!$second.WaitForExit(15000) -or $second.ExitCode -ne 0) { throw 'Hidden-window second launch did not hand off and exit cleanly' }
        Wait-For 'Second launch did not show hidden original window' { [NativeWalk]::IsWindowVisible($script:hwnd) } | Out-Null
        Retained; Capture 'reopened-second-launch-hidden'
        [NativeWalk]::ShowWindow($script:hwnd, 6) | Out-Null
        Wait-For 'Window did not minimize' { [NativeWalk]::IsIconic($script:hwnd) } | Out-Null
        $second = Start-Process $Executable -NoNewWindow -PassThru
        if (!$second.WaitForExit(15000) -or $second.ExitCode -ne 0) { throw 'Second executable launch did not hand off and exit cleanly' }
        Wait-For 'Second launch did not restore original window' { ![NativeWalk]::IsIconic($script:hwnd) } | Out-Null
        Retained; Capture 'reopened-second-launch-minimized'
        if (@(Get-Process -Name orka-bridge).Count -ne 1) { throw 'A second Bridge process survived' }
        $commands = Tray-Menu
        Capture 'menu-stop' $true
        Press $commands[1]; Wait-Exit
        Start-Bridge
        Press (Wait-For 'Native File menu missing' { Find-Control $script:window 'File' $menuItem })
        Press (Wait-For 'Standard native Quit missing' { Find-Control $desktop 'Quit' $menuItem })
        Wait-Exit
        if ((Test-Path $selected) -or (Test-Path $vmData)) { throw 'Saved local selection or VM data appeared' }
        "PASS $theme`: native Close/hide, tray menu/Open, same PID $script:originalPid/window $script:hwnd, File view/edited text retained, hidden/minimized second launches, menu stop, standard Quit. Integrated debug shell only; empty image source, no VM/Ready/provider/cluster/session logout proof. OS popup theme is captured as drawn, not forced by product hooks." | Tee-Object (Join-Path $script:themeOutput 'evidence.txt')
    }
} catch {
    if ($script:themeOutput) { try { Capture 'failure' $true } catch { Write-Warning $_ } }
    throw
} finally {
    # Force cleanup only on test failure; it is not evidence that the stop route worked.
    if ($script:app -and !$script:app.HasExited) { Stop-Process -Id $script:app.Id -Force -ErrorAction SilentlyContinue }
    foreach ($name in $prior.Keys) {
        if ($null -eq $prior[$name]) { Remove-ItemProperty $personalize $name -ErrorAction SilentlyContinue }
        else { Set-ItemProperty $personalize $name $prior[$name] }
    }
    if (!$existed) { Remove-Item $personalize -ErrorAction SilentlyContinue }
    Broadcast-Theme
}
