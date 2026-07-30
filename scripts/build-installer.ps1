<#
.SYNOPSIS
    Собрать TVOICE и упаковать в установщик.

.DESCRIPTION
    Версию берёт из Cargo.toml — руками её нигде дублировать не нужно.
    Результат: dist\TVOICE-<версия>-setup.exe

    Нужен Inno Setup 6 (iscc.exe). Если его нет, скрипт скажет, где взять, и остановится:
    молча собрать «установщик» без установщика хуже, чем не собрать вовсе.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
#>

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# --- версия из Cargo.toml ---
$cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
    throw 'не нашёл version в Cargo.toml'
}
$version = $Matches[1]
Write-Host "TVOICE $version" -ForegroundColor Cyan

# --- сборка ---
Write-Host 'cargo build --release' -ForegroundColor Cyan
Push-Location $root
try {
    & cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build вернул $LASTEXITCODE" }
} finally {
    Pop-Location
}

$exe = Join-Path $root 'target\release\tvoice.exe'
if (-not (Test-Path $exe)) { throw "не собрался $exe" }

# --- поиск Inno Setup ---
# Пользовательская установка идёт первой: `winget install JRSoftware.InnoSetup` без прав
# администратора кладёт Inno Setup именно в профиль, а не в Program Files.
$iscc = $null
$candidates = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
)
foreach ($c in $candidates) {
    if ($c -and (Test-Path $c)) { $iscc = $c; break }
}
if (-not $iscc) {
    $cmd = Get-Command iscc -ErrorAction SilentlyContinue
    if ($null -ne $cmd) { $iscc = $cmd.Source }
}
if (-not $iscc) {
    Write-Host ''
    Write-Host 'Inno Setup 6 не найден — установщик собрать нечем.' -ForegroundColor Yellow
    Write-Host '  скачать:  https://jrsoftware.org/isdl.php'
    Write-Host '  или:      winget install JRSoftware.InnoSetup'
    Write-Host ''
    Write-Host "Собранная программа готова: $exe"
    exit 1
}

# --- упаковка ---
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Write-Host "iscc ($iscc)" -ForegroundColor Cyan
& $iscc (Join-Path $root 'installer\tvoice.iss') "/DAppVersion=$version"
if ($LASTEXITCODE -ne 0) { throw "iscc вернул $LASTEXITCODE" }

$setup = Join-Path $dist "TVOICE-$version-setup.exe"
if (-not (Test-Path $setup)) { throw "iscc отработал, но $setup нет" }
$size = [math]::Round((Get-Item $setup).Length / 1MB, 1)
Write-Host ''
Write-Host "Готово: $setup ($size МБ)" -ForegroundColor Green
