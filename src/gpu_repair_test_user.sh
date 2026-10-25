#!/bin/sh
# Fix only the experiment's Recovery-created local dev account.
set -e
sudo dscl . -create /Users/dev AuthenticationAuthority ';ShadowHash;HASHLIST:<SALTED-SHA512-PBKDF2>'
sudo dscl . -passwd /Users/dev 'WsmGpu2026!'
dscl . -authonly dev 'WsmGpu2026!'
sudo dscl . -read /Users/dev AuthenticationAuthority
sync
