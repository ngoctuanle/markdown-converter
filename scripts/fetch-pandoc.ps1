<#
.SYNOPSIS
  Download, verify and unpack the pinned pandoc release into vendor\pandoc\<triple>\.

.DESCRIPTION
  Windows counterpart of scripts/fetch-pandoc.sh. Safe to re-run: it skips the
  download when vendor\pandoc\<triple>\ already holds the pinned pandoc and its
  licence. Keep VERSION and the SHA-256 in sync with fetch-pandoc.sh and
  PINNED_VERSION in src/pandoc.rs.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\fetch-pandoc.ps1
#>
[CmdletBinding()]
param(
    # Only x86_64-pc-windows-msvc: pandoc publishes no Windows arm64 build.
    [string]$Triple = 'x86_64-pc-windows-msvc'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue' # the progress bar makes Invoke-WebRequest very slow in PS 5.1
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Version = '3.13'
$Assets = @{
    'x86_64-pc-windows-msvc' = @{
        Name   = "pandoc-$Version-windows-x86_64.zip"
        Sha256 = '23e1f2e978a3df80db8bf65c9c59534f41a23140480cfb4f783a882ab7dc6a4e'
    }
}

$Root = Split-Path -Parent $PSScriptRoot
$Dir = Join-Path $Root "vendor\pandoc\$Triple"
$Bin = Join-Path $Dir 'pandoc.exe'
$Copying = Join-Path $Dir 'COPYING.md'

if (-not $Assets.ContainsKey($Triple)) {
    throw "no pinned pandoc asset for target '$Triple' (use scripts/fetch-pandoc.sh for macOS and Linux)"
}

function Test-AlreadyHave {
    if (-not ((Test-Path $Bin -PathType Leaf) -and (Test-Path $Copying -PathType Leaf))) { return $false }
    try { $first = (& $Bin --version | Select-Object -First 1) } catch { return $false }
    return $first -eq "pandoc $Version"
}

if (Test-AlreadyHave) {
    Write-Host "pandoc $Version already present for $Triple"
    exit 0
}

$asset = $Assets[$Triple]
$work = Join-Path ([IO.Path]::GetTempPath()) ("fetch-pandoc-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $zip = Join-Path $work $asset.Name
    Write-Host "Downloading $($asset.Name) ..."
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/jgm/pandoc/releases/download/$Version/$($asset.Name)" -OutFile $zip

    $got = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
    if ($got -ne $asset.Sha256) {
        throw "SHA-256 mismatch for $($asset.Name)`n  expected $($asset.Sha256)`n  got      $got"
    }

    $x = Join-Path $work 'x'
    Expand-Archive -Path $zip -DestinationPath $x
    $found = Get-ChildItem -Path $x -Recurse -File -Filter 'pandoc.exe' | Select-Object -First 1
    if (-not $found) { throw "pandoc.exe not found inside $($asset.Name)" }

    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    Copy-Item -Force -Path $found.FullName -Destination $Bin
    Invoke-WebRequest -UseBasicParsing -Uri "https://raw.githubusercontent.com/jgm/pandoc/$Version/COPYING.md" -OutFile $Copying
    Write-Host "Installed $Bin"
}
finally {
    Remove-Item -Recurse -Force -Path $work -ErrorAction SilentlyContinue
}
