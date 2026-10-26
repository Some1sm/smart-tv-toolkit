# Build the Windows release and zip it with its license files.
# Run from the repo root (in a Developer PowerShell, so the MSVC tools are set):
#   .\scripts\package-windows.ps1
# Optional: THIRD-PARTY-LICENSES.html in the repo root is included in the zip.
$ErrorActionPreference = "Stop"

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Output "Building Smart TV Toolkit $version for Windows..."
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$stage = "dist\SmartTVToolkit-$version-windows-x64"
Remove-Item -Recurse -Force dist -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item target\release\smart-tv-toolkit.exe "$stage\Smart TV Toolkit.exe"
Copy-Item LICENSE, NOTICE.md $stage
if (Test-Path THIRD-PARTY-LICENSES.html) { Copy-Item THIRD-PARTY-LICENSES.html $stage }

$zip = "dist\SmartTVToolkit-$version-windows-x64.zip"
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Remove-Item -Recurse -Force $stage
Write-Output "Done: $zip"
