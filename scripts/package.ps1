<#
.SYNOPSIS
  Build the Windows packages of Markdown Converter with the bundled pandoc.

.DESCRIPTION
  Windows counterpart of scripts/package.sh. It downloads and checks pandoc
  (scripts\fetch-pandoc.ps1), copies it to vendor\pandoc\current\, builds a
  release binary and then makes the requested packages in dist\:

    nsis  markdown-converter_<version>_x64-setup.exe   per-user installer (default)
    zip   markdown-converter_<version>_x64-portable.zip  unzip-and-run copy (default)
    wix   markdown-converter_<version>_x64_en-US.msi   MSI, for managed deployment

  Requires the MSVC build tools and cargo-packager:
    cargo install cargo-packager --locked --version 0.11.8

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\package.ps1

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\package.ps1 -Formats nsis,zip,wix
#>
[CmdletBinding()]
param(
    # Comma-separated or an array: nsis, zip, wix.
    [string[]]$Formats = @('nsis', 'zip'),

    # Only x86_64-pc-windows-msvc: pandoc publishes no Windows arm64 build.
    [ValidateSet('x86_64-pc-windows-msvc')]
    [string]$Triple = 'x86_64-pc-windows-msvc'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# "powershell -File" passes "-Formats nsis,zip" as one string, so split it here.
$Formats = @($Formats | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim().ToLowerInvariant() } | Where-Object { $_ })
foreach ($f in $Formats) {
    if ($f -notin 'nsis', 'zip', 'wix') { throw "unknown format '$f' (expected nsis, zip or wix)" }
}

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

# Native commands don't throw on failure in PowerShell 5.1, so check every exit code.
# A plain function with $args, so arguments such as --release reach the exe untouched.
function Invoke-Native {
    $exe, $rest = $args
    & $exe @rest
    if ($LASTEXITCODE -ne 0) { throw "'$($args -join ' ')' failed with exit code $LASTEXITCODE" }
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo not found. Install Rust (for example: scoop install rustup-msvc) and open a new terminal.'
}
& cargo packager --version *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'cargo-packager not found. Install it with: cargo install cargo-packager --locked --version 0.11.8'
}

# 1. pandoc for the target, staged where Cargo.toml's packager resources expect it.
& (Join-Path $PSScriptRoot 'fetch-pandoc.ps1') -Triple $Triple
$current = Join-Path $Root 'vendor\pandoc\current'
if (Test-Path $current) { Remove-Item -Recurse -Force $current }
New-Item -ItemType Directory -Path $current | Out-Null
Copy-Item -Path (Join-Path $Root "vendor\pandoc\$Triple\pandoc.exe") -Destination $current
# Staged under its shipped name: the MSI packager ignores a resource's target name.
Copy-Item -Path (Join-Path $Root "vendor\pandoc\$Triple\COPYING.md") -Destination (Join-Path $current 'pandoc-COPYING.md')

# 2. Release build. packager reads target\<triple>\release. CARGO_BUILD_TARGET makes
#    its before-packaging-command ("cargo build --release") build into that same
#    folder, so it finds the build up to date instead of compiling everything again.
$env:CARGO_BUILD_TARGET = $Triple
try {
    Invoke-Native cargo build --release --target $Triple

    $packagerFormats = @($Formats | Where-Object { $_ -ne 'zip' })
    if ($packagerFormats.Count -gt 0) {
        Invoke-Native cargo packager --release --target $Triple --formats ($packagerFormats -join ',')
    }
}
finally {
    Remove-Item Env:\CARGO_BUILD_TARGET -ErrorAction SilentlyContinue
}

# 3. Portable zip: the same files the installer puts in its install folder.
$dist = Join-Path $Root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
if ($Formats -contains 'zip') {
    $meta = (& cargo metadata --no-deps --format-version 1 | Out-String | ConvertFrom-Json).packages |
        Where-Object { $_.name -eq 'markdown-converter' }
    $zip = Join-Path $dist "markdown-converter_$($meta.version)_x64-portable.zip"
    $stage = Join-Path $dist '.portable'
    $folder = Join-Path $stage 'Markdown Converter'
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Path $folder | Out-Null
    Copy-Item (Join-Path $Root "target\$Triple\release\markdown-converter.exe") $folder
    Copy-Item (Join-Path $current 'pandoc.exe') $folder
    Copy-Item (Join-Path $current 'pandoc-COPYING.md') $folder
    Copy-Item (Join-Path $Root 'THIRD_PARTY_NOTICES.md') $folder
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Write-Host "Zipping $zip ..."
    Compress-Archive -Path $folder -DestinationPath $zip -CompressionLevel Optimal
    Remove-Item -Recurse -Force $stage
}

Write-Host ''
Write-Host 'Packages in dist\:'
Get-ChildItem $dist -File | Where-Object { $_.Extension -in '.exe', '.msi', '.zip' } |
    ForEach-Object { '  {0}  ({1:N0} MB)' -f $_.Name, ($_.Length / 1MB) }
