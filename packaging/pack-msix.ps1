<#
.SYNOPSIS
  Packs seam.exe and seam-settings.exe into an MSIX. Needs only the Windows SDK.

.DESCRIPTION
  Always writes dist\<IdentityName>_<version>_x64-store.msix, unsigned. That is
  the file for Partner Center, where Microsoft signs it.

  -Sign     also writes dist\...-test.msix, signed with a self-signed certificate
            whose subject is the manifest Publisher. For this machine only.
  -Install  implies -Sign, then removes the old package by PackageFullName and
            installs the test package.
  -Upload   sends the -store.msix to Partner Center as a draft with msstore
            (publish --noCommit). It never commits the submission.

  Build first: cargo build --release --workspace
#>
[CmdletBinding()]
param(
    [switch]$Sign,
    [switch]$Install,
    [switch]$Upload,
    # The workflow passes the version it was asked for, so a mismatch stops the run.
    [string]$ExpectVersion
)

$ErrorActionPreference = 'Stop'
if ($Install) { $Sign = $true }

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$release = Join-Path $root 'target\release'
$work = Join-Path $root 'target\msix'
$layout = Join-Path $work 'layout'
$dist = Join-Path $root 'dist'

# 1. The newest SDK that has both tools.
$sdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$sdk = Get-ChildItem $sdkBin -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    Where-Object { (Test-Path (Join-Path $_.FullName 'x64\makeappx.exe')) -and (Test-Path (Join-Path $_.FullName 'x64\makepri.exe')) } |
    Select-Object -First 1
if (-not $sdk) { throw "No Windows SDK with makeappx.exe and makepri.exe under $sdkBin. Install the Windows 11 SDK." }
$tools = Join-Path $sdk.FullName 'x64'
Write-Host "SDK $($sdk.Name)"

# 2. The version from Cargo.toml, as a four-part number. The Store takes no pre-release tags.
$line = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $line) { throw 'No version in Cargo.toml.' }
$version = $line.Matches[0].Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "Version $version has a pre-release tag or isn't x.y.z. The Store can't take it." }
if ($ExpectVersion -and $ExpectVersion -ne $version) { throw "Cargo.toml is $version, but $ExpectVersion was asked for." }
$quad = "$version.0"

# 3. The identity from Partner Center.
$identity = Get-Content (Join-Path $PSScriptRoot 'identity.json') -Raw | ConvertFrom-Json
$placeholder = ($identity.IdentityName -like 'REPLACE*') -or ($identity.Publisher -like 'CN=00000000*') -or ($identity.PublisherDisplayName -like 'REPLACE*')
if ($placeholder) {
    Write-Warning 'identity.json still holds placeholders. This package works for local tests only. Partner Center will refuse it.'
}

foreach ($exe in 'seam.exe', 'seam-settings.exe') {
    if (-not (Test-Path (Join-Path $release $exe))) { throw "$exe is missing from target\release. Run: cargo build --release --workspace" }
}

# 4. Stage a clean layout. No VFS: both exes sit at the package root.
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Path $layout, $dist -Force | Out-Null
Copy-Item (Join-Path $release 'seam.exe'), (Join-Path $release 'seam-settings.exe') $layout
Copy-Item (Join-Path $PSScriptRoot 'Assets') (Join-Path $layout 'Assets') -Recurse

$manifest = Get-Content (Join-Path $PSScriptRoot 'AppxManifest.xml') -Raw
$manifest = $manifest.Replace('@IDENTITY_NAME@', $identity.IdentityName).Replace('@PUBLISHER@', $identity.Publisher).Replace('@PUBLISHER_DISPLAY_NAME@', $identity.PublisherDisplayName).Replace('@VERSION@', $quad)
[IO.File]::WriteAllText((Join-Path $layout 'AppxManifest.xml'), $manifest, (New-Object Text.UTF8Encoding($false)))

# 5. resources.pri, which the scaled assets need. The config stays outside the layout.
$priConfig = Join-Path $work 'priconfig.xml'
& (Join-Path $tools 'makepri.exe') createconfig /cf $priConfig /dq en-US /o
if ($LASTEXITCODE -ne 0) { throw 'makepri createconfig failed.' }
& (Join-Path $tools 'makepri.exe') new /pr $layout /cf $priConfig /mn (Join-Path $layout 'AppxManifest.xml') /of (Join-Path $layout 'resources.pri') /o
if ($LASTEXITCODE -ne 0) { throw 'makepri new failed.' }

# 6. The unsigned package for the Store.
$base = Join-Path $dist "$($identity.IdentityName)_${quad}_x64"
$store = "$base-store.msix"
& (Join-Path $tools 'makeappx.exe') pack /d $layout /p $store /o
if ($LASTEXITCODE -ne 0) { throw 'makeappx pack failed.' }
Write-Host "Wrote $store (unsigned, for Partner Center)"

# 7. The self-signed test package.
if ($Sign) {
    $signtool = Join-Path $tools 'signtool.exe'
    if (-not (Test-Path $signtool)) { throw "signtool.exe is missing from $tools." }
    $subject = $identity.Publisher
    $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $subject -and $_.FriendlyName -eq 'Seam test' } | Select-Object -First 1
    if (-not $cert) {
        $cert = New-SelfSignedCertificate -Type Custom -Subject $subject -FriendlyName 'Seam test' -KeyUsage DigitalSignature `
            -CertStoreLocation Cert:\CurrentUser\My -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
    }
    $trusted = Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object { $_.Thumbprint -eq $cert.Thumbprint }
    if (-not $trusted) {
        $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        if (-not $admin) { throw 'The test certificate must be trusted once. Run this from an elevated prompt.' }
        $cer = Join-Path $work 'seam-test.cer'
        Export-Certificate -Cert $cert -FilePath $cer | Out-Null
        Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    }
    $test = "$base-test.msix"
    Copy-Item $store $test -Force
    & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /s My $test
    if ($LASTEXITCODE -ne 0) { throw 'signtool failed.' }
    Write-Host "Wrote $test (self-signed, this machine only)"
}

# 8. Install the test package. A same-version package that is staged but not
#    registered makes Add-AppxPackage fail with 0x80073cfb, and
#    Get-AppxPackage doesn't list staged packages, so remove by PackageFullName.
if ($Install) {
    function Get-PublisherId([string]$Publisher) {
        $hash = [Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::Unicode.GetBytes($Publisher))
        $bits = (($hash[0..7] | ForEach-Object { [Convert]::ToString($_, 2).PadLeft(8, '0') }) -join '') + '0'
        $alphabet = '0123456789abcdefghjkmnpqrstvwxyz'
        -join (0..12 | ForEach-Object { $alphabet[[Convert]::ToInt32($bits.Substring($_ * 5, 5), 2)] })
    }
    $names = @("$($identity.IdentityName)_${quad}_x64__$(Get-PublisherId $identity.Publisher)")
    $names += Get-AppxPackage -Name $identity.IdentityName | ForEach-Object { $_.PackageFullName }
    foreach ($full in ($names | Select-Object -Unique)) {
        Remove-AppxPackage -Package $full -ErrorAction SilentlyContinue
    }
    Add-AppxPackage -Path $test
    Write-Host 'Installed. Start Seam from the Start menu.'
}

# 9. A draft in Partner Center. Going live stays a click there.
if ($Upload) {
    if ($placeholder -or $identity.StoreId -like 'REPLACE*') { throw 'Fill in identity.json (IdentityName, Publisher, PublisherDisplayName, StoreId) before uploading.' }
    if (-not (Get-Command msstore -ErrorAction SilentlyContinue)) { throw 'msstore is missing. Install it: winget install "Microsoft Store Developer CLI"' }
    msstore publish $store --appId $identity.StoreId --noCommit
    if ($LASTEXITCODE -ne 0) { throw 'msstore publish failed.' }
    Write-Host "Draft uploaded. Look at it in Partner Center without editing it, then commit with: msstore submission publish $($identity.StoreId)"
}
