# Security policy

## Reporting a vulnerability

Please report security issues privately:

1. Open a [GitHub Security Advisory](https://github.com/pjourney/osint-rat/security/advisories/new) on this repository, or contact the maintainers directly.
2. Include what you were doing, the os-int version (`os-int --version`), and the smallest steps that show the problem.
3. Redact investigation data, tokens, and cookies. A synthetic fixture is enough.

Do not file a public issue for a vulnerability that could be exploited.

We will acknowledge receipt as soon as we can and work on a fix before any public disclosure.

CI runs `cargo audit` on every pull request. Dependabot opens weekly PRs for Cargo and GitHub Actions. That does not replace a private report for a vulnerability in os-int itself.

## What os-int does

os-int is a local investigation notebook for observation, monitoring, and diagnosis. It:

- stores data on the machine that runs it
- fetches public URLs, DNS and RDAP records, and certificate-transparency search results the analyst asks it to fetch
- runs local agent CLIs and passive OSINT utilities as subprocesses
- records when a later collection of the same public record has changed

Built-in collectors perform ordinary DNS queries, RDAP lookups, a single HTTP(S) fetch of a user-specified URL, a TLS handshake, a query to the public crt.sh search, inspection of public git metadata, inspection of a local file, public profile URL checks, and passive subdomain expansion. Diagnostics describe published records (mail policy, response headers, certificate validity, DNSSEC, and `security.txt`). File ingest (STIX, MISP, CSV, Evidence Batch) treats the file as untrusted: size-capped, decoded without panicking, sanitized before display. Optional API connectors redact keys in logs and source content.

## What the process boundary covers

Subprocesses are constrained by:

- no shell — arguments are an explicit argv
- timeouts and bounded output
- process-group kill on cancellation
- refusal of tools and sidecars whose configuration sets `activity = "active"`
- for Claude Code, `--tools ""`, so that CLI's built-in tools are disabled
- for Codex, `--sandbox read-only`

os-int does not sandbox arbitrary custom tools beyond those controls. A custom tool runs with the privileges of the user who launched os-int.

Collected material is sanitized before it is drawn in the terminal, and wrapped in a per-request `<<<UNTRUSTED …>>>` fence before it is shown to an agent. Accepting an agent proposal is a separate, explicit step. Accepted agent findings keep provenance `agent`.

## Please report

| Area | Examples |
| --- | --- |
| Path handling | Path traversal in import, export, or snapshot storage |
| Command execution | Command injection via argument templates or agent argv |
| Terminal safety | Escape-sequence injection that survives sanitization |
| Trust boundary | Prompt injection that causes os-int itself (rather than the model) to treat untrusted content as instructions |
| Secrets | Accidental logging of secrets or the process environment |

## Usually out of scope

- The accuracy of third-party OSINT data
- Model-level prompt injection that remains labelled as untrusted evidence and cannot drive os-int actions on its own
- Issues that require the analyst to configure an obviously dangerous custom tool
- Availability or content of crt.sh, RDAP registries, or other public services

## Secrets

Built-in collectors do not need API keys. Optional URLScan / OTX wrappers read `api_key_env` (preferred) or `api_key`; prefer the environment variable. Avoid putting tokens in `config.toml` argument lists. Values that match common secret patterns are redacted in job displays and logs. Redaction is best-effort, and a token in an argument is still passed to the program you configured.

## See also

- [User guide — using it responsibly](docs/guide.md#using-it-responsibly)
- [Architecture — principles](docs/ARCHITECTURE.md#principles)
- [Contributing](CONTRIBUTING.md)
