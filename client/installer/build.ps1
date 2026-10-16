# Gera o instalador do Windows em client\dist\ampero-<versao>-windows-x64-setup.exe.
# Uso: powershell -ExecutionPolicy Bypass -File installer\build.ps1 [-NoBuild] [-BinDir <pasta>]
#   -NoBuild  usa os executaveis ja compilados (o workflow de release compila antes)
#   -BinDir   pasta com ampero.exe e ampero-gui.exe (padrao: target\release)
param(
    [switch]$NoBuild,
    [string]$BinDir
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Versao: $version"

if (-not $NoBuild) {
    Push-Location $root
    try {
        cargo build --release --features gui
        if ($LASTEXITCODE -ne 0) { throw "cargo build falhou ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}
if (-not $BinDir) { $BinDir = "$root\target\release" }
$BinDir = (Resolve-Path $BinDir).Path

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { $iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source }
if (-not $iscc) { throw 'Inno Setup nao encontrado. Instale com: winget install -e --id JRSoftware.InnoSetup' }

& $iscc "/DAppVersion=$version" "/DBinDir=$BinDir" "$PSScriptRoot\ampero.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC falhou ($LASTEXITCODE)" }

Write-Host "Instalador: $root\dist\ampero-$version-windows-x64-setup.exe"
