# Security Policy

## Supported Versions

Only the [latest release](https://github.com/Vianpyro/sunrise-brightness/releases/latest) receives
security fixes. The app tells you when a new version is out; a fix ships as a new patch release.

## Reporting a Vulnerability

Please do not open a public issue. Report it privately through
[GitHub's private vulnerability reporting](https://github.com/Vianpyro/sunrise-brightness/security/advisories/new),
with the version, your platform, and the steps to reproduce it.

You should get an answer within a week. If the report is confirmed, the fix is released as soon as
it is ready and the advisory is published with it, crediting you unless you prefer otherwise. If it
is declined, you will be told why.

## What the pipeline checks

- Known vulnerabilities in the locked crates, every night (`cargo deny`, `nightly.yml`), and in the
  crates a pull request adds (`dependency-review`).
- Secrets in every commit (`gitleaks`), CodeQL on the Rust code and the workflows, and the
  OpenSSF Scorecard.
- Release binaries carry build provenance attestations, and every release lists their SHA-256.
