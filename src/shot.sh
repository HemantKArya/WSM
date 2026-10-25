#!/usr/bin/env bash
# QMP screendump -> half-size PNG. usage: src/shot.sh OUT.png [QMP_PORT=4544]
cd "$(dirname "$0")/.."
t=$(mktemp -u).ppm; w=$(cygpath -m "$t")
.venv/Scripts/python.exe src/qmp.py ${2:-4544} screendump "{\"filename\":\"$w\"}" >/dev/null && ffmpeg -loglevel error -y -i "$t" -vf scale=960:-1 "$1" && rm -f "$t" && echo "$1"
