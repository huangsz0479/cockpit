[CmdletBinding()]
param(
  [string]$ProjectDirectory = (Get-Location).Path
)

$ErrorActionPreference = "Stop"
$targetDirectory = Join-Path $ProjectDirectory "src-tauri/target"

$applicationFiles = @(
  Get-ChildItem -Path $targetDirectory -Filter "cockpit.exe" -File -Recurse |
    Where-Object { $_.FullName -match "[\\/]release[\\/]cockpit\.exe$" }
)
$installerFiles = @(
  Get-ChildItem -Path $targetDirectory -Filter "*.msi" -File -Recurse |
    Where-Object { $_.FullName -match "[\\/]bundle[\\/]msi[\\/]" }
)

if ($applicationFiles.Count -eq 0) {
  throw "The built Cockpit executable was not found."
}

if ($installerFiles.Count -eq 0) {
  throw "The built MSI installer was not found."
}

$filesToVerify = @($applicationFiles + $installerFiles)
foreach ($file in $filesToVerify) {
  $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
  if ($signature.Status -ne "Valid") {
    throw "Invalid Authenticode signature for $($file.FullName): $($signature.StatusMessage)"
  }

  if ($null -eq $signature.TimeStamperCertificate) {
    throw "The Authenticode signature is not timestamped: $($file.FullName)"
  }

  Write-Host "Verified signed file: $($file.FullName)"
  Write-Host "Publisher: $($signature.SignerCertificate.Subject)"
}
