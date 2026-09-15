# examples

**Nothing in this folder is part of Toglet.**

It is not built, not typechecked, not a dependency of the application, and not packaged into
the installer. Prettier and ESLint's basic rules still cover it, but nothing here ships. Toglet
does not host, run or operate any of it, and ships no address pointing at it. These are
reference implementations of the _other end_ of a protocol Toglet speaks — you run them, on
your own machine, at your own risk.

| Folder           | What it is                                                                  |
| ---------------- | --------------------------------------------------------------------------- |
| `remote-bridge/` | A ~200-line Node service that relays commands between your phone and Toglet |
| `pwa/`           | A page you add to your phone's home screen, which sends those commands      |

## Why there is an "other end" at all

Toglet sits behind NAT with no inbound listener, so your phone cannot reach it. The direction is
reversed: Toglet polls a bridge you run, hands it a status receipt, and takes away at most one
command. The bridge only ferries bytes.

## The bridge is not trusted, and you should not trust it either

Every command is signed end to end with HMAC-SHA256 against a secret you type into Toglet and
into the page. The bridge holds neither. If somebody takes over your bridge completely, what
they can do is **drop your commands** — a denial of service. What they cannot do is forge one.

That property only holds if you keep two things true:

1. **The secret is long and is not reused.** Sixteen characters minimum; use a generated one.
2. **The bridge is reached over `https`.** Toglet refuses plain `http` to anything but loopback,
   so you need a real certificate — a reverse proxy in front of this is the normal way.

What this example does **not** do, and what you must decide for yourself: TLS termination,
authentication of who may POST to it, storage that survives a restart, rate limiting at the edge,
and logging you are happy to keep. It holds everything in memory and forgets it when it stops.

## The protocol

The wire format — envelope fields, the signed byte string and the result codes — is defined in
`src-tauri/src/remote/envelope.rs`. `remote-bridge/e2e.mjs` signs fixtures independently and
`src-tauri/tests/remote_wire.rs` checks them against Toglet's verifier, so the two cannot drift
apart unnoticed.
