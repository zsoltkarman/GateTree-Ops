#!/bin/sh
set -eu

previous_pid=$1
bundle=$2
opener=$3
registrar=$4
remaining=$5
shift 5

# LaunchServices must see the old application as terminated before reopening it.
while /bin/kill -0 "$previous_pid" 2>/dev/null; do
  remaining=$((remaining - 1))
  if [ "$remaining" -le 0 ]; then
    echo 'The updated application was not reopened because the previous process is still running.' >&2
    exit 1
  fi
  /bin/sleep 0.1
done

# The updater replaced the bundle at the same path; refresh its OS registration.
if [ -x "$registrar" ]; then
  "$registrar" -f "$bundle" || echo 'Could not refresh application registration; trying normal open.' >&2
fi

# Do not force a new instance (-n), or execute Contents/MacOS directly.
if [ "$#" -gt 0 ]; then
  exec "$opener" -a "$bundle" --args "$@"
fi
exec "$opener" -a "$bundle"
