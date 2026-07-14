$ErrorActionPreference = 'Stop'

$packageName = 'confold'
$url64       = 'https://github.com/confold/confold/releases/download/v0.6.0/Confold_0.6.0_x64-setup.exe'
$checksum64  = '74EC138182382A352C9E75AB8FB7CDC302CA0E0DBA4889E89EE8AD8AB31570E6'

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
