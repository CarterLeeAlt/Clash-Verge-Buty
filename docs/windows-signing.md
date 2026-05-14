# Windows Signing Strategy

## Why Windows shows "Unknown Publisher"
Windows shows **Unknown Publisher** when an executable has no valid Authenticode signature (or the signature chain is not trusted).

## Unsigned build behavior in this repo
This repo supports unsigned fallback for both alpha and release Windows builds:
- Build succeeds without signing configuration.
- Portable artifact filename gets `_unsigned` suffix.
- Workflow summary explicitly warns that Unknown Publisher / SmartScreen prompts are expected.

## Why self-signed certs are not suitable for public release
Self-signed certificates are not trusted by default on end-user machines, so they do not solve Unknown Publisher for public distribution.

## Signing modes
Set GitHub **Repository Variable** `WINDOWS_SIGNING_MODE`:
- `unsigned` (default): no signing, unsigned artifacts.
- `pfx`: sign with your own PFX via `signtool`.
- `signpath`: sign via SignPath service (open-source friendly path).
- `ossign`: reserved integration path (placeholder step intentionally fails until OSSign command is configured).

If signing mode is explicitly set to `pfx/signpath/ossign` and required secrets are missing, workflow fails.

## PFX mode configuration
Required GitHub **Secrets**:
- `WINDOWS_CERTIFICATE_BASE64`
- `WINDOWS_CERTIFICATE_PASSWORD`

Optional: `WINDOWS_CERTIFICATE_SHA1` (not required by current scripts).

Convert `.pfx` to base64 (PowerShell):
```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("your-cert.pfx"))
```

## SignPath open-source mode
SignPath provides an open-source signing program (application/review required). Typical secrets:
- `SIGNPATH_API_TOKEN`
- `SIGNPATH_ORGANIZATION_ID`
- `SIGNPATH_PROJECT_SLUG`
- `SIGNPATH_SIGNING_POLICY_SLUG`
- `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG`

Current workflow includes a SignPath GitHub Action entry point for Windows executable signing.

## OSSign mode
OSSign is kept as an integration path in workflow design. The current step is a **deliberate placeholder** that fails with setup instructions until exact OSSign CLI/action parameters are provided for your approved project.

Suggested secrets for future OSSign wiring:
- `OSSIGN_CLIENT_ID`
- `OSSIGN_CLIENT_SECRET`
- `OSSIGN_PROJECT` (optional, depending on OSSign API/CLI)

## Signature verification
When signing is enabled, workflow runs:
```powershell
signtool verify /pa /v <file>
Get-AuthenticodeSignature <file>
```

## SmartScreen reputation vs signature presence
- **Signature presence**: determines whether Publisher can be identified.
- **SmartScreen reputation**: may still warn for newly signed binaries with low reputation.

So a signed file can still receive a "not commonly downloaded" warning; that is different from Unknown Publisher.
