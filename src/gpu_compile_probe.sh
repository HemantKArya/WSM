#!/usr/bin/env bash
set -euo pipefail
cd /d/MyProjects/wsm
S=(ssh -i work/ssh/id_wsm -p 2422 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o BatchMode=yes dev@127.0.0.1)
"${S[@]}" 'cat > /tmp/wsm-metaltest.swift' < work/x4/metaltest.swift
"${S[@]}" 'xcode-select -p; swiftc -O /tmp/wsm-metaltest.swift -o /tmp/wsm-metaltest && /tmp/wsm-metaltest' || rc=$?
scp -q -i work/ssh/id_wsm -P 2422 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR dev@127.0.0.1:/tmp/wsm-metaltest work/gpu-followup/metaltest
echo "Expected software baseline: nil Metal device. Probe exit=${rc:-0}"
