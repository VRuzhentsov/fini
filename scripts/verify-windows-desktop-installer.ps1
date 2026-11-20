# Install the NSIS setup.exe silently, as a user does. Make sure that
# fini-app.exe is a window program (PE subsystem 2), not a console program
# (PE subsystem 3, the CLI). Start the app, make sure that it still runs
# after 15 seconds, then stop it.
param(
  [Parameter(Mandatory = $true)][string]$Installer
)

$ErrorActionPreference = 'Stop'

Write-Host "Verifying Windows desktop installer: $Installer"
$install = Start-Process -FilePath $Installer -ArgumentList '/S' -PassThru -Wait
if ($install.ExitCode -ne 0) {
  throw "Windows installer failed with exit code $($install.ExitCode)"
}

$app = Join-Path $env:LOCALAPPDATA 'Fini/fini-app.exe'
if (!(Test-Path -LiteralPath $app)) {
  throw "Installer did not put the desktop app at $app"
}

# PE subsystem: 2 is a window program, 3 is a console program.
$bytes = [IO.File]::ReadAllBytes($app)
$peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
$subsystem = [BitConverter]::ToUInt16($bytes, $peOffset + 24 + 68)
if ($subsystem -ne 2) {
  throw "Desktop app is a console program (PE subsystem $subsystem): the installer contains the CLI"
}

$process = Start-Process -FilePath $app -PassThru
Start-Sleep -Seconds 15
if ($process.HasExited) {
  throw "Desktop app stopped after start with exit code $($process.ExitCode)"
}
Stop-Process -Id $process.Id -Force
