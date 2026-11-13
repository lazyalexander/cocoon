# Cocoon benchmark protocol

English (authoritative) · [简体中文](README.zh-CN.md)

Measure release builds on a named Apple Silicon host with pinned macOS, kernel,
root image, runner, Rust toolchain and power mode. Record their hashes/versions in
a private run manifest. Use the actual Rust host service and framework runner;
contract tests, preflight, mock responses and native fallback never count.

| Scenario | Measurement | Required observation |
| --- | --- | --- |
| Cold create/start/stop | host monotonic wall time and p95 | framework start and stop evidence; no unknown VM |
| Warm guest file/command | request to response latency, bytes/s, exit status | correct bytes and attempt/generation correlation |
| Import/export | 4 KiB, 1 MiB, 16 MiB; latency and throughput | hashes, bounded host destination, baseline unchanged |
| Memory/disk | idle and active host RSS, disk allocated/logical | 1/4/8 resident and 1/4 active, no swap or OOM |
| Saturation | accepted work/s and p95 at 1/2/4 callers | one primary agent per VM; guest subagents share its limits |
| Recovery | crash/timeout/kill at every durable intent boundary | no duplicate command; unknown outcome remains quarantined |
| Security boundary | expired/revoked/stale/wrong-VM grants | zero unauthorized host effects; no private path/token output |
| Snapshot | suspend/resume/cold recovery time and memory | RAM nonce and disk hash; confirmed original runner stop |

Run a correctness warmup first. For timing distributions collect at least 100
successful independent samples per scenario; collect 300 for p99. Any failure or
unknown sample makes that scenario incomplete, and retries get new IDs. Report
median, nearest-rank p95/p99, sample count, failures and unknowns. Separate image
construction/download from VM lifecycle timing. Do not pool unlike host/image
configurations or infer an M5/16 GB result from another Mac.

Use the latency and capacity gates in [SPECIFICATION](../SPECIFICATION.md); do not
invent a passing threshold where none is specified. The JSONL row shape is
`{"run_id":"UUID","host":"M5-16GB-config-hash","scenario":"cold_boot","metric":"ready_ms","unit":"ms","value":123.4,"result":"passed"}`.
`python3 benchmarks/analyze.py private-samples.jsonl` checks identities and
computes summary statistics. It does not collect or synthesize measurements.
Keep raw samples, manifests, guest output and private paths out of Git. Release
qualification requires real macOS VM evidence and explicit human review; CI
runs deterministic checks only, not noisy timing gates.
