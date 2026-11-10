import Foundation
import Virtualization
import Darwin

// One process owns one VM. This is an experiment, not an admission endpoint.
struct Request: Decodable {
    let kernel: String
    let initrd: String
    let token: String
    let memoryMiB: UInt64
    let seconds: Double
}

final class Runner: NSObject, VZVirtualMachineDelegate {
    let queue = DispatchQueue(label: "cocoon.vm")
    let serial = Pipe()
    let origin = DispatchTime.now().uptimeNanoseconds
    var vm: VZVirtualMachine?
    var request: Request?
    var bytes = Data()
    var stopping = false
    var stopSent = false
    var finished = false
    var report: [String: Any] = ["schema": "cocoon-vz-experiment-v1",
        "guest_ready": false, "guest_done": false, "termination_confirmed": false,
        "network_devices": 0, "storage_devices": 0, "shared_directories": 0,
        "native_fallback": false]

    func elapsed() -> Double {
        Double(DispatchTime.now().uptimeNanoseconds - origin) / 1_000_000
    }

    func finish(_ status: String, _ code: Int32) {
        guard !finished else { return }
        finished = true
        serial.fileHandleForReading.readabilityHandler = nil
        report["status"] = status
        report["runner_elapsed_ms"] = elapsed()
        report["serial_base64"] = bytes.base64EncodedString()
        var usage = rusage()
        if getrusage(RUSAGE_SELF, &usage) == 0 {
            report["runner_peak_rss_bytes"] = usage.ru_maxrss
        }
        let data = try! JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
        exit(code)
    }

    func begin(_ req: Request) {
        request = req
        report["memory_mib"] = req.memoryMiB
        report["vcpus"] = 1
        report["framework_supported"] = VZVirtualMachine.isSupported
        let loader = VZLinuxBootLoader(kernelURL: URL(fileURLWithPath: req.kernel))
        loader.initialRamdiskURL = URL(fileURLWithPath: req.initrd)
        loader.commandLine = "console=hvc0 rdinit=/init quiet panic=-1"
        let config = VZVirtualMachineConfiguration()
        config.bootLoader = loader
        config.cpuCount = 1
        config.memorySize = req.memoryMiB * 1024 * 1024
        let port = VZVirtioConsoleDeviceSerialPortConfiguration()
        port.attachment = VZFileHandleSerialPortAttachment(
            fileHandleForReading: nil, fileHandleForWriting: serial.fileHandleForWriting)
        config.serialPorts = [port]
        // No disk, network, graphics, shared directory, vsock or balloon device.
        do { try config.validate() } catch {
            let e = error as NSError
            report["error"] = ["domain": e.domain, "code": e.code, "message": e.localizedDescription]
            finish("blocked", 2)
            return
        }
        let machine = VZVirtualMachine(configuration: config, queue: queue)
        vm = machine
        machine.delegate = self
        serial.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty else { return }
            self?.queue.async { self?.receive(data) }
        }
        report["start_requested_ms"] = elapsed()
        queue.asyncAfter(deadline: .now() + req.seconds) { self.stop("deadline") }
        machine.start { result in
            switch result {
            case .success:
                self.report["vm_running_ms"] = self.elapsed()
                if self.stopping { self.performStop() }
            case .failure(let error):
                self.report["error"] = error.localizedDescription
                // A failed start is not asserted to be confirmed termination.
                self.finish("start_failed", 2)
            }
        }
    }

    func receive(_ data: Data) {
        guard !finished else { return }
        guard bytes.count + data.count <= 65536 else { stop("serial_limit"); return }
        bytes.append(data)
        guard let req = request else { return }
        let text = String(decoding: bytes, as: UTF8.self).replacingOccurrences(of: "\r\n", with: "\n")
        if report["guest_ready"] as? Bool != true && text.contains("COCOON_READY:" + req.token + "\n") {
            report["guest_ready"] = true
            report["guest_ready_ms"] = elapsed()
        }
        if text.contains("COCOON_DONE:" + req.token + "\n") {
            report["guest_done"] = true
            report["guest_done_ms"] = elapsed()
            stop("guest_done")
        }
    }

    func stop(_ reason: String) {
        guard !stopping && !finished else { return }
        stopping = true
        report["stop_reason"] = reason
        report["stop_requested_ms"] = elapsed()
        queue.asyncAfter(deadline: .now() + 5) { self.finish("termination_unconfirmed", 3) }
        performStop()
    }

    func performStop() {
        guard !stopSent, let machine = vm, machine.canStop else { return }
        stopSent = true
        machine.stop { error in
            guard error == nil && machine.state == .stopped else {
                self.report["stop_error"] = error?.localizedDescription ?? "VM not stopped"
                self.finish("termination_unconfirmed", 3)
                return
            }
            self.report["termination_confirmed"] = true
            self.report["stopped_ms"] = self.elapsed()
            self.finish("stopped", 0)
        }
    }

    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        report["termination_confirmed"] = virtualMachine.state == .stopped
        finish("guest_stopped", virtualMachine.state == .stopped ? 0 : 3)
    }

    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        report["error"] = error.localizedDescription
        finish("vm_error", 3)
    }
}

let runner = Runner()
do {
    guard CommandLine.arguments.count == 2 else { throw NSError(domain: "usage: cocoon-vz REQUEST.json", code: 1) }
    let url = URL(fileURLWithPath: CommandLine.arguments[1])
    let handle = try FileHandle(forReadingFrom: url)
    let data = try handle.read(upToCount: 16385) ?? Data()
    try handle.close()
    guard data.count <= 16384 else { throw NSError(domain: "request too large", code: 1) }
    let req = try JSONDecoder().decode(Request.self, from: data)
    guard req.token.count == 32, req.token.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
          (128...4096).contains(req.memoryMiB), (1...60).contains(req.seconds) else {
        throw NSError(domain: "invalid experimental request", code: 1)
    }
    runner.queue.async { runner.begin(req) }
    dispatchMain()
} catch {
    runner.report["error"] = error.localizedDescription
    runner.finish("invalid_request", 2)
}
