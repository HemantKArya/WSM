#!/usr/bin/env bash
# WSM unified QEMU: aljus7/reims-vgpu (pinned) + tqmane QEMU (pinned) + WSM patches.
# Mirrors the upstream scripts/windows/build.ps1 release steps, using only the project-local
# toolchains under tools/ (MSYS2 UCRT64, rustup). Run with tools/msys64/usr/bin/bash.exe -l.
#
#   build-qemu.sh SRC_DIR OUT_DIR
#
# SRC_DIR is an aljus7/reims-vgpu checkout with vendor/qemu at its pinned commit.
# OUT_DIR receives qemu-system-x86_64.exe, qemu-img.exe, reims_vgpu_c.dll, ReimsVgpu.rom, build.json.
set -euo pipefail
SRC=$(cygpath -u "$1"); OUT=$(cygpath -u "$2")
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
PATCHES="$ROOT/src/patches"

export MSYSTEM=UCRT64
export HOME="${HOME:-$SRC/.work/home}"; mkdir -p "$HOME"
export USERPROFILE="$(cygpath -w "$SRC/.work/home")"; mkdir -p "$SRC/.work/home"
export PATH="$ROOT/tools/rustup/cargo/bin:/ucrt64/bin:/usr/bin:$PATH"
export RUSTUP_HOME="$(cygpath -w "$ROOT/tools/rustup/home")" CARGO_HOME="$(cygpath -w "$ROOT/tools/rustup/cargo")"
export RUSTUP_TOOLCHAIN=1.98.1-x86_64-pc-windows-gnu
export CARGO_TARGET_DIR="$(cygpath -w "$SRC/.work/target")"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$(cygpath -w /ucrt64/bin/clang.exe)"
export CC=clang CXX=clang++ RC=llvm-windres CARGO_BUILD_JOBS=6 PYTHONUTF8=1
export CMAKE_GENERATOR=Ninja CMAKE_TOOLCHAIN_FILE="$(cygpath -m "$SRC/scripts/windows/ucrt64.cmake")" CMAKE_BUILD_PARALLEL_LEVEL=6
export CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=symbols
export TEMP="$(cygpath -w "$SRC/.work/tmp")" TMP="$(cygpath -w "$SRC/.work/tmp")" TMPDIR="$SRC/.work/tmp"
mkdir -p "$SRC/.work/tmp" "$OUT"

cd "$SRC"
# 1. Option ROM and the Vulkan library (+ import lib and pkg-config file).
MSYS2_ARG_CONV_EXCL="*" make -C firmware OS=Windows_NT BUILD="$(cygpath -m "$SRC/.work/target")/firmware" rom
cargo xtask dist --platform vulkan --profile release

# 2. Our QEMU patches, in numeric order. They stack (later ones touch lines earlier ones added), so a
#    per-patch "already applied" check can't work: reset every file they touch to the pinned commit,
#    then apply the whole series.
touched=$(for p in "$PATCHES"/[0-9][0-9]-*.patch; do git -C vendor/qemu apply --numstat "$p" | cut -f3; done | sort -u)
git -C vendor/qemu checkout HEAD -- $touched
for p in "$PATCHES"/[0-9][0-9]-*.patch; do
    git -C vendor/qemu apply "$p"; echo "[wsm] applied: $(basename "$p")"
done

# 3. QEMU, configured exactly as upstream's release build.
mkdir -p "$SRC/build/qemu"; cd "$SRC/build/qemu"
if [ ! -f build.ninja ]; then
    PKG_CONFIG_PATH="$SRC/.work/target/dist-vulkan/lib/pkgconfig:${PKG_CONFIG_PATH:-}" \
    WINDRES=llvm-windres CC_LD=lld CXX_LD=lld "$SRC/vendor/qemu/configure" \
        --target-list=x86_64-softmmu --cc=clang --cxx=clang++ --extra-ldflags=-fuse-ld=lld \
        --enable-whpx --disable-kvm --disable-hvf --enable-pvg --enable-libusb --enable-sdl \
        --enable-slirp --disable-gtk --disable-vnc --disable-spice --disable-gnutls --disable-curses \
        --disable-nettle --disable-rust --disable-werror --disable-debug-info --disable-qom-cast-debug
fi
ninja -j6 qemu-system-x86_64.exe qemu-img.exe
for exe in qemu-system-x86_64.exe qemu-img.exe; do llvm-strip --strip-all "$exe"; cp "$exe" "$OUT/"; done
cp "$SRC/.work/target/dist-vulkan/lib/reims_vgpu_c.dll" "$SRC/.work/target/dist-vulkan/share/reims-vgpu/ReimsVgpu.rom" "$OUT/"

cd "$SRC"
{
    echo "{"
    echo "  \"reims\": \"$(git rev-parse HEAD)\","
    echo "  \"qemu\": \"$(git -C vendor/qemu rev-parse HEAD)\","
    echo "  \"patches\": {"
    first=1
    for p in "$PATCHES"/[0-9][0-9]-*.patch; do
        [ $first = 1 ] || echo ","
        printf '    "%s": "%s"' "$(basename "$p")" "$(sha256sum "$p" | cut -d' ' -f1)"; first=0
    done
    echo; echo "  },"
    echo "  \"files\": {"
    first=1
    for f in qemu-system-x86_64.exe qemu-img.exe reims_vgpu_c.dll ReimsVgpu.rom; do
        [ $first = 1 ] || echo ","
        printf '    "%s": "%s"' "$f" "$(sha256sum "$OUT/$f" | cut -d' ' -f1)"; first=0
    done
    echo; echo "  },"
    echo "  \"rustc\": \"$(rustc --version)\", \"clang\": \"$(clang --version | head -1)\""
    echo "}"
} > "$OUT/build.json"
echo "[wsm] done: $OUT"
