# Releasing

The repository holds source, docs and tests only. Binaries and the package are release assets:
`dist/` is git-ignored and filled by `scripts/release.ps1`, then attached to a GitHub release.

```powershell
# from the dev workspace (same environment as HANDOFF/README build section)
cargo build --release --locked
powershell -File scripts\release.ps1 -Version 0.1.0              # build package into dist\, print the gh command
powershell -File scripts\release.ps1 -Version 0.1.0 -Publish     # also create a DRAFT release (needs gh, logged in)
```

Release assets: `wsm-setup.zip`, `SHA256SUMS`. Release notes: `docs/releases/v<version>.md`.

Never attach or commit: ISOs, `.dmg`, `.qcow2` bases or instances, `state.bin`, keys. `.gitignore` blocks them.
The release script refuses to run if `dist\` contains anything else.
