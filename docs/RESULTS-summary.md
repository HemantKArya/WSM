# WSM v0.1 results (round 4, one AMD 7840HS / RTX 4050 laptop, single runs)

| Step | Status | Numbers |
|---|---|---|
| S1 base build | PASS | sealed read-only base 17.93 GB, sha256 + manifest; stage 3 only (~10 min); autologin fix baked in |
| S2 shrink | SKIPPED | zstd recompression saves 0.5 % (base is already ~81 % compressed clusters) |
| S3 core smoke | PASS | first boot 37.5 s (incl. provisioning), cold boot to SSH ~24-27 s, suspend 1.7-2.7 s, resume to SSH 2.0-2.7 s x5, same session 5/5, warm `exec true` 57 ms (117 ms right after boot) |
| S3 idle suspend | PASS (5 min config) | suspended ~5 min after last access; the literal 10 min idle test was not run |
| S4 gpu smoke | PASS with caveat | same base, 4.96 GiB overlay; Metal clear+shader PASS, 200 submits 1.4 ms; cold boot to SSH 23.8 s; prewarm 15 s, `gui start` 1.8 s; animation 30-45 fps presented (not 60); `wsm stop` = shutdown 8.6 s |
| S5 cleanup | PASS | +66 GB free; steady state about 25 GB (base 17.9 + core 2.2 + gpu 5.1) |
| S6 package | PASS | wsm-setup.zip 49.8 MB; portable install, import, boot, suspend, resume, uninstall all pass |

Defects found and fixed this round: GUI password prompt on a fresh instance (loginwindow lastUserName), reims DLL d9024afc made every GPU submit ~200 ms (7bdb4ec0 used instead), packager pin and test-harness errors.
Not run: 1 h idle, 20-cycle A/B, 30 min GUI, Intel hosts, Finder drag, boot comparison of a compressed base.
