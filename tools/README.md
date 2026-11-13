# Build and validation entry points

English (authoritative) · [简体中文](README.zh-CN.md)

`tools/build check` checks the pinned Rust workspace quickly. `tools/build release` builds release binaries; `tools/build guest` cross-builds Linux ARM64
fixtures with an explicit target/linker; `tools/build runner` compiles and signs
the minimal macOS adapter under ignored `target/`. The latter requires macOS.
`tools/check` runs documentation links, Rust formatting/tests/Clippy and Python
harness tests. Keep production VM benchmarks outside CI: they require a real,
named Mac and private inputs. CI runs Linux source checks and macOS runner
protocol checks independently, cancels superseded runs, and never deploys.
Release or CD requires a separate explicit publication decision.
