<div align="center">

<img src="src-tauri/icons/128x128.png" width="72" height="72" alt="" />

# Toglet

**Your Codex quota, always in the corner of your eye. Account switching, only when you ask.**

[![CI](https://github.com/leazoot/toglet/actions/workflows/ci.yml/badge.svg)](https://github.com/leazoot/toglet/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-6b7280)](https://github.com/leazoot/toglet/releases)
[![License](https://img.shields.io/badge/license-MIT-6b7280)](LICENSE)

**English** · [简体中文](README.zh-CN.md)

</div>

---

Toglet is a small desktop app for Windows and macOS. It rests against the edge of your screen and
shows the five-hour and weekly quota of the Codex account you are signed in to. Move the pointer
onto it and a panel opens with every account you have added, each with its own numbers. Pick one,
confirm, and Toglet exchanges the sign-in for you.

Nothing switches on its own, and nothing leaves your machine.

Toglet is young. The Windows build is the one in daily use; the macOS build is newer and has
seen less mileage.

## Why

Codex quota is a number you want often and can only get from a terminal. And if you keep more than
one account, changing which one Codex uses means editing the file it authenticates with by hand —
an edit that goes fine ninety-nine times and is expensive the hundredth.

Toglet keeps the number in sight, and turns the exchange into a deliberate, reversible operation:
copy the file aside, replace it in one step, ask Codex who it is now, and record the change only
if the answer agrees. If anything fails along the way, the previous sign-in comes back.

## What it does

- **Both windows at a glance.** Five-hour and weekly, for the account in use, without a terminal.
- **Every account in one panel.** Hover to open, click a row to switch, `Esc` to leave.
- **A switch you drive.** Pre-checks, a confirmation, four visible steps, verification, and a
  rollback that restores the previous sign-in if a step fails.
- **Credentials kept in the system store.** Keychain on macOS, DPAPI on Windows. No plain-text copy.
- **Numbers that admit what they are.** A quota that could not be read says so; it never shows 0%.
- **Placed where you want it.** Either screen edge, any monitor, light or dark, English or 中文.

## Install

Download the current build from [Releases](https://github.com/leazoot/toglet/releases).

| Platform                      | File             |
| ----------------------------- | ---------------- |
| Windows 10 / 11               | `.msi` or `.exe` |
| macOS, Apple silicon or Intel | `.dmg`           |

You need the [Codex CLI](https://developers.openai.com/codex/cli) installed and signed in at least
once. Toglet works with the accounts Codex already knows; it never asks you for a password, and it
accepts no pasted tokens.

The macOS build is not notarised yet, so the first launch needs Control-click → **Open**.

## How it works

**Reading a quota.** Toglet starts `codex app-server`, asks it for the account and its rate limits,
and shuts it down again. The account in use is read in Codex's own home directory. Every other
account is read in a temporary home, so looking up a quota can never disturb the sign-in you are
working with.

**Switching an account.** Copy `auth.json` aside → write the new one atomically → ask the server
which account is now signed in → record the change only when the identity matches → restore the
copy if it does not. A switch interrupted by a crash is repaired the next time Toglet starts.

**What it will not do.** Rotate accounts by itself, fail over when a quota runs out, share accounts
between machines, or report a success it has not verified.

## Build from source

Node 22+, pnpm 10, and Rust 1.94 (pinned in `rust-toolchain.toml`).

```bash
pnpm install
pnpm dev      # run the app
pnpm check    # format, lint, types, tests
pnpm build    # installers for the current platform
```

## Privacy

Toglet has no server, no account of its own, no telemetry and no analytics. The only network
traffic is what Codex itself makes when it reads a quota or signs you in. Everything Toglet keeps
stays in your user profile, and tokens are never written to logs, diagnostics or the clipboard.

Found a security problem? Please read [SECURITY.md](SECURITY.md) first.

## License

[MIT](LICENSE)
