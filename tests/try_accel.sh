#!/usr/bin/env bash
# usage: try_accel.sh INSTANCE "ACCEL" [seconds]   -- cold boots INSTANCE with the given -accel and reports time to SSH
# Dev helper for P1: needs WSM_HOME and work/r3/bin/wsm.exe.
set -u
W=${WSM_BIN:-work/r3/bin/wsm.exe}; N=$1; ACC=$2; LIM=${3:-60}
M="$WSM_HOME/instances/$N/manifest.toml"
$W stop --kill --name "$N" >/dev/null 2>&1
sed -i "s|^accel = .*|accel = \"$ACC\"|" "$M"
t0=$(date +%s.%N)
$W start --cold --no-wait --name "$N" >/dev/null 2>&1
while :; do
  if $W exec --name "$N" true >/dev/null 2>&1; then echo "accel=[$ACC] ssh_after=$(awk "BEGIN{print $(date +%s.%N)-$t0}")s"; exit 0; fi
  [ "$(awk "BEGIN{print int($(date +%s.%N)-$t0)}")" -ge "$LIM" ] && { echo "accel=[$ACC] NO SSH in ${LIM}s"; exit 1; }
  sleep 1
done
