# remote-bridge

A reference bridge. **Not part of Toglet** — see `../README.md`.

```sh
node bridge.mjs              # 127.0.0.1:8787
PORT=9000 HOST=0.0.0.0 node bridge.mjs
```

Zero dependencies. Everything is in memory and is lost when it stops, which is deliberate: a
command nobody collected should expire.

## Three paths

| Path            | Who calls it | What happens                                                                                                                                                             |
| --------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `POST /toglet`  | Toglet       | Stores the receipt; answers with one queued command or `null`. **Holds the request for up to 25 seconds** when the queue is empty, which makes Toglet's poll a long poll |
| `GET /status`   | the page     | The last receipt verbatim, plus how many seconds old it is                                                                                                               |
| `POST /command` | the page     | Queues one signed command and wakes a parked poll                                                                                                                        |

## Deploying

The deployment is Docker Compose: the bridge, plus a Caddy that fetches the certificate and
serves the phone page from the same domain. It installs nothing on the host and touches nothing
outside its own directory. You need Docker Compose v2 and an A record pointing at the machine.

### One file

`../pack.sh` packs everything into a single self-extracting script, and the release workflow
attaches it to every GitHub release. On the server:

```sh
curl -fsSL https://github.com/leazoot/toglet/releases/latest/download/toglet-bridge-install.sh \
  | bash -s -- bridge.example.com
```

Or build it from a checkout and copy it over:

```sh
cd examples && ./pack.sh                                  # writes toglet-bridge-install.sh
scp toglet-bridge-install.sh root@YOUR_SERVER:
ssh root@YOUR_SERVER 'bash toglet-bridge-install.sh bridge.example.com'
```

It unpacks into `/opt/toglet` (`TOGLET_DIR=` to choose), then runs `up.sh`, which asks for
whatever you did not pass, writes `.env` and starts the stack. Piped in, it cannot ask, so pass
the domain as an argument.

### Or the whole folder

**Copy all of `examples/`**, not just this directory: Compose mounts `bridge.mjs`, `Caddyfile`
and `compose.yaml` from here, and the phone page from `../pwa`. `up.sh` stops and names anything
missing.

```sh
cd <the repository>
tar -czf - examples | ssh root@YOUR_SERVER \
  'mkdir -p /opt/toglet && tar -xzf - -C /opt/toglet --strip-components=1'
```

Then, on the machine:

```sh
cd /opt/toglet/remote-bridge
./up.sh
```

It asks for the domain, offers a random path prefix (press enter to take it), writes `.env` and
starts the stack. Both answers can be arguments instead — `./up.sh bridge.example.com` or
`./up.sh bridge.example.com a1b2c3d4` — so it works without a terminal too.

Afterwards it is plain Compose: `docker compose logs -f`, `ps`, `restart`, `down`. Re-running
`up.sh` keeps the existing prefix, because changing it unpairs every phone.

### With nothing else on the machine

Caddy gets the certificate by itself and serves both halves from the one domain:

|                   |                                                               |
| ----------------- | ------------------------------------------------------------- |
| open on the phone | `https://DOMAIN/PREFIX/app/` — then add it to the home screen |
| into Toglet       | `https://DOMAIN/PREFIX/toglet`                                |
| into the phone    | `https://DOMAIN/PREFIX/`                                      |

### If the machine already has a reverse proxy

Then it already owns 80 and 443, and a second Caddy cannot start. `up.sh` detects that, starts
the bridge alone and prints the block to add to your proxy. The bundled Caddy is behind a
profile, so `docker compose up -d` never starts it; `docker compose --profile standalone up -d`
does.

The bridge is published on `127.0.0.1:8787` (`BRIDGE_PORT` to change it) for a proxy installed on
the host. **A proxy that runs in a container cannot reach that** — `127.0.0.1` inside it is its
own loopback. Put it on this stack's network instead:

```sh
docker network connect remote-bridge_default YOUR_PROXY_CONTAINER
# then proxy to `bridge:8787`, and mount examples/pwa to serve the page
```

You can also run `bridge.mjs` without Compose, behind any `https` proxy — Toglet refuses plain
`http` to anything but loopback.

### Proxy requirements

Whatever proxy you use, two settings are not optional: **do not buffer the response** and **do
not cut idle connections under 30 seconds**. The bridge holds a poll open for 25 seconds; a proxy
that breaks that turns the long poll into a slow one, and the phone's button takes up to twenty
seconds to do anything. Caddy: `flush_interval -1`. nginx: `proxy_buffering off` with
`proxy_read_timeout 90s`.

## Security notes

- **The shared secret is not set up here.** The bridge relays bytes it can neither read nor
  sign, and putting the secret on the machine you treat as untrusted would defeat that. Make it
  on your own computer (`openssl rand -base64 24`) and type it into Toglet and the phone.
- **`PREFIX` is a capability URL, not authentication.** It keeps out scanners and does nothing
  against somebody who has the address. They still cannot forge a command — the signature is
  checked inside Toglet — but they can fill the queue and crowd yours out.
- **Logging.** This process writes no receipts or commands to disk. If you add logging, remember
  the receipt says what your task is doing.

## What was verified

The same `compose.yaml`, `Caddyfile` and `bridge.mjs`, brought up on a plain port so that no
certificate was involved:

|                                         |                                             |
| --------------------------------------- | ------------------------------------------- |
| `/PREFIX/status` through the proxy      | answers                                     |
| `/PREFIX/app/`, `app.js`, the manifest  | served                                      |
| anything outside the prefix             | `404`                                       |
| an empty poll                           | **held 25s** — the proxy does not buffer it |
| a command posted while a poll is parked | **came back in 3s**, not at the 25s mark    |

The packed installer was unpacked into an empty directory: all nine files byte-identical to
their sources, `.env` written, Compose called.

`up.sh` was run against a stubbed `docker`: both answers as arguments, both typed at a terminal,
a re-run that keeps the existing prefix, and each of the three refusals — no domain, an invalid
domain, a prefix containing `/` — none of which leave an `.env` behind.

**Not verified:** the certificate. That needs a real domain pointing at a real machine.

## Notifications

This bridge does not send Web Push. Toglet's notification channels already reach a phone — Bark,
Telegram, a webhook of your own. If you want push, point a Toglet **webhook channel** at your own
endpoint and send from there. The body is `{"source":"toglet","title":…,"body":…}`.
