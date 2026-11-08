<#
.SYNOPSIS
  Installs the pinned Inno Setup 7 into a folder of its own and checks its version.

.DESCRIPTION
  Inno Setup 7 isn't on Chocolatey or winget (both stop at 6.x), so this takes the
  official release from github.com/jrsoftware/issrc, checks its SHA-256, and installs
  it silently for the current user into -Dir. Whatever Inno the machine already
  has is never used. In a workflow it also sets ISCC for the later steps.
#>
[CmdletBinding()]
param(
    [string]$Dir = (Join-Path ($env:RUNNER_TEMP ?? $env:TEMP) 'inno')
)

$ErrorActionPreference = 'Stop'

$version = '7.1.1'
$url = "https://github.com/jrsoftware/issrc/releases/download/is-7_1_1/innosetup-$version-x64.exe"
# Authenticode signed by Pyrsys B.V.
$sha256 = '8a5a6711c3766a6e5fb74f280ba0e3551e719ce94538d901fdbd1f8f9920b3f0'

$setup = Join-Path ($env:RUNNER_TEMP ?? $env:TEMP) "innosetup-$version-x64.exe"
foreach ($try in 1..3) {
    try {
        Invoke-WebRequest -Uri $url -OutFile $setup
        if ((Get-FileHash $setup -Algorithm SHA256).Hash -ieq $sha256) { break }
        throw "innosetup-$version-x64.exe doesn't match its pinned SHA-256."
    } catch {
        if ($try -eq 3) { throw }
        Start-Sleep -Seconds (10 * $try)
    }
}

$install = Start-Process $setup -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/SP-', '/NOICONS', '/TASKS=', "/DIR=$Dir"
)
if ($install.ExitCode -ne 0) { throw "The Inno Setup installer exited with $($install.ExitCode)." }

$iscc = Join-Path $Dir 'ISCC.exe'
if (-not (Test-Path $iscc)) { throw "ISCC.exe is missing from $Dir." }
# ISCC prints only "Inno Setup 7" and its files carry no version, so the
# per-user uninstall entry the installer just wrote is the record of what's there.
$entry = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 7_is1' -ErrorAction SilentlyContinue
if (-not $entry -or $entry.DisplayVersion -ne $version) { throw "Inno Setup ${version} isn't what got installed (found $($entry.DisplayVersion))." }
if ((Resolve-Path $entry.InstallLocation).Path.TrimEnd('\') -ne (Resolve-Path $Dir).Path.TrimEnd('\')) {
    throw "Inno Setup ${version} is registered at $($entry.InstallLocation), not $Dir."
}
Write-Host "Inno Setup $version at $iscc"

if ($env:GITHUB_ENV) { "ISCC=$iscc" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8 }
