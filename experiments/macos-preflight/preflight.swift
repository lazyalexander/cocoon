import Foundation
import Hypervisor
import Virtualization

// This creates an empty hypervisor context, never boots an OS or opens a disk.
var report: [String: Any] = [
    "schema": "cocoon-macos-vm-preflight-v1",
    "os": ProcessInfo.processInfo.operatingSystemVersionString,
    "virtualization_supported": VZVirtualMachine.isSupported,
    "guest_boot_tested": false,
    "offline_execution_tested": false,
    "file_transfer_tested": false,
    "timeout_stop_tested": false,
]
if CommandLine.arguments.count == 2 {
    let config = VZVirtualMachineConfiguration()
    config.cpuCount = 2
    config.memorySize = 1024 * 1024 * 1024
    config.bootLoader = VZLinuxBootLoader(kernelURL: URL(fileURLWithPath: CommandLine.arguments[1]))
    config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]
    config.networkDevices = []
    config.storageDevices = []
    do {
        try config.validate()
        report["configuration_validated"] = true
    } catch {
        let failure = error as NSError
        report["configuration_validated"] = false
        report["configuration_error"] = ["domain": failure.domain, "code": failure.code,
                                          "description": failure.localizedDescription]
    }
}
#if arch(arm64)
report["arch"] = "arm64"
let created = hv_vm_create(nil)
report["hv_vm_create"] = String(format: "0x%08x", UInt32(bitPattern: created))
if created == HV_SUCCESS {
    let destroyed = hv_vm_destroy()
    report["hv_vm_destroy"] = String(format: "0x%08x", UInt32(bitPattern: destroyed))
    report["empty_context_cycle_passed"] = destroyed == HV_SUCCESS
} else {
    report["empty_context_cycle_passed"] = false
}
#else
report["arch"] = "unsupported"
report["empty_context_cycle_passed"] = false
#endif
let bytes = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
FileHandle.standardOutput.write(bytes)
FileHandle.standardOutput.write(Data("\n".utf8))
let passed = report["empty_context_cycle_passed"] as? Bool == true
    && report["virtualization_supported"] as? Bool == true
    && report["configuration_validated"] as? Bool == true
exit(passed ? 0 : 2)
