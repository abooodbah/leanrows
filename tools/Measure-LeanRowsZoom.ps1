[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [Parameter(Mandatory = $true)][string]$DocumentPath,
    [int]$Runs = 3,
    [int]$LoadSeconds = 4,
    [string]$Label = '',
    [string]$OutJson
)
# Drives a running LeanRows window from outside and times zoom. The saved
# zoom level is set to 100% for the run and put back afterwards.
# One step: WM_COMMAND zoom in/out, then wait until the list shows the state a
# settled window has at that level (header height and row height), then until
# the list and its column titles have painted it: no update region is left and
# a no-op message sent to the list has been answered.
# Bursts post the input at once (Ctrl+wheel notches) or every 33 ms (a held
# Ctrl+Plus at the keyboard repeat rate) and time until the final level is
# shown and painted. A sampler thread records the working set every 2 ms.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

if (-not ('ZoomProbe' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class ZoomProbe {
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool RedrawWindow(IntPtr h, IntPtr r, IntPtr g, uint f);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EnumProc p, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr p, uint f);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    public const uint WM_COMMAND = 0x0111, WM_MOUSEWHEEL = 0x020A;
    const uint LVM_GETHEADER = 0x101F, LVM_GETCOUNTPERPAGE = 0x1028, LVM_GETITEMCOUNT = 0x1004;
    const uint RDW_UPDATENOW = 0x100, RDW_ALLCHILDREN = 0x80;

    public static IntPtr FindList(IntPtr window) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(window, (h, l) => {
            var s = new StringBuilder(64); GetClassNameW(h, s, 64);
            if (s.ToString() == "SysListView32" && found == IntPtr.Zero) { found = h; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static int ItemCount(IntPtr list) { return (int)SendMessageW(list, LVM_GETITEMCOUNT, IntPtr.Zero, IntPtr.Zero); }
    // Header height and rows per page identify the zoom a settled list shows.
    public static long State(IntPtr list) {
        IntPtr header = SendMessageW(list, LVM_GETHEADER, IntPtr.Zero, IntPtr.Zero);
        RECT r; GetWindowRect(header, out r);
        // LVM_APPROXIMATEVIEWRECT for 500 rows: its height gives the row height.
        long view = (long)SendMessageW(list, 0x1040, (IntPtr)500, (IntPtr)(-1));
        int viewHeight = (int)((view >> 16) & 0xffff);
        return ((long)(r.Bottom - r.Top) << 32) | (uint)viewHeight;
    }
    [DllImport("user32.dll")] public static extern bool GetUpdateRect(IntPtr h, IntPtr r, bool erase);
    // Waits until the list and its titles have nothing left to paint, then
    // sends a no-op message, which the UI thread answers only once it has
    // finished the paint it is in.
    public static void Painted(IntPtr list) {
        IntPtr header = SendMessageW(list, LVM_GETHEADER, IntPtr.Zero, IntPtr.Zero);
        var clock = Stopwatch.StartNew();
        while ((GetUpdateRect(list, IntPtr.Zero, false) || GetUpdateRect(header, IntPtr.Zero, false)) && clock.ElapsedMilliseconds < 5000) { Thread.Sleep(0); }
        SendMessageW(list, 0, IntPtr.Zero, IntPtr.Zero);
    }
    public static void Flush(IntPtr window) { RedrawWindow(window, IntPtr.Zero, IntPtr.Zero, RDW_UPDATENOW | RDW_ALLCHILDREN); }
    public static void Command(IntPtr window, int id) { SendMessageW(window, WM_COMMAND, (IntPtr)id, IntPtr.Zero); }
    public static void PostCommand(IntPtr window, int id) { PostMessageW(window, WM_COMMAND, (IntPtr)id, IntPtr.Zero); }
    public static void PostWheel(IntPtr list, int notches) {
        int delta = 120 * notches;
        long w = ((long)(delta & 0xffff) << 16) | 0x0008;
        PostMessageW(list, WM_MOUSEWHEEL, (IntPtr)w, IntPtr.Zero);
    }
    // Waits until the list shows `expected` and has painted it. Returns ms.
    public static double WaitFor(IntPtr window, IntPtr list, long expected, Stopwatch clock, int timeoutMs) {
        while (clock.ElapsedMilliseconds < timeoutMs) {
            if (State(list) == expected) { Painted(list); Flush(window); return clock.Elapsed.TotalMilliseconds; }
            Thread.Sleep(0);
        }
        return -1;
    }

    // Working-set sampler.
    static volatile bool sampling;
    static long peakWs, peakPrivate;
    static Thread sampler;
    public static void StartSampling(Process p) {
        peakWs = 0; peakPrivate = 0; sampling = true;
        sampler = new Thread(() => {
            while (sampling) {
                p.Refresh();
                long ws = p.WorkingSet64, pv = p.PrivateMemorySize64;
                if (ws > peakWs) peakWs = ws;
                if (pv > peakPrivate) peakPrivate = pv;
                Thread.Sleep(2);
            }
        });
        sampler.IsBackground = true; sampler.Start();
    }
    public static long[] StopSampling() { sampling = false; sampler.Join(); return new long[] { peakWs, peakPrivate }; }
    public static uint Gdi(Process p) { return GetGuiResources(p.Handle, 0); }
}
'@
}

$IdIn = 123; $IdOut = 124; $IdReset = 125
$levels = @(50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300)

$regPath = 'HKCU:\Software\LeanRows'
$savedZoom = (Get-ItemProperty -Path $regPath -Name ZoomPercent -ErrorAction SilentlyContinue)
if ($savedZoom) { $savedZoom = $savedZoom.ZoomPercent }
New-Item -Path $regPath -Force | Out-Null
Set-ItemProperty -Path $regPath -Name ZoomPercent -Value 100 -Type DWord

$process = Start-Process -FilePath $BinaryPath -ArgumentList @('--', $DocumentPath) -PassThru
try {
    $null = $process.WaitForInputIdle(30000)
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ($process.MainWindowHandle -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 50; $process.Refresh()
    }
    $window = $process.MainWindowHandle
    [void][ZoomProbe]::SetWindowPos($window, [IntPtr]::Zero, 40, 40, 1400, 900, 0x0014)
    $list = [ZoomProbe]::FindList($window)
    if ($list -eq [IntPtr]::Zero) { throw 'list not found' }
    # Wait until the row count stops growing.
    $last = -1; $stable = 0
    while ($stable -lt 10) {
        Start-Sleep -Milliseconds 200
        $count = [ZoomProbe]::ItemCount($list)
        if ($count -eq $last -and $count -gt 0) { $stable++ } else { $stable = 0 }
        $last = $count
    }
    Start-Sleep -Seconds $LoadSeconds
    $process.Refresh()
    $restWs = $process.WorkingSet64; $restPrivate = $process.PrivateMemorySize64
    $gdiBefore = [ZoomProbe]::Gdi($process)

    # Reference state for each level, after a settled step.
    [ZoomProbe]::Command($window, $IdReset); Start-Sleep -Milliseconds 300
    $reference = @{}
    $reference[100] = [ZoomProbe]::State($list)
    $zoom = 100
    while ($zoom -lt 300) {
        [ZoomProbe]::Command($window, $IdIn); Start-Sleep -Milliseconds 300
        $zoom = ($levels | Where-Object { $_ -gt $zoom } | Select-Object -First 1)
        $reference[$zoom] = [ZoomProbe]::State($list)
    }
    while ($zoom -gt 50) {
        [ZoomProbe]::Command($window, $IdOut); Start-Sleep -Milliseconds 300
        $zoom = ($levels | Where-Object { $_ -lt $zoom } | Select-Object -Last 1)
        $reference[$zoom] = [ZoomProbe]::State($list)
    }
    [ZoomProbe]::Command($window, $IdReset); Start-Sleep -Milliseconds 300

    [ZoomProbe]::StartSampling($process)
    $steps = New-Object System.Collections.Generic.List[double]
    $stepsByLevel = @{}
    $bursts = @{ wheel8 = (New-Object System.Collections.Generic.List[double]); key8 = (New-Object System.Collections.Generic.List[double]); wheel_down12 = (New-Object System.Collections.Generic.List[double]) }
    for ($run = 0; $run -lt $Runs; $run++) {
        # Single steps 100 -> 300 -> 50 -> 100.
        $zoom = 100
        $plan = @()
        foreach ($l in $levels) { if ($l -gt 100) { $plan += , @($IdIn, $l) } }
        foreach ($l in ($levels[($levels.Length - 2)..0])) { $plan += , @($IdOut, $l) }
        foreach ($l in $levels) { if ($l -gt 50 -and $l -le 100) { $plan += , @($IdIn, $l) } }
        foreach ($step in $plan) {
            $clock = [System.Diagnostics.Stopwatch]::StartNew()
            [ZoomProbe]::Command($window, $step[0])
            $ms = [ZoomProbe]::WaitFor($window, $list, $reference[$step[1]], $clock, 10000)
            $steps.Add($ms)
            $key = "$($step[1])"
            if (-not $stepsByLevel.ContainsKey($key)) { $stepsByLevel[$key] = New-Object System.Collections.Generic.List[double] }
            $stepsByLevel[$key].Add($ms)
            Start-Sleep -Milliseconds 60
        }
        # Burst: eight Ctrl+wheel notches at once, 100 -> 300.
        Start-Sleep -Milliseconds 200
        $clock = [System.Diagnostics.Stopwatch]::StartNew()
        for ($i = 0; $i -lt 8; $i++) { [ZoomProbe]::PostWheel($list, 1) }
        $bursts.wheel8.Add([ZoomProbe]::WaitFor($window, $list, $reference[300], $clock, 20000))
        Start-Sleep -Milliseconds 200
        # Burst: twelve notches down at once, 300 -> 50.
        $clock = [System.Diagnostics.Stopwatch]::StartNew()
        for ($i = 0; $i -lt 12; $i++) { [ZoomProbe]::PostWheel($list, -1) }
        $bursts.wheel_down12.Add([ZoomProbe]::WaitFor($window, $list, $reference[50], $clock, 20000))
        Start-Sleep -Milliseconds 200
        [ZoomProbe]::Command($window, $IdReset); Start-Sleep -Milliseconds 200
        # Held Ctrl+Plus: eight zoom-in commands 33 ms apart; time from the
        # last key to the final level painted.
        $clock = [System.Diagnostics.Stopwatch]::StartNew()
        for ($i = 0; $i -lt 8; $i++) {
            [ZoomProbe]::PostCommand($window, $IdIn)
            if ($i -lt 7) { $target = $clock.ElapsedMilliseconds + 33; while ($clock.ElapsedMilliseconds -lt $target) { } }
        }
        $lastKey = $clock.Elapsed.TotalMilliseconds
        $done = [ZoomProbe]::WaitFor($window, $list, $reference[300], $clock, 20000)
        $bursts.key8.Add($done - $lastKey)
        Start-Sleep -Milliseconds 200
        [ZoomProbe]::Command($window, $IdReset); Start-Sleep -Milliseconds 200
    }
    $peak = [ZoomProbe]::StopSampling()
    $process.Refresh()
    $afterWs = $process.WorkingSet64
    $gdiAfter = [ZoomProbe]::Gdi($process)

    function Stat($values) {
        $sorted = @($values | Sort-Object)
        $n = $sorted.Count
        [pscustomobject]@{ n = $n; median = [math]::Round($sorted[[int][math]::Floor($n / 2)], 1); p90 = [math]::Round($sorted[[int][math]::Min($n - 1, [math]::Floor($n * 0.9))], 1); max = [math]::Round($sorted[$n - 1], 1); min = [math]::Round($sorted[0], 1) }
    }
    $levelStats = [ordered]@{}
    foreach ($l in $levels) { if ($stepsByLevel.ContainsKey("$l")) { $levelStats["$l"] = (Stat $stepsByLevel["$l"]).median } }
    $result = [pscustomobject]@{
        label = $Label
        file = [System.IO.Path]::GetFileName($DocumentPath)
        rows = [ZoomProbe]::ItemCount($list)
        step_ms = (Stat $steps)
        step_median_by_level = $levelStats
        wheel8_settle_ms = (Stat $bursts.wheel8)
        wheel_down12_settle_ms = (Stat $bursts.wheel_down12)
        held_key8_after_last_key_ms = (Stat $bursts.key8)
        rest_ws_mib = [math]::Round($restWs / 1MB, 1)
        rest_private_mib = [math]::Round($restPrivate / 1MB, 1)
        zoom_peak_ws_mib = [math]::Round($peak[0] / 1MB, 1)
        zoom_peak_private_mib = [math]::Round($peak[1] / 1MB, 1)
        after_ws_mib = [math]::Round($afterWs / 1MB, 1)
        gdi_before = $gdiBefore
        gdi_after = $gdiAfter
    }
    if ($OutJson) { $result | ConvertTo-Json -Depth 5 | Set-Content -Path $OutJson -Encoding utf8 }
    $result | ConvertTo-Json -Depth 5
} finally {
    if (-not $process.HasExited) { $process.CloseMainWindow() | Out-Null; if (-not $process.WaitForExit(5000)) { $process.Kill() } }
    if ($savedZoom) { Set-ItemProperty -Path $regPath -Name ZoomPercent -Value $savedZoom -Type DWord }
}
