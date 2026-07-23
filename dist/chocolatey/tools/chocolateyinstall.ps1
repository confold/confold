$ErrorActionPreference = 'Stop'

$packageName = 'confold'
$url64       = 'https://github.com/confold/confold/releases/download/v0.6.1/Confold_0.6.1_x64-setup.exe'
$checksum64  = 'D38ED3425C425B094623C7C88DEEFD61E98A3D23C5C146902A6A373D89E875AE'

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
