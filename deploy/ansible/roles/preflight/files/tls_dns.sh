#!/bin/sh
# Takes the TLS names as arguments. Prints one line per name that does NOT
# resolve to an address this host actually holds, and nothing at all when they
# all do.
#
# ACME validation is the name resolving, publicly, to this host. A name that
# points elsewhere does not make Caddy fail — it makes Caddy retry, forever,
# serving nothing, which is the shape of failure the role exists to refuse.
#
# A file rather than a line inside the task: the previous version was a shell
# one-liner with awk inside it, quoted through Jinja, YAML and sh, and the
# escaping lost `$4` so every name looked wrong.
set -u

mine=$(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | tr '\n' ' ')
[ -n "$mine" ] || { echo "(could not read this host's own addresses)"; exit 0; }

for name in "$@"; do
    got=$(getent ahostsv4 "$name" 2>/dev/null | awk '{print $1; exit}')
    case " $mine " in
        *" ${got:-__none__} "*) ;;
        *) echo "$name -> ${got:-(does not resolve)}; this host has: $mine" ;;
    esac
done
