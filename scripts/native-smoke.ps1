param([string]$DataDirectory = '.local/gpui-native')
$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$DataPath = [IO.Path]::GetFullPath((Join-Path $ProjectRoot $DataDirectory))
$TestRoot = [IO.Path]::GetFullPath((Join-Path $ProjectRoot '.local')) + [IO.Path]::DirectorySeparatorChar
if (-not $DataPath.StartsWith($TestRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Native smoke data must be in this project .local directory.' }
New-Item -ItemType Directory -Force $DataPath | Out-Null
$TestLock = [IO.File]::Open((Join-Path $DataPath 'instance.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
New-Item -ItemType Directory -Force (Join-Path $DataPath 'history') | Out-Null
New-Item -ItemType Directory -Force (Join-Path $DataPath 'sessions') | Out-Null
$WorkspaceId = [guid]::NewGuid().ToString()
$TabId = [guid]::NewGuid().ToString()
$Profile = @{id='shell';name='PowerShell';command='powershell.exe';args=@();kind='shell'}
$Tab = @{id=$TabId;title='PowerShell';profile=$Profile;cwd=$ProjectRoot;session_id=$null;launched=$false;rows=30;cols=100}
$Data = @{version=1;selected_workspace=$WorkspaceId;profiles=@($Profile);workspaces=@(@{id=$WorkspaceId;name='vibe-terminal';directory=$ProjectRoot;tabs=@($Tab);selected_tab=$TabId})}
$Utf8 = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText((Join-Path $DataPath 'state.json'),($Data | ConvertTo-Json -Depth 12),$Utf8)
[IO.File]::WriteAllText((Join-Path $DataPath "history/$TabId.ansi"),"GPUI native smoke test`r`n`r`n  src/main.rs:12`r`n  src/terminal.rs:34`r`n  .local/gpui-native/edit-me.rs:2`r`n  .local/gpui-native/window.png`r`n`r`nThis is saved history. Resume starts a real PowerShell process.`r`n",$Utf8)
$TestLock.Dispose()
$env:VIBE_TERMINAL_DATA_DIR = $DataPath
$Metadata = (& cargo metadata --format-version 1 --no-deps --locked --manifest-path (Join-Path $ProjectRoot 'Cargo.toml')) -join "`n" | ConvertFrom-Json
$Executable = Join-Path $Metadata.target_directory 'debug/vibe-terminal.exe'
if (-not (Test-Path -LiteralPath $Executable)) { throw "Debug binary not found at $Executable. Run cargo build --locked first." }
$Process = Start-Process -FilePath $Executable -WorkingDirectory $ProjectRoot -PassThru -RedirectStandardError (Join-Path $DataPath 'stderr.log') -RedirectStandardOutput (Join-Path $DataPath 'stdout.log')
$Deadline = [DateTime]::UtcNow.AddSeconds(15)
do {
    Start-Sleep -Milliseconds 100
    $Process.Refresh()
    if ($Process.HasExited) { throw "Application exited: $($Process.ExitCode). See $DataPath/stderr.log" }
} while ($Process.MainWindowHandle -eq 0 -and [DateTime]::UtcNow -lt $Deadline)
if ($Process.MainWindowHandle -eq 0) { throw 'No native window appeared.' }
@{pid=$Process.Id;handle=$Process.MainWindowHandle.ToInt64();workspace=$WorkspaceId;tab=$TabId;data=$DataPath} | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $DataPath 'run.json')
Write-Output "PID=$($Process.Id) Handle=$($Process.MainWindowHandle) Data=$DataPath"
