# Security policy

## Reporting a vulnerability

Please report security issues privately through GitHub Security Advisories on this repository, or by contacting the maintainers. Do not file a public issue for vulnerabilities that could be exploited.

We will acknowledge receipt as soon as we can and work on a fix before any disclosure.

## What this software does and does not do

os-int is a local investigation notebook. It:

- stores data on the machine that runs it
- fetches URLs and DNS/RDAP records the analyst asks it to fetch
- runs local agent CLIs and OSINT utilities as subprocesses

It does not sandbox those subprocesses beyond: no shell, explicit argv, timeouts, output bounds, process-group kill, and (for Claude Code) `--tools ""`.

## Please report

- Path traversal in import/export or snapshot storage
- Command injection via argument templates or agent argv
- Terminal escape-sequence injection that survives sanitization
- Prompt-injection issues that cause os-int itself (not the agent model) to treat untrusted content as instructions
- Accidental logging of secrets or the process environment

## Out of scope (usually)

- The accuracy of third-party OSINT data
- Model-level prompt injection that remains labelled as untrusted evidence and cannot run tools
- Issues that require the analyst to configure an obviously dangerous custom tool

## Handling of secrets

os-int does not need API keys for built-in collectors. Do not put tokens in `config.toml` argument lists if you can avoid it; values matching common secret patterns are redacted in job displays and logs, but redaction is best-effort.
