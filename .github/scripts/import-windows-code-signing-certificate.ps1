[CmdletBinding()]
param(
  [string]$ProjectDirectory = (Get-Location).Path
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE)) {
  throw "WINDOWS_CERTIFICATE is required and must contain the Base64-encoded PFX certificate."
}

if ([string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE_PASSWORD)) {
  throw "WINDOWS_CERTIFICATE_PASSWORD is required."
}

$encodedCertificatePath = Join-Path $env:RUNNER_TEMP "cockpit-windows-certificate.txt"
$certificatePath = Join-Path $env:RUNNER_TEMP "cockpit-windows-certificate.pfx"
$configPath = Join-Path $ProjectDirectory "src-tauri/tauri.windows.conf.json"

try {
  [System.IO.File]::WriteAllText(
    $encodedCertificatePath,
    $env:WINDOWS_CERTIFICATE,
    [System.Text.UTF8Encoding]::new($false)
  )

  & certutil.exe -f -decode $encodedCertificatePath $certificatePath | Out-Host
  if ($LASTEXITCODE -ne 0) {
    throw "Failed to decode WINDOWS_CERTIFICATE."
  }

  $securePassword = ConvertTo-SecureString `
    -String $env:WINDOWS_CERTIFICATE_PASSWORD `
    -AsPlainText `
    -Force
  $importedCertificates = @(
    Import-PfxCertificate `
      -FilePath $certificatePath `
      -CertStoreLocation "Cert:\CurrentUser\My" `
      -Password $securePassword
  )

  $codeSigningOid = "1.3.6.1.5.5.7.3.3"
  $signingCertificate = $importedCertificates |
    Where-Object {
      $_.HasPrivateKey -and
      (($_.EnhancedKeyUsageList | ForEach-Object { $_.ObjectId.Value }) -contains $codeSigningOid)
    } |
    Select-Object -First 1

  if ($null -eq $signingCertificate) {
    throw "The PFX does not contain a certificate with a private key and the Code Signing EKU."
  }

  $timestampUrl = $env:WINDOWS_TIMESTAMP_URL
  if ([string]::IsNullOrWhiteSpace($timestampUrl)) {
    $timestampUrl = "http://timestamp.digicert.com"
  }

  $windowsConfig = [ordered]@{
    bundle = [ordered]@{
      targets = @("msi")
      windows = [ordered]@{
        certificateThumbprint = $signingCertificate.Thumbprint
        digestAlgorithm = "sha256"
        timestampUrl = $timestampUrl
        tsp = $true
      }
    }
  }
  $configJson = $windowsConfig | ConvertTo-Json -Depth 5
  [System.IO.File]::WriteAllText(
    $configPath,
    $configJson,
    [System.Text.UTF8Encoding]::new($false)
  )

  Write-Host "Imported Windows code-signing certificate: $($signingCertificate.Subject)"
  Write-Host "Configured certificate thumbprint: $($signingCertificate.Thumbprint)"
}
finally {
  Remove-Item -LiteralPath $encodedCertificatePath -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $certificatePath -Force -ErrorAction SilentlyContinue
}
