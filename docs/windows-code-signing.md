# Windows code signing

Cockpit publishes Windows releases as MSI packages only. Release workflows fail
closed when a trusted Authenticode certificate is unavailable or when either the
application executable or MSI signature cannot be verified.

## Required GitHub configuration

Add these Actions secrets to the repository or its protected release environment:

- `WINDOWS_CERTIFICATE`: Base64-encoded PFX code-signing certificate. Generate it
  with `certutil -encode certificate.pfx certificate-base64.txt` and store the
  complete contents of `certificate-base64.txt`.
- `WINDOWS_CERTIFICATE_PASSWORD`: Password used to export the PFX.

The PFX must include its private key and the Code Signing extended key usage
(`1.3.6.1.5.5.7.3.3`). SSL/TLS and self-signed certificates are not valid for
public Windows releases.

The optional Actions variable `WINDOWS_TIMESTAMP_URL` overrides the default RFC
3161 timestamp service at `http://timestamp.digicert.com`.

## Repair an existing release without moving its tag

Run the `Repair Windows release assets` workflow and enter the existing tag. The
workflow rebuilds that tag's source, signs and verifies the application and MSI,
removes the unsigned NSIS installer, and replaces the MSI release asset. It never
moves or recreates the Git tag.
