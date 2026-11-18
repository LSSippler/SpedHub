# Baut das Setup: Launcher, Plugin, spedhub-save und Server im Release-Modus, dann Inno Setup.
#
#   pwsh installer/build.ps1                     # Ergebnis in dist\
#   pwsh installer/build.ps1 -OutDir dist-setup  # anderer Zielordner (CI)
#   pwsh installer/build.ps1 -SkipBuild          # nur Inno Setup, target\release ist schon gebaut
#
# Ergebnis: <OutDir>\SpedHub-Setup-<Version>.exe und <OutDir>\SHA256SUMS.txt. Die Version kommt
# aus der Cargo.toml im Wurzelordner. Inno Setup 6 wird gesucht und, wenn es fehlt, über
# Chocolatey installiert. Jeder Fehler bricht ab.

[CmdletBinding()]
param(
    [string]$OutDir = "dist",
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Invoke-Checked {
    param([string]$What, [scriptblock]$Command)
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$What ist fehlgeschlagen (Exit-Code $LASTEXITCODE)"
    }
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    if (-not $SkipBuild) {
        Write-Host "Baue Launcher, Plugin, spedhub-save und Server im Release-Modus"
        Invoke-Checked 'cargo build' {
            cargo build --release --locked -p spedhub-launcher -p spedhub-plugin -p spedhub-save -p spedhub-server
        }
    }

    $metadata = (cargo metadata --no-deps --format-version 1 --locked) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata ist fehlgeschlagen (Exit-Code $LASTEXITCODE)" }
    $launcher = ($metadata | ConvertFrom-Json).packages | Where-Object { $_.name -eq 'spedhub-launcher' }
    if (-not $launcher) { throw "spedhub-launcher fehlt in cargo metadata" }
    $version = [string]$launcher.version
    if ($version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') {
        throw "Version '$version' aus Cargo.toml ist keine Versionsnummer wie 0.2.0"
    }
    Write-Host "Version $version"

    foreach ($file in 'spedhub-launcher.exe', 'spedhub_plugin.dll', 'spedhub-save.exe', 'spedhub-server.exe') {
        $path = Join-Path $root "target\release\$file"
        if (-not (Test-Path $path)) { throw "$path fehlt. Erst ohne -SkipBuild bauen" }
    }

    # Inno Setup 6 suchen, sonst über Chocolatey installieren
    $candidates = @()
    if (${env:ProgramFiles(x86)}) { $candidates += Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe' }
    if ($env:ProgramFiles) { $candidates += Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe' }
    if ($env:LOCALAPPDATA) { $candidates += Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe' }
    $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) {
        $onPath = Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue
        if ($onPath) { $iscc = $onPath.Source }
    }
    if (-not $iscc) {
        if (-not (Get-Command 'choco' -ErrorAction SilentlyContinue)) {
            throw "Inno Setup 6 fehlt und Chocolatey ist nicht da. Inno Setup 6 von jrsoftware.org installieren"
        }
        Write-Host "Inno Setup 6 fehlt, installiere es über Chocolatey"
        Invoke-Checked 'choco install innosetup' { choco install innosetup -y --no-progress }
        $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
        if (-not $iscc) { throw "ISCC.exe nach der Installation nicht gefunden" }
    }
    Write-Host "Inno Setup: $iscc"

    if ([System.IO.Path]::IsPathRooted($OutDir)) { $out = $OutDir } else { $out = Join-Path $root $OutDir }
    New-Item -ItemType Directory -Force $out | Out-Null
    $name = "SpedHub-Setup-$version.exe"
    $setup = Join-Path $out $name
    if (Test-Path $setup) { Remove-Item $setup -Force }

    Invoke-Checked 'Inno Setup' {
        & $iscc "/DAppVersion=$version" "/DRootDir=$root" "/DOutputDir=$out" (Join-Path $root 'installer\spedhub.iss')
    }
    if (-not (Test-Path $setup)) { throw "$setup wurde nicht erzeugt" }

    # Prüfsumme wie sha256sum: Kleinbuchstaben, zwei Leerzeichen, Zeilenende LF
    $hash = (Get-FileHash -Algorithm SHA256 $setup).Hash.ToLowerInvariant()
    $sums = Join-Path $out 'SHA256SUMS.txt'
    [System.IO.File]::WriteAllText($sums, "$hash  $name`n", (New-Object System.Text.UTF8Encoding $false))

    Write-Host "Fertig: $setup"
    Write-Host "SHA-256: $hash"
}
finally {
    Pop-Location
}
