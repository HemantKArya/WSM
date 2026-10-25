#!/usr/bin/env bash
# Published Windows/Tahoe pair from MAXZVER issue #1. Isolated from round-2 builds.
set -euo pipefail
cd /d/MyProjects/wsm
export PATH="/d/MyProjects/wsm/tools/build/bin:/ucrt64/bin:/usr/bin:$PATH"
export CARGO_HOME=/d/MyProjects/wsm/tools/build/cargo-home
export CARGO_BUILD_JOBS=4 PYTHONUTF8=1 CARGO_NET_OFFLINE=true
cd "${1:-tools/gpu-followup/reims-vgpu}"
REIMS_VGPU_BACKEND=vulkan scripts/qemu-build/qemu-build.sh --target x86_64 --backend vulkan
