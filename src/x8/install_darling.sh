#!/bin/bash
# Install Darling CLI debs (skip darling-gui: needs Ubuntu noble ffmpeg-6 libs absent on Debian 13).
cd /opt/wsm-darling/debs_20260609 || exit 1
pkgs=()
for f in *.deb; do case "$f" in darling-gui_*|darling_*) ;; *) pkgs+=("./$f");; esac; done
DEBIAN_FRONTEND=noninteractive apt-get install -y -q "${pkgs[@]}" 2>&1 | tail -12
command -v darling && darling --version 2>&1 | head -2
