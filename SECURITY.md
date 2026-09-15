# Security Policy

Toglet stores and replaces Codex login credentials on the user's machine. A defect here can
expose an account, so security reports are treated as the highest priority class of issue.

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

Report it privately through GitHub: open the repository's **Security** tab and choose
**Report a vulnerability**. That opens a private advisory only the maintainers can read. If the
form is unavailable to you, open a public issue that says only that you have a security report
and asks for a private channel — no details.

A report is most useful when it includes:

- affected version and platform (Windows / macOS),
- what an attacker can achieve,
- reproduction steps,
- **no real credentials** — redact tokens, `auth.json` contents and full e-mail addresses.

Expect an acknowledgement within a few days. Fixes for confirmed reports are released before the
advisory is published.

## Scope

In scope:

- Credential storage, encryption and the temporary decryption directory.
- The atomic replacement, verification and rollback path for the default `auth.json`.
- File and directory permissions created by Toglet.
- Leakage of secrets into logs, error messages, the switch journal or diagnostics.
- Subprocess invocation and argument handling.
- The Tauri command surface exposed to the frontend.

Out of scope:

- Vulnerabilities in Codex itself or in the OpenAI service — report those to their vendor.
- Issues that require an attacker to already have code execution as the same OS user, which
  is outside Toglet's threat model (its protection boundary is other users on the machine
  and accidental disclosure, not a fully compromised account).

## Design commitments

These hold regardless of any individual report:

- Credentials are never stored in plaintext outside the Codex-managed default `auth.json`;
  there is no plaintext fallback if the OS credential store is unavailable.
- Tokens, `auth.json` contents, full e-mail addresses, OAuth callback parameters and
  absolute paths never reach logs, error messages, the journal or the frontend.
- Permissions are applied before content is written, on every path.
- No telemetry, analytics or crash-reporting service is used, and no user data leaves the
  machine.
- There is no interface that exports credentials in plaintext.
