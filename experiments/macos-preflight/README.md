# macOS VM stage A experiment

English (authoritative) · [简体中文](README.zh-CN.md)

This experiment directory contains a real Apple framework preflight. It does
not modify product code, start existing Lima/Docker machines, install a service,
or grant a spool execution capability. The supplied kernel is used only to
validate a VM configuration; no guest boots and no guest disk is opened.

Run on an Apple Silicon Mac with Xcode tools:

```sh
python3 run_preflight.py --kernel /absolute/path/to/linux-arm64-Image
```

The script compiles Swift, ad-hoc signs the temporary executable with Apple's
hypervisor and virtualization entitlements, validates a diskless/networkless
Linux VM configuration, and creates/destroys an empty Hypervisor context.
It writes `preflight-result.json`. Exit 2 means a prerequisite failed, not a
successful VM test. Even exit 0 qualifies only preflight, not guest execution.

## Observed boundary

The current managed execution context compiled and signed the probe. The empty
Hypervisor context create/destroy calls succeeded. Virtualization.framework
reported `isSupported == false` and rejected the configuration with
`VZErrorDomain` code 2. This does not establish that the physical Mac lacks
virtualization support; the discrepancy requires checking from an ordinary
Terminal session. No environment restriction was disabled.

Apple `container` is absent from PATH. A bounded HTTPS request to GitHub failed
DNS resolution, preventing acquisition of Containerization dependencies.
The existing Docker ARM64 Linux kernel was used read-only for configuration
validation. Docker and Lima were not started or changed.

## Remaining stage A acceptance

1. Resolve framework availability in the actual execution context.
2. Select and pin a Linux ARM64 kernel and guest image. The newer
   [minimal microVM experiment](../microvm/README.md) uses Virtualization.framework
   directly; Apple Containerization is not a required dependency.
3. Start an independent VM without guest networking or broad host mounts.
4. Transfer a fixed input, execute a bounded transformation, and compare output.
5. Run a task past its deadline; stop the owned VM and confirm termination.
6. Record versions, identities, timings and exact failures before selecting the
   production engine. A guest exit or CLI exit alone is insufficient evidence.

This experiment has not passed those guest acceptance checks and is not ready
to promote into the execution backend.
