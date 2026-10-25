#!/usr/bin/env bash
# G3 tool checks on the (already running) full-OS variant over SSH as dev (port 2322).
# Re-runs the round-1 failures without workarounds. Prints one timed line per step.
cd "$(dirname "$0")/.."
S="ssh -i work/ssh/id_wsm -p 2322 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR dev@127.0.0.1"
t() { local s=$(date +%s); "$@"; echo "[$(( $(date +%s)-s ))s] ${*: -1}" | cut -c1-120; }
D=work/dl-clt
echo "== copy CLT pkgs (scp)"
t scp -q -i work/ssh/id_wsm -P 2322 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
  $D/CLTools_Executables_Universal.pkg $D/CLTools_macOSLMOS_SDK.pkg $D/CLTools_macOSNMOS_SDK.pkg dev@127.0.0.1:/tmp/
echo "== installer -pkg (round 1: refused, free space on the 1.4 GB root)"
t $S 'for p in CLTools_Executables_Universal CLTools_macOSLMOS_SDK CLTools_macOSNMOS_SDK; do sudo installer -pkg /tmp/$p.pkg -target / 2>&1 | tail -1; done'
echo "== toolchain"
$S 'sudo xcode-select -s /Library/Developer/CommandLineTools; xcode-select -p; clang --version | head -1; swift --version 2>&1 | head -1; git --version; ls /Library/Developer/CommandLineTools/SDKs | tr "\n" " "; echo'
echo "== compile C + Swift"
$S 'cd /tmp && printf "#include <stdio.h>\nint main(void){puts(\"hello\");return 0;}\n" > h.c && clang -O2 h.c -o h && ./h; echo "print(\"swift ok\")" > h.swift && swiftc -O h.swift -o hs && ./hs'
echo "== Homebrew as non-root dev (round 1: needed /etc/passwd hack, own volume, HOMEBREW_TEMP, shims)"
t $S 'sudo mkdir -p /usr/local/Homebrew /usr/local/bin && sudo chown -R dev:staff /usr/local/Homebrew /usr/local/bin && git clone -q --depth 1 https://github.com/Homebrew/brew /usr/local/Homebrew && ln -sf ../Homebrew/bin/brew /usr/local/bin/brew && /usr/local/bin/brew --version | head -1'
echo "== brew bottle (ca-certificates: an all-platform bottle)"
t $S 'HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_ANALYTICS=1 /usr/local/bin/brew install ca-certificates 2>&1 | tail -2'
echo "== brew source build (hello)"
t $S 'HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_ANALYTICS=1 /usr/local/bin/brew install hello 2>&1 | tail -2; /usr/local/bin/hello'
echo "== round-1 broken tools"
$S 'uptime; top -l 1 -n 0 | head -2; echo abc | shasum -a 256; which readlink diff patch cmp install du gzip file | wc -l'
echo DONE
