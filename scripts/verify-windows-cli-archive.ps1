param(
  [Parameter(Mandatory = $true)][string]$Archive,
  [Parameter(Mandatory = $true)][string]$ExpectedVersion
)

$ErrorActionPreference = 'Stop'
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $temp | Out-Null

try {
  Write-Host "Verifying staged Windows CLI archive: $Archive"
  Expand-Archive -LiteralPath $Archive -DestinationPath $temp -Force
  $cli = Join-Path $temp 'fini.exe'
  if (!(Test-Path -LiteralPath $cli)) {
    throw 'Archive does not contain fini.exe at top level'
  }

  $output = & $cli --version 2>&1
  $status = $LASTEXITCODE
  Write-Host "fini.exe --version exit=$status"
  Write-Host "output=$output"
  if ($status -ne 0) {
    exit $status
  }

  if ($output -notmatch "^fini $([regex]::Escape($ExpectedVersion))(-rc\.[0-9]+)?$") {
    throw "Unexpected fini.exe --version output: $output"
  }
} finally {
  Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
