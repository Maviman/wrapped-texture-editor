<#
CI smoke test of the Windows installer (run by .github/workflows/installer.yml on a clean windows-latest runner).

  1. Silent install of Setup.exe with every component and task; checks the files, shortcuts, .wtep association and the
     Add/Remove Programs entry (per user, version).
  2. Starts the installed editor: the graphics check, the built-in "smoke" self-test scenario, and a plain start that
     must still be running with its window's graphics device up after 20 s.
  3. Silent in-place upgrade with UpgradeSetup.exe (a higher test version): still one Add/Remove Programs entry, now
     with the new version; the tasks chosen before are kept.
  4. Silent uninstall: everything installed is gone, the user data in %APPDATA%\WrappedTextureEditor stays.
  5. Install again and uninstall with /PURGEUSERDATA: the user data goes too.

Any failed check throws, so the job fails and the release is not published. Logs go to -LogDir.
#>
param(
    [Parameter(Mandatory)] [string]$Setup,
    [Parameter(Mandatory)] [string]$UpgradeSetup,
    [Parameter(Mandatory)] [string]$Version,
    [Parameter(Mandatory)] [string]$UpgradeVersion,
    [string]$LogDir = (Join-Path $PWD 'smoke-logs')
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

$AppName = 'Wrapped Texture Editor'
$ExeName = "$AppName.exe"
$UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{C73E1B7D-2702-4D07-9460-D71FB97FD2B4}_is1'
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\$AppName"
$StartMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\$AppName"
$Desktop = [Environment]::GetFolderPath('Desktop')
$DataDir = Join-Path $env:APPDATA 'WrappedTextureEditor'

# Start-Process joins -ArgumentList items with spaces without quoting them: quote each one here.
function Q([string[]]$items) { ($items | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' ' }

function Check([bool]$ok, [string]$what) {
    if (-not $ok) { throw "FAILED: $what" }
    Write-Host "ok: $what"
}

function Run-Setup([string]$exe, [string[]]$extra, [string]$log) {
    $argList = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', "/LOG=$log") + $extra
    $p = Start-Process -FilePath $exe -ArgumentList (Q $argList) -PassThru -Wait
    Check ($p.ExitCode -eq 0) "$([IO.Path]::GetFileName($exe)) $($extra -join ' ') exits 0 (got $($p.ExitCode), log $log)"
}

function Uninstall([string[]]$extra, [string]$log) {
    $cmd = (Get-ItemProperty $UninstallKey).UninstallString.Trim('"')
    $argList = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=$log") + $extra
    Start-Process -FilePath $cmd -ArgumentList (Q $argList) -Wait | Out-Null
    # The uninstaller hands over to a copy of itself in %TEMP%: wait until it has removed its entry.
    $deadline = (Get-Date).AddSeconds(120)
    while ((Test-Path $UninstallKey) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    while ((Test-Path (Join-Path $InstallDir $ExeName)) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    # The second phase (%TEMP%\_iu*.tmp) deletes unins000.* and the folder last.
    while ((Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -like '_iu*' }) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
    }
    while ((Test-Path (Join-Path $InstallDir 'unins000.dat')) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    Start-Sleep -Seconds 2
}

# --- 1. Install --------------------------------------------------------------------------------------------------
Check (-not (Test-Path $UninstallKey)) 'the runner starts without an install'
Run-Setup $Setup @('/COMPONENTS=main,gpulaunchers,examples', '/TASKS=desktopicon,assoc') (Join-Path $LogDir 'install.log')

$entry = Get-ItemProperty $UninstallKey
Check ($entry.DisplayName -eq $AppName) "Add/Remove Programs lists '$AppName' (got '$($entry.DisplayName)')"
Check ($entry.DisplayVersion -eq $Version) "Add/Remove Programs shows version $Version (got '$($entry.DisplayVersion)')"
Check ($entry.UninstallString -match 'unins\d+\.exe') 'Add/Remove Programs has an uninstaller'
Check ((Resolve-Path $entry.InstallLocation).Path.TrimEnd('\') -eq $InstallDir) "installed per user into $InstallDir"
Check (-not (Test-Path 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{C73E1B7D-2702-4D07-9460-D71FB97FD2B4}_is1')) 'nothing written to HKLM'
foreach ($f in @($ExeName, 'app.ico', 'Start - DirectX 12.cmd', 'Start - Vulkan without overlays.cmd', 'Examples\torus_knot.obj')) {
    Check (Test-Path (Join-Path $InstallDir $f)) "installed $f"
}
foreach ($l in @("$AppName.lnk", "$AppName (DirectX 12).lnk", "$AppName (Vulkan without overlays).lnk")) {
    Check (Test-Path (Join-Path $StartMenu $l)) "Start Menu shortcut $l"
}
Check (Test-Path (Join-Path $Desktop "$AppName.lnk")) 'desktop shortcut (task desktopicon)'
$open = (Get-ItemProperty 'HKCU:\Software\Classes\WrappedTextureEditor.Project\shell\open\command').'(default)'
Check ($open -like "*$ExeName*%1*") ".wtep opens with the editor ($open)"
Check ((Get-Content (Join-Path $LogDir 'install.log') -Raw) -match 'Graphics check exit code') 'the installer ran its graphics check'

# --- 2. Start the installed editor ---------------------------------------------------------------------------------
$exe = Join-Path $InstallDir $ExeName
$report = Join-Path $LogDir 'gpu-check.txt'
$p = Start-Process -FilePath $exe -ArgumentList (Q @('--gpu-check', $report)) -PassThru -Wait
Write-Host (Get-Content $report -Raw)
Check ($p.ExitCode -in 0, 2) "--gpu-check finds a usable adapter (exit $($p.ExitCode))"

# A hosted runner has no GPU: DirectX 12 on Windows' software rasterizer (WARP) is what it can draw with.
if ($p.ExitCode -eq 2) { $env:WGPU_BACKEND = 'dx12' }
$shot = Join-Path $LogDir 'smoke.png'
$p = Start-Process -FilePath $exe -ArgumentList (Q @('--screenshot', $shot, '--scenario', 'smoke', '--size', '1280x800')) -PassThru
if (-not $p.WaitForExit(300000)) { Stop-Process -Id $p.Id -Force; throw 'FAILED: the smoke scenario did not finish within 5 minutes' }
$json = Get-Content (Join-Path $LogDir 'smoke.json') -Raw | ConvertFrom-Json
Check ($p.ExitCode -eq 0 -and $json.ok) "the self-test scenario 'smoke' passes (exit $($p.ExitCode), ok=$($json.ok), errors: $($json.errors -join '; '))"
Check (Test-Path $shot) 'the smoke scenario wrote its screenshot'

# A normal start, as from the Start Menu: still running after 20 s, with the window's graphics device up.
$p = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 20
$alive = -not $p.HasExited
$responding = $alive -and $p.Responding
if ($alive) { Stop-Process -Id $p.Id -Force; $p.WaitForExit(10000) | Out-Null }
# Read the log only now: the app holds it open for writing while it runs.
$log = Join-Path $DataDir 'log.txt'
$logText = if (Test-Path $log) { Get-Content $log -Raw } else { '' }
Copy-Item $log (Join-Path $LogDir 'app-start.log') -ErrorAction SilentlyContinue
Check $alive 'a normal start keeps running'
Check ($logText -match 'graphics backend in use') 'a normal start creates its window and graphics device'
# Any ERROR line (a panic, 'graphics device lost', a wgpu error, 'could not start'): after a fatal error the app waits on
# an error box, so "still running" alone does not prove it works.
$errors = @($logText -split "`n" | Where-Object { $_ -match '\sERROR\s' })
Check ($errors.Count -eq 0) "a normal start logs no error ($($errors -join ' | '))"
Check $responding 'the window answers messages (not hung)'

# --- 3. Upgrade in place -------------------------------------------------------------------------------------------
Run-Setup $UpgradeSetup @() (Join-Path $LogDir 'upgrade.log')
$entries = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' | Where-Object {
    (Get-ItemProperty $_.PSPath).DisplayName -eq $AppName })
Check ($entries.Count -eq 1) "one Add/Remove Programs entry after the upgrade (got $($entries.Count))"
Check ((Get-ItemProperty $UninstallKey).DisplayVersion -eq $UpgradeVersion) "the entry now shows $UpgradeVersion"
Check ((Get-ItemProperty $UninstallKey).InstallLocation.TrimEnd('\') -eq $InstallDir) 'the upgrade went into the same folder'
Check (Test-Path (Join-Path $Desktop "$AppName.lnk")) 'the upgrade kept the desktop shortcut chosen before'
Check (Test-Path (Join-Path $InstallDir 'Examples\torus_knot.obj')) 'the upgrade kept the components chosen before'

# --- 4. Uninstall, keeping user data ---------------------------------------------------------------------------------
Check (Test-Path $DataDir) "user data exists before the uninstall ($DataDir)"
Uninstall @() (Join-Path $LogDir 'uninstall.log')
Check (-not (Test-Path $UninstallKey)) 'the Add/Remove Programs entry is gone'
Check (-not (Test-Path (Join-Path $InstallDir $ExeName))) 'the editor is removed'
Check (-not (Test-Path $StartMenu)) 'the Start Menu folder is removed'
Check (-not (Test-Path (Join-Path $Desktop "$AppName.lnk"))) 'the desktop shortcut is removed'
Check (-not (Test-Path 'HKCU:\Software\Classes\WrappedTextureEditor.Project')) 'the .wtep association is removed'
Check (Test-Path $DataDir) 'a silent uninstall keeps the user data'

# --- 5. Install again, uninstall with /PURGEUSERDATA -----------------------------------------------------------------
Run-Setup $Setup @('/COMPONENTS=main') (Join-Path $LogDir 'install2.log')
Uninstall @('/PURGEUSERDATA') (Join-Path $LogDir 'uninstall2.log')
Check (-not (Test-Path $UninstallKey)) 'the second uninstall removed its entry'
Check (-not (Test-Path $DataDir)) '/PURGEUSERDATA deletes the user data'
Check (-not (Test-Path $InstallDir)) 'nothing is left in the install folder'

Write-Host 'Installer smoke test passed.'
