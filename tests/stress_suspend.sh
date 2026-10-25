#!/usr/bin/env bash
# Suspend/resume stress: N cycles on a busy guest, checking that each resume continues the same boot.
# usage: WSM_HOME=... tests/stress_suspend.sh INSTANCE [CYCLES] [--busy]
# Same session = same kern.bootsessionuuid (kern.boottime shifts when wsmd resyncs the clock) and the
# /tmp marker survives. Also counts guest panic reports and QEMU `wsm-event` trace lines per save.
set -u
W=${WSM_BIN:-work/r3/bin/wsm.exe}; N=$1; CYCLES=${2:-10}; BUSY=${3:-}
export MSYS_NO_PATHCONV=1
LOGS="$WSM_HOME/instances/$N/logs"
g() { "$W" exec --name "$N" -- "$@"; }
if [ "$BUSY" = "--busy" ]; then
  # Detached from the SSH session (otherwise exec waits for the loops): memory + VM + FS churn.
  g 'for i in 1 2 3 4 5 6 7 8; do nohup sh -c "while :; do ls -R /usr/share >/dev/null 2>&1; dd if=/dev/zero of=/tmp/churn.\$\$ bs=1m count=64 2>/dev/null; rm -f /tmp/churn.\$\$; done" >/dev/null 2>&1 & done; echo busy'
fi
panics0=$(g 'ls /Library/Logs/DiagnosticReports | grep -c panic')
fail=0
for k in $(seq 1 "$CYCLES"); do
  u0=$(g 'sysctl -n kern.bootsessionuuid'); g "echo m$k > /tmp/wsm-marker"
  s=$("$W" suspend --name "$N" | tail -1)
  ev=$(grep -c "^wsm-event" "$LOGS/qemu.err.log" 2>/dev/null)
  r=$("$W" start --name "$N" | tail -1)
  sleep 10
  u1=$(g 'sysctl -n kern.bootsessionuuid'); mk=$(g 'cat /tmp/wsm-marker' 2>/dev/null)
  ok=yes; { [ "$u0" = "$u1" ] && [ "$mk" = "m$k" ]; } || { ok=NO; fail=$((fail+1)); }
  echo "cycle $k same_session=$ok events=$ev | $s | $r"
done
panics1=$(g 'ls /Library/Logs/DiagnosticReports | grep -c panic')
echo "SUMMARY cycles=$CYCLES failed=$fail new_panic_reports=$((panics1-panics0))"
[ "$BUSY" = "--busy" ] && g 'pkill -f "churn" ; pkill -f "ls -R" ; true' >/dev/null
