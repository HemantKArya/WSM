#!/usr/bin/env bash
# ssh into the full Tahoe (reims) guest as dev: src/tssh.sh [command...]
cd "$(dirname "$0")/.."
exec ssh -i work/ssh/id_wsm -p ${TSSH_PORT:-2322} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=3 -o BatchMode=yes dev@127.0.0.1 "$@"
