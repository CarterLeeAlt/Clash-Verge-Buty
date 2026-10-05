param([string]$WorkDir, [string]$WatchdogPath)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
if (-not $WorkDir) { $WorkDir = Join-Path $root ('.tmp\watchdog-test-' + [guid]::NewGuid().ToString('N')) }
$WorkDir = [IO.Path]::GetFullPath($WorkDir)
if (-not $WorkDir.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Test output must be inside the workspace' }
if (-not $WatchdogPath) { $WatchdogPath = Join-Path $root 'src-tauri\watchdog\target\x86_64-pc-windows-msvc\release\clash-verge-buty-watchdog.exe' }
if (Test-Path $WorkDir) { throw "Test directory must be new: $WorkDir" }
New-Item -ItemType Directory -Path $WorkDir | Out-Null
& rustc --edition 2021 "$root\src-tauri\watchdog\test-fixtures\gui.rs" -o "$WorkDir\fixture.exe"
if ($LASTEXITCODE -ne 0) { throw 'fixture compile failed' }
$helper = Join-Path $WorkDir 'clash-verge-buty-watchdog.exe'
Copy-Item -LiteralPath $WatchdogPath -Destination $helper
$fixture = Join-Path $WorkDir 'fixture.exe'
function Start-Case([string]$mode, [string]$name) {
    $dir = Join-Path $WorkDir $name
    New-Item -ItemType Directory $dir | Out-Null
    $start = [Diagnostics.ProcessStartInfo]::new($helper)
    $start.UseShellExecute = $false
    foreach ($arg in $fixture, $mode, $dir, 'path with spaces\') { $start.ArgumentList.Add($arg) }
    @{ Proc=[Diagnostics.Process]::Start($start); Dir=$dir }
}
function Wait-File([string]$path) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path $path)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "file timeout: $path" }
        Start-Sleep -Milliseconds 50
    }
}
foreach ($mode in 'clean','retry-once','restart','fail') {
    $case = Start-Case $mode $mode
    if (-not $case.Proc.WaitForExit(45000)) { throw "timeout: $mode" }
    $count = @(Get-Content (Join-Path $case.Dir 'launches.txt')).Count
    $expected = @{ clean=1; 'retry-once'=2; restart=2; fail=4 }[$mode]
    $exit = if ($mode -eq 'fail') { 7 } else { 0 }
    if ($count -ne $expected -or $case.Proc.ExitCode -ne $exit) { throw "failed $mode count=$count exit=$($case.Proc.ExitCode)" }
    $args = Get-Content (Join-Path $case.Dir 'args.txt')
    if ($args[-1] -ne '--clash-verge-watchdog-child' -or $args[-2] -ne 'path with spaces\') { throw "arguments changed: $mode" }
    Write-Output "PASS $mode"
}
$held = Start-Case 'hold' 'singleton'
Wait-File (Join-Path $held.Dir 'launches.txt')
$duplicate = Start-Case 'hold' 'duplicate'
if (-not $duplicate.Proc.WaitForExit(5000) -or $duplicate.Proc.ExitCode -ne 0 -or (Test-Path (Join-Path $duplicate.Dir 'launches.txt'))) { throw 'duplicate supervisor started a GUI' }
[IO.File]::WriteAllText((Join-Path $held.Dir 'exit'), 'exit')
if (-not $held.Proc.WaitForExit(5000)) { throw 'singleton exit timeout' }
Write-Output 'PASS singleton'
$external = Start-Case 'external-survives' 'external-survives'
Wait-File (Join-Path $external.Dir 'external.pid')
$externalPid = [int](Get-Content (Join-Path $external.Dir 'external.pid'))
[IO.File]::WriteAllText((Join-Path $external.Dir 'exit'), 'exit')
if (-not $external.Proc.WaitForExit(5000)) { throw 'external case exit timeout' }
if (-not (Get-Process -Id $externalPid -ErrorAction SilentlyContinue)) { throw 'external application killed' }
Stop-Process -Id $externalPid
Write-Output 'PASS external application survives'
Write-Output "Test artifacts: $WorkDir"
