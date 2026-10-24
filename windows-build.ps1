<#
.SYNOPSIS
    Construit les installateurs Windows de VNC Viewer (setup NSIS et MSI), comme
    la release GitHub, et peut installer le résultat.

.DESCRIPTION
    Produit target\release\bundle\nsis\*-setup.exe et target\release\bundle\msi\*.msi.
    Les outils manquants (Trunk, Tauri CLI) sont téléchargés ou compilés dans
    target\install-tools ; NSIS et WiX sont téléchargés par Tauri.

    Si l'exécution de scripts est bloquée :
        powershell -ExecutionPolicy Bypass -File .\windows-build.ps1

.PARAMETER Deps
    Installe d'abord Rust (rustup) et les outils de compilation C++ de Visual
    Studio avec winget, s'ils sont absents.

.PARAMETER Bundles
    Formats à produire : nsis, msi (défaut : les deux).

.PARAMETER Install
    Lance ensuite l'installateur NSIS produit (ou le MSI à défaut).

.EXAMPLE
    .\windows-build.ps1 -Deps -Install
#>
[CmdletBinding()]
param(
    [switch]$Deps,
    [ValidateSet('nsis', 'msi')]
    [string[]]$Bundles = @('nsis', 'msi'),
    [switch]$Install
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$AppName = 'VNC Viewer'
$TrunkVersion = 'v0.21.14'
# Même version que la CI de release.
$TauriCliVersion = '2.12.3'

$Root = $PSScriptRoot
$Tools = Join-Path $Root 'target\install-tools'

function Write-Info([string]$Message) { Write-Host "==> $Message" -ForegroundColor Blue }
function Write-Warn([string]$Message) { Write-Warning $Message }
function Stop-WithError([string]$Message) {
    Write-Host "erreur : $Message" -ForegroundColor Red
    exit 1
}

function Test-Command([string]$Name) {
    [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

# Lance un programme externe et s'arrête s'il échoue.
function Invoke-Checked([string]$File, [string[]]$Arguments) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) {
        Stop-WithError "« $File $($Arguments -join ' ') » a échoué (code $LASTEXITCODE)"
    }
}

function Update-SessionPath {
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    $env:Path = "$cargoBin;$machine;$user"
}

# Outils C++ de Visual Studio (éditeur de liens MSVC), nécessaires à Rust.
function Test-MsvcTool {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { return $false }
    $path = & $vswhere -latest -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    return [bool]$path
}

function Install-Dependency {
    if (-not (Test-Command 'winget')) {
        Stop-WithError "winget est introuvable ; installez Rust (https://rustup.rs) et les « Build Tools for Visual Studio » (charge de travail C++) à la main"
    }
    if (-not (Test-MsvcTool)) {
        Write-Info 'Installation des outils de compilation C++ de Visual Studio (winget)'
        Invoke-Checked 'winget' @('install', '--id', 'Microsoft.VisualStudio.2022.BuildTools', '-e',
            '--accept-source-agreements', '--accept-package-agreements', '--override',
            '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
    }
    if (-not (Test-Command 'cargo')) {
        Write-Info 'Installation de Rust (winget, rustup)'
        Invoke-Checked 'winget' @('install', '--id', 'Rustlang.Rustup', '-e',
            '--accept-source-agreements', '--accept-package-agreements')
        Update-SessionPath
        if (Test-Command 'rustup') { Invoke-Checked 'rustup' @('default', 'stable') }
    }
}

# Trunk précompilé (version figée, somme de contrôle vérifiée), placé dans target\.
function Initialize-Trunk {
    if (Test-Command 'trunk') { return }
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        Write-Info "Compilation de Trunk ($env:PROCESSOR_ARCHITECTURE)"
        Invoke-Checked 'cargo' @('install', 'trunk', '--locked', '--version', $TrunkVersion.TrimStart('v'),
            '--root', $Tools)
        $env:Path = "$(Join-Path $Tools 'bin');$env:Path"
        return
    }
    $dir = Join-Path $Tools "trunk-$TrunkVersion"
    $env:Path = "$dir;$env:Path"
    if (Test-Path (Join-Path $dir 'trunk.exe')) { return }

    Write-Info "Téléchargement de Trunk $TrunkVersion"
    $asset = 'trunk-x86_64-pc-windows-msvc.zip'
    $base = "https://github.com/trunk-rs/trunk/releases/download/$TrunkVersion"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $zip = Join-Path $dir $asset
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $zip
    $expected = ((Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset.sha256").Content -split '\s+')[0]
    $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash
    if (-not $expected -or $expected -ne $actual) {
        Remove-Item -Force $zip
        Stop-WithError 'somme de contrôle de Trunk invalide'
    }
    Expand-Archive -Force -Path $zip -DestinationPath $dir
    Remove-Item -Force $zip
}

# Tauri CLI : celle de cargo si elle est à la bonne version, sinon npx, sinon
# compilée une fois dans target\. Renvoie la commande à lancer.
function Get-TauriCli {
    if ((Test-Command 'cargo-tauri') -and
        ((& cargo-tauri --version 2>$null) -eq "tauri-cli $TauriCliVersion")) {
        return , @('cargo-tauri')
    }
    $local = Join-Path $Tools 'bin\cargo-tauri.exe'
    if (Test-Path $local) { return , @($local) }
    if (Test-Command 'npx') { return , @('npx', '--yes', "@tauri-apps/cli@$TauriCliVersion") }
    Write-Info "Compilation de la Tauri CLI $TauriCliVersion (une seule fois)"
    Invoke-Checked 'cargo' @('install', 'tauri-cli', '--locked', '--version', $TauriCliVersion, '--root', $Tools)
    return , @($local)
}

# ---- Dépendances ------------------------------------------------------------

# $IsWindows n'existe qu'à partir de PowerShell 6 (Windows PowerShell 5.1 : Desktop).
if ($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) {
    Stop-WithError 'ce script ne fonctionne que sous Windows (Linux : linux-build.sh)'
}

if ($Deps) { Install-Dependency }
if (-not (Test-Command 'cargo')) { Update-SessionPath }
if (-not (Test-Command 'cargo')) {
    Stop-WithError 'Rust est introuvable ; relancez avec -Deps ou installez-le depuis https://rustup.rs'
}
if (-not (Test-MsvcTool)) {
    Write-Warn "les outils C++ de Visual Studio semblent absents ; la compilation risque d'échouer (relancez avec -Deps)"
}

if (Test-Command 'rustup') {
    Invoke-Checked 'rustup' @('target', 'add', 'wasm32-unknown-unknown')
}
else {
    Write-Warn "rustup est absent : la cible wasm32-unknown-unknown doit déjà être installée"
}

Initialize-Trunk
$tauri = Get-TauriCli

# ---- Construction -----------------------------------------------------------

Set-Location $Root
$bundleList = $Bundles -join ','
Write-Info "Construction des installateurs : $bundleList"
# Exécute aussi `trunk build --release` (beforeBuildCommand de tauri.conf.json).
$tauriArgs = @()
if ($tauri.Count -gt 1) { $tauriArgs = $tauri[1..($tauri.Count - 1)] }
Invoke-Checked $tauri[0] ($tauriArgs + @('build', '--bundles', $bundleList))

$bundleDir = Join-Path $Root 'target\release\bundle'
$produced = @(Get-ChildItem -ErrorAction SilentlyContinue -Path `
        (Join-Path $bundleDir 'nsis\*.exe'), (Join-Path $bundleDir 'msi\*.msi'))
if ($produced.Count -eq 0) { Stop-WithError "aucun installateur produit dans $bundleDir" }

Write-Info 'Installateurs produits :'
$produced | ForEach-Object { Write-Host "  $($_.FullName)" }

# ---- Installation -----------------------------------------------------------

if ($Install) {
    $latest = { param($pattern) Get-ChildItem -ErrorAction SilentlyContinue -Path $pattern |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 }
    $setup = & $latest (Join-Path $bundleDir 'nsis\*-setup.exe')
    if ($setup) {
        Write-Info "Lancement de $($setup.Name)"
        $process = Start-Process -FilePath $setup.FullName -Wait -PassThru
    }
    else {
        $msi = & $latest (Join-Path $bundleDir 'msi\*.msi')
        Write-Info "Lancement de $($msi.Name)"
        $process = Start-Process -FilePath 'msiexec.exe' -ArgumentList @('/i', "`"$($msi.FullName)`"") `
            -Wait -PassThru
    }
    if ($process.ExitCode -ne 0) { Stop-WithError "l'installation a échoué ou a été annulée (code $($process.ExitCode))" }
    Write-Info "$AppName est installé : lancez-le depuis le menu Démarrer."
}
