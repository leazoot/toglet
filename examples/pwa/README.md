# pwa

The page you add to your phone's home screen. **Not part of Toglet** — see `../README.md`.

No build step and no dependencies. Serve the folder over `https` (or `http://localhost` while
you are trying it out — Web Crypto needs a secure context, and without it nothing can be signed):

```sh
npx serve .          # or python3 -m http.server, behind a TLS proxy
```

Then open it on the phone, add it to the home screen, and pair it with the same bridge address
and secret you gave Toglet.

## What it does

One screen. It asks the bridge what Toglet last said, and it can send four commands: continue,
pause, cancel, status. Each one is signed with HMAC-SHA256 against your secret using the phone's
own Web Crypto — **the secret never leaves the phone** and the bridge never sees it.

## Never show a button that would not work

When the session is waiting on a Codex approval, a remote continue cannot help: Toglet would
re-read the thread and land back on the same state. So on that screen the continue button does
not exist — not greyed out, not disabled. The same goes for a stop that only somebody at the
computer can clear: an expired sign-in, the continuation limit, a deadline.

- **A command's last step lights only when Toglet confirms it** — not when the bridge accepts
  it, and not on a timer. Until then the page says the computer may be asleep.
- **An expired status is shown as unknown**, with the old state labelled expired. A status is
  expired once it is older than `max(120, nextPollSeconds + 90)` seconds: one missed poll plus a
  grace period, since Toglet's poll interval depends on its state.

## Language

The page follows the phone's language (Chinese or English). Toglet sends stable codes, never
sentences, so the page renders its own words — which is why a notification sent by Toglet, in
the desktop's language, can be worded differently. Both dictionaries are at the top of `app.js`
and must carry the same keys.

## What is stored on the phone

The bridge address and the secret, in `localStorage`. Nothing else. Clearing the site data
unpairs it; pair again with the same secret and it picks up where it was, because the counter it
needs comes back in the next status.

## Verification

Each state — unpaired, running, waiting, stopped, needs-you, paused, command-in-flight and
unreachable — was rendered in a headless browser at 390 × 844, in both themes and both
languages, and checked for horizontal overflow at widths from 360 to 430.

**Not verified:** a real phone, `standalone` safe areas, and the light theme on a device.
