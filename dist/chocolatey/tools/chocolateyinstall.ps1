$ErrorActionPreference = 'Stop'

$packageName = 'confold'
$url64       = 'https://github.com/confold/confold/releases/download/v0.5.0/Confold_0.5.0_x64-setup.exe'
$checksum64  = 'FF051F5D8700FD394A55768427DF70878124E0CAF95DAE92D19884619E339CD9'

# Downloads the official Tauri NSIS installer, verifies its SHA256, and installs it silently.
# Chocolatey's auto-uninstaller picks up the Add/Remove Programs entry NSIS registers, so no
# explicit uninstall script is needed.
$packageArgs = @{
  packageName    = $packageName
  fileType       = 'exe'
  url64bit       = $url64
  checksum64     = $checksum64
  checksumType64 = 'sha256'
  silentArgs     = '/S'
  validExitCodes = @(0)
}

Install-ChocolateyPackage @packageArgs
