$ErrorActionPreference = 'Stop'

$packageName = 'confold'
$url64       = 'https://github.com/confold/confold/releases/download/v0.5.1/Confold_0.5.1_x64-setup.exe'
$checksum64  = '950B6E0C673A82C73CA9793CCC753F59906FAACF9F57FED1E17AA644B53D8B7A'

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
