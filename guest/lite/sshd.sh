#!/bin/sh
# com.wsm.sshd: sshd in the foreground under launchd (a daemon started from the run-once init job
# would be killed with that job's process group). Waits for init.sh to write this boot's config.
R=/System/Library/WSM/run
while [ ! -f $R/sshd_config.ready ]; do sleep 0.2; done
exec /usr/sbin/sshd -D -f $R/sshd_config -E $R/sshd.log
