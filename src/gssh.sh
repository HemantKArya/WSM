#!/usr/bin/env bash
# ssh into the running WSM guest: src/gssh.sh [command...]
cd "$(dirname "$0")/.."
exec ssh -i work/ssh/id_wsm -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=3 root@127.0.0.1 "$@"
