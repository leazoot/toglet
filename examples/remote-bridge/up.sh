#!/usr/bin/env bash
# Asks the three questions, writes .env, and starts the stack. NOT PART OF TOGLET - see ../README.md.
#
#   ./up.sh                               asks for whatever is not already in .env
#   ./up.sh bridge.example.com            asks for the prefix and the status key
#   ./up.sh bridge.example.com a1b2       asks only for the status key
#   ./up.sh bridge.example.com a1b2 <key> asks nothing
#
# The status key also comes from the environment, which is how it is supplied when this runs
# with no terminal to ask at (piped from curl, or any unattended re-run):
#
#   STATUS_KEY=<64 hex> ./up.sh                      keeps the domain and prefix already in .env
#   STATUS_KEY=<64 hex> ./up.sh bridge.example.com a1b2
#
# An argument wins over the environment, which wins over what .env already holds.
#
# The values go into `.env` because every later `docker compose` command reads them from there.
# Installs nothing and touches no file outside this directory.

set -euo pipefail
cd "$(dirname "$0")"
HERE="$PWD"
PWA="$(cd ../pwa 2>/dev/null && pwd || printf '%s/../pwa' "$HERE")"

die() { printf '\033[31m%s\033[0m\n' "$*" >&2; exit 1; }

# Compose mounts files from this directory and the phone page from ../pwa. Name what is missing
# rather than letting a mount fail.
missing=()
for needed in compose.yaml Caddyfile bridge.mjs ../pwa/index.html ../pwa/app.js; do
  [[ -e "$needed" ]] || missing+=("$needed")
done
if [[ ${#missing[@]} -gt 0 ]]; then
  printf '\033[31mMissing: %s\033[0m\n' "${missing[*]}" >&2
  die "Copy the whole examples/ folder, not just this script. The phone page lives in ../pwa."
fi

command -v docker >/dev/null || die "Docker is not installed."
docker compose version >/dev/null 2>&1 || die "This needs Docker Compose v2 (docker compose ...)."

# Keep whatever is already set up: a re-run should not quietly change the deployment.
OLD_DOMAIN=""; OLD_PREFIX=""; OLD_KEY=""
if [[ -f .env ]]; then
  OLD_DOMAIN="$(sed -n 's/^DOMAIN=//p' .env | head -n1)"
  OLD_PREFIX="$(sed -n 's/^PREFIX=//p' .env | head -n1)"
  OLD_KEY="$(sed -n 's/^STATUS_KEY=//p' .env | head -n1)"
fi

DOMAIN="${1:-}"
PREFIX="${2:-}"
# Argument, else the environment, else asked for below. Deliberately only the status key: on a
# re-run .env already carries the domain and the prefix, and an exported DOMAIN quietly
# repointing an existing deployment would be a worse surprise than the one this solves.
STATUS_KEY="${3:-${STATUS_KEY:-}}"

ask() { # prompt, current value -> answer on stdout
  local reply
  if [[ -t 0 ]]; then
    # Read stdin, not /dev/tty, so the script can still be driven non-interactively.
    read -r -p "$1" reply
    printf '%s' "${reply:-$2}"
  else
    printf '%s' "$2"
  fi
}

if [[ -z "$DOMAIN" ]]; then
  DOMAIN="$(ask "Domain${OLD_DOMAIN:+ [$OLD_DOMAIN]}: " "$OLD_DOMAIN")"
fi
[[ -n "$DOMAIN" ]] || die "A domain is needed. An A record for it must already point here."
[[ "$DOMAIN" =~ ^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$ ]] || die "That does not look like a domain: $DOMAIN"

if [[ -z "$PREFIX" ]]; then
  # Offer the existing prefix as the default: changing it unpairs every phone.
  if [[ -n "$OLD_PREFIX" ]]; then
    PREFIX="$(ask "Path prefix [$OLD_PREFIX, keep it - changing it unpairs your phone]: " "$OLD_PREFIX")"
  else
    suggested="$(openssl rand -hex 12 2>/dev/null || head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')"
    PREFIX="$(ask "Path prefix [$suggested]: " "$suggested")"
  fi
fi
# A slash or space in the prefix would silently break the Caddyfile.
[[ "$PREFIX" =~ ^[A-Za-z0-9_-]+$ ]] || die "The prefix may only hold letters, digits, - and _: $PREFIX"

# The status key authenticates a `/status` read, because the last receipt can carry a sealed
# excerpt of your session and handing that to whoever asks is a leak.
#
# It is NOT the shared secret, and this script deliberately will not compute it for you: doing so
# would require the secret on this machine, which is the one thing the design keeps off it. It is
# a one-way derivation, so what lands here cannot sign a command or open an excerpt.
if [[ -z "$STATUS_KEY" ]]; then
  if [[ -n "$OLD_KEY" ]]; then
    STATUS_KEY="$(ask "Status key [$OLD_KEY, keep it]: " "$OLD_KEY")"
  elif [[ ! -t 0 ]]; then
    # Piped in, so there is nobody to ask, and nothing stored to fall back on. Say exactly what
    # to do rather than failing the hex check below with an empty value.
    die "No status key, and no terminal to ask at.

  Toglet shows it: settings -> phone remote, next to the paired bridge, with a copy button.

  Or derive it ON YOUR OWN COMPUTER from your shared secret:

      printf '%s' 'toglet-remote/2 status<YOUR SECRET>' | shasum -a 256 | cut -d' ' -f1

  Then pass it in either way:

      STATUS_KEY=<64 hex> bash toglet-bridge-install.sh $DOMAIN $PREFIX
      bash toglet-bridge-install.sh $DOMAIN $PREFIX <64 hex>"
  else
    cat <<'HOWTO'

  One more value: the status key.

  Easiest: Toglet shows it. Settings -> phone remote, next to the paired bridge, with a copy
  button. Copy it from there and paste it here.

  Or derive it yourself ON YOUR OWN COMPUTER, with your shared secret. Either way, do not type
  the secret itself on this machine.

      printf '%s' 'toglet-remote/2 status<YOUR SECRET>' | shasum -a 256 | cut -d' ' -f1

  (Linux without shasum: `sha256sum` instead. Note there is no space before <YOUR SECRET>.)

HOWTO
    STATUS_KEY="$(ask "Status key: " "")"
  fi
fi
# Says the length, never the value: this is a key, and a refusal often means one mistyped
# character in an otherwise real one. Piped installs have their output captured.
[[ "$STATUS_KEY" =~ ^[0-9a-fA-F]{64}$ ]] \
  || die "The status key must be 64 hex characters (got ${#STATUS_KEY}). Not echoing it - it is a key."
STATUS_KEY="$(printf '%s' "$STATUS_KEY" | tr '[:upper:]' '[:lower:]')"

cat > .env <<ENV
# Written by up.sh. All three are read by compose.yaml; edit them here or re-run up.sh.

# An A record for this must point at this machine.
DOMAIN=$DOMAIN

# A path nobody can guess. Not authentication - it is a capability URL: it stops the scanning
# that finds every open port on the internet within the hour, and does nothing against somebody
# who has the address. Either way a command cannot be forged; the signature is checked inside
# Toglet. What this protects is the queue.
#
# Changing it unpairs every phone.
PREFIX=$PREFIX

# Authenticates a \`/status\` read. NOT the shared secret - it is
# SHA-256("toglet-remote/2 status" + secret), derived on your own machine. One-way: somebody who
# takes this file still cannot sign a command or open a sealed excerpt. They could read your
# status, so treat it as a password, but the zero-trust property survives.
STATUS_KEY=$STATUS_KEY
ENV

# Something already on 80 or 443 means an existing reverse proxy. A second one cannot bind the
# ports, so start the bridge alone and use that proxy.
taken=""
for port in 80 443; do
  if (command -v ss >/dev/null && ss -ltn "sport = :$port" 2>/dev/null | grep -q LISTEN) \
    || (command -v lsof >/dev/null && lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1); then
    taken="$taken $port"
  fi
done

printf '\nStarting\n'
if [[ -n "$taken" ]]; then
  printf 'Port%s%s already in use - starting the bridge only, without a proxy of its own.\n' \
    "$([[ "$taken" == *" "*" "* ]] && printf 's')" "$taken"
  docker compose up -d
  BEHIND_PROXY=1
else
  docker compose --profile standalone up -d
  BEHIND_PROXY=0
fi

if [[ "$BEHIND_PROXY" == "1" ]]; then
  cat <<PROXY

  Add this to the reverse proxy you already run, then reload it.

  Caddy:

    $DOMAIN {
      handle_path /$PREFIX/app/* {
        root * $PWA
        file_server
      }
      handle_path /$PREFIX/* {
        reverse_proxy 127.0.0.1:${BRIDGE_PORT:-8787} {
          flush_interval -1
          transport http { read_timeout 90s; write_timeout 90s }
        }
      }
    }

  nginx: proxy_pass to 127.0.0.1:${BRIDGE_PORT:-8787}, with
         'proxy_buffering off' and 'proxy_read_timeout 90s'.

  Those two settings are not decoration: the bridge holds a poll open for 25 seconds, and a
  proxy that buffers the reply or hangs up early turns it into a slow poll - the phone's button
  then takes twenty seconds to do anything.

  If your proxy runs in a container it cannot reach 127.0.0.1 here. Put it on this network
  instead and use the name 'bridge':

      docker network connect ${COMPOSE_PROJECT_NAME:-remote-bridge}_default YOUR_PROXY_CONTAINER
      # then: reverse_proxy bridge:8787   (and mount $PWA for the page)

PROXY
fi

cat <<DONE

  Open on the phone      https://$DOMAIN/$PREFIX/app/     (share -> add to home screen)

  Then pair, with the same secret on both sides:

    into Toglet          https://$DOMAIN/$PREFIX/toglet
    into the phone       https://$DOMAIN/$PREFIX/

  Make the secret on your own computer, never here:

      openssl rand -base64 24

  The bridge cannot sign a command: that MAC uses the shared secret, which never reaches this
  machine. Generating the secret here would put it somewhere the design treats as untrusted.

  What this machine does hold is the status key you pasted - a one-way derivation of the secret.
  It lets the bridge tell your phone apart from a stranger when something reads /status. It
  cannot be turned back into the secret, cannot sign a command, and cannot open a sealed excerpt.

  docker compose logs -f      what it is doing
  docker compose ps           what is running
  docker compose down         stop it

DONE
