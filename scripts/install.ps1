<#
.SYNOPSIS
    meta-ast installer for Windows.
.DESCRIPTION
    Downloads and installs the latest (or specified) pre-built meta-ast
    binary from GitHub Releases.
.EXAMPLE
    irm https://raw.githubusercontent.com/metacall/meta-ast/main/scripts/install.ps1 | iex
.EXAMPLE
    & ([scriptblock]::Create((irm https://.../install.ps1))) -Version v0.5.0
#>

param(
    [string]$Version = "latest",
    [switch]$Deploy,
    [string]$InstallDir = "$env:USERPROFILE\.local\bin"
)

$ErrorActionPreference = "Stop"

$Repo = "metacall/meta-ast"
$BinaryName = "meta-ast.exe"

function Write-Info($msg) { Write-Host $msg }
function Fail($msg) { Write-Error $msg; exit 1 }

# ---- Detect architecture ----
$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
    "AMD64" { $Arch = "x86_64" }
    "ARM64" { $Arch = "aarch64" }
    default { Fail "Unsupported architecture: $arch" }
}

$Platform = "pc-windows-msvc"
$Variant = if ($Deploy) { "-deploy" } else { "" }
$Asset = "meta-ast-$Arch-$Platform$Variant.exe"

# ---- Resolve download URL ----
if ($Version -eq "latest") {
    $DownloadUrl = "https://github.com/$Repo/releases/latest/download/$Asset"
} else {
    $DownloadUrl = "https://github.com/$Repo/releases/download/$Version/$Asset"
}

Write-Info "Detected platform: $Arch-$Platform$Variant"
Write-Info "Downloading: $DownloadUrl"

# ---- Download ----
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$OutFile = Join-Path $InstallDir $BinaryName

try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $OutFile -UseBasicParsing
} catch {
    Fail "Download failed. Check that the version/platform combination exists."
}

Write-Info "Installed $BinaryName to $OutFile"

# ---- PATH check ----
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$InstallDir*") {
    Write-Info ""
    Write-Info "NOTE: $InstallDir is not on your PATH."
    Write-Info "Add it by running:"
    Write-Info "  [Environment]::SetEnvironmentVariable('Path', `$env:Path + ';$InstallDir', 'User')"
}

Write-Info ""
Write-Info "Run '$BinaryName --help' to get started."
