#!/usr/bin/env bash
# Asks the two questions, writes .env, and starts the stack. NOT PART OF TOGLET - see ../README.md.
#
#   ./up.sh                          asks for both
#   ./up.sh bridge.example.com       asks only for the prefix
#   ./up.sh bridge.example.com a1b2  asks nothing
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
OLD_DOMAIN=""; OLD_PREFIX=""
if [[ -f .env ]]; then
  OLD_DOMAIN="$(sed -n 's/^DOMAIN=//p' .env | head -n1)"
  OLD_PREFIX="$(sed -n 's/^PREFIX=//p' .env | head -n1)"
fi

DOMAIN="${1:-}"
PREFIX="${2:-}"

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

cat > .env <<ENV
# Written by up.sh. Both are read by compose.yaml; edit them here or re-run up.sh.

# An A record for this must point at this machine.
DOMAIN=$DOMAIN

# A path nobody can guess. Not authentication - it is a capability URL: it stops the scanning
# that finds every open port on the internet within the hour, and does nothing against somebody
# who has the address. Either way a command cannot be forged; the signature is checked inside
# Toglet. What this protects is the queue.
#
# Changing it unpairs every phone.
PREFIX=$PREFIX
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

  The bridge relays bytes it can neither read nor sign. Generating the secret on this machine
  would put it somewhere the design treats as untrusted.

  docker compose logs -f      what it is doing
  docker compose ps           what is running
  docker compose down         stop it

DONE
