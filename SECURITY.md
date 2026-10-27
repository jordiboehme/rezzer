# Security policy

## Report a vulnerability

Please don't open a public issue for a security problem. Use GitHub's private vulnerability reporting instead:

1. Open the [Security tab](https://github.com/jordiboehme/rezzer/security) of this repository.
2. Click "Report a vulnerability".
3. Describe the problem and how to reproduce it. A sample file helps a lot.

Only the maintainer can see your report.

## Supported versions

Only the latest release gets security fixes. Please check that the problem still exists there before you report it.

## Check what you downloaded

Every release has these files next to the archives:

- `SHA256SUMS`: the SHA-256 checksums of the archives and the SBOMs
- `SHA256SUMS.sigstore.json`: a cosign signature of `SHA256SUMS`, made in GitHub Actions, with no key to keep
- `rezzer-v<version>-<platform>.cdx.json`: a CycloneDX SBOM for each platform, with the crates in that binary

GitHub also keeps a build provenance attestation for every file of the release and an SBOM attestation for each archive.

Check the checksum of your archive. On macOS use `shasum -a 256` instead of `sha256sum`. On Windows run `(Get-FileHash <file>).Hash.ToLower()` in PowerShell and compare the hash with the line in `SHA256SUMS`.

```sh
sha256sum -c SHA256SUMS --ignore-missing
```

Check the signature of the checksum file (cosign 3 or newer):

```sh
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp '^https://github\.com/jordiboehme/rezzer/\.github/workflows/release\.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
```

Check where a file was built:

```sh
gh attestation verify <file> --repo jordiboehme/rezzer
```

If a check fails, don't run the binary. Download it again. If it still fails, please report it as described above.

## How Rezzer treats input files

Rezzer reads files from unknown sources, so it treats every input file as untrusted:

- Rezzer never runs code from a file. Macros, scripts and embedded programs are not executed.
- Rezzer limits the size of a file, the depth of nested structures and the time it spends on one file. When a file goes over a limit, Rezzer stops and says so in the output.

A file that gets around one of these limits is a security problem. Please report it as described above.
