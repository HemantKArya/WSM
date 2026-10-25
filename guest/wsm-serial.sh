#!/bin/sh
# com.wsm.serial: login shell on the serial console (QEMU chardev <-> `wsm shell`), no SSH needed.
# The boot-args `serial=3` make the kernel console the first serial port, so launchd gives us
# /dev/console as stdio. (There is no /dev/tty.serial* node: no driver claims QEMU's ISA UART, and
# launchd then fails the job with EX_CONFIG.) login -f skips the password prompt: the console is
# only reachable from the owner's Windows session through an owner-only socket file.
USER_NAME=$(cat /var/db/wsm/user 2>/dev/null)
# before first-boot completes there is no user yet: give a root shell so a broken boot is debuggable
[ -z "$USER_NAME" ] && USER_NAME=root
stty -f /dev/console sane 2>/dev/null
exec /usr/bin/login -fp "$USER_NAME"
