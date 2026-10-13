---
name: Bug report
about: Something in os-int did not work as expected
title: ""
labels: bug
---

## What happened

What did you see? Quote the status line, the job error, or the command output.

## What you expected

## Steps to reproduce

1.
2.
3.

A synthetic target (`example.com`, a file under `/tmp`) is ideal. Leave real investigation data out.

## Environment

`os-int doctor` output, with home-directory paths trimmed if you prefer:

```text

```

- os-int version (`os-int --version`):
- OS:
- Terminal and size (the layout changes below 96 columns):
- Install method: `cargo install --path .` / `cargo install --git` / other
- Agent CLI, if the bug involves Ask (`claude`, `codex`, or a custom profile):

## Logs

Logs are daily files under the data directory. `os-int config` prints the log directory (`~/.local/share/os-int/logs` on Linux).

Paste the relevant lines, or attach the file. Redact tokens, cookies, and personal data first. Pass `--debug` and retry once if the default log does not show the failure.

## Checklist

- [ ] I searched existing issues for the same failure
- [ ] This is a defect in os-int, not a question about a third-party OSINT result
