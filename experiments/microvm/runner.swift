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
    let rootDisk: String?
    let diskMode: String?
    let payload: String?
    let project: String?
    let snapshotMode: String?
    let snapshotPath: String?
    let restorePath: String?
}

func hostDiagnostics() -> [String: Any] {
    var probes: [String: Any] = [:]
    for name in ["kern.hv_support", "hw.optional.arm64"] {
        var value: Int32 = 0
        var length = MemoryLayout<Int32>.size
        let result = sysctlbyname(name, &value, &length, nil, 0)
        let savedErrno = errno
        if result == 0 {
            probes[name] = ["readable": true, "value": value]
        } else {
            probes[name] = ["readable": false, "errno": savedErrno,
                            "message": String(cString: strerror(savedErrno))]
        }
    }
    return ["framework_supported": VZVirtualMachine.isSupported, "sysctl": probes,
            "os": ProcessInfo.processInfo.operatingSystemVersionString]
}

final class Runner: NSObject, VZVirtualMachineDelegate {
    let queue = DispatchQueue(label: "cocoon.vm")
    let serial = Pipe()
    let origin = DispatchTime.now().uptimeNanoseconds
    var vm: VZVirtualMachine?
    var serialSource: DispatchSourceRead?
    var signalSources: [DispatchSourceSignal] = []
    var request: Request?
    var bytes = Data()
    var projectStarted = false
    var projectRequests: [[String: Any]] = []
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
        serialSource?.cancel()
        signalSources.forEach { $0.cancel() }
        report["status"] = status
        report["runner_elapsed_ms"] = elapsed()
        report["serial_base64"] = bytes.base64EncodedString()
        report["guest_serial"] = String(decoding: bytes, as: UTF8.self)
        var usage = rusage()
        if getrusage(RUSAGE_SELF, &usage) == 0 {
            report["runner_peak_rss_bytes"] = usage.ru_maxrss
        }
        let data = try! JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
        exit(code)
    }

    func installCancellation() {
        // Applies only to this runner process. All control stays on the VM queue.
        for number in [SIGINT, SIGTERM] {
            signal(number, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: number, queue: queue)
            source.setEventHandler { self.stop("signal") }
            signalSources.append(source)
            source.resume()
        }
    }

    func installSerialReader() -> Bool {
        let descriptor = serial.fileHandleForReading.fileDescriptor
        let flags = fcntl(descriptor, F_GETFL)
        guard flags >= 0, fcntl(descriptor, F_SETFL, flags | O_NONBLOCK) == 0 else {
            report["error"] = "cannot configure bounded serial reader"
            finish("setup_failed", 2)
            return false
        }
        // Read on the VM queue rather than enqueueing unbounded callback data.
        let source = DispatchSource.makeReadSource(fileDescriptor: descriptor, queue: queue)
        source.setEventHandler {
            guard !self.finished else { return }
            var chunk = [UInt8](repeating: 0, count: 4096)
            let count = chunk.withUnsafeMutableBytes {
                Darwin.read(descriptor, $0.baseAddress, $0.count)
            }
            if count > 0 {
                self.receive(Data(chunk.prefix(count)))
            } else if count == 0 {
                source.cancel()
            } else if errno != EAGAIN && errno != EINTR {
                self.report["serial_error"] = String(cString: strerror(errno))
                self.stop("serial_error")
            }
        }
        serialSource = source
        source.resume()
        return true
    }

    func begin(_ req: Request) {
        request = req
        report["boot_token"] = req.token
        report["memory_mib"] = req.memoryMiB
        report["vcpus"] = 1
        report["framework_supported"] = VZVirtualMachine.isSupported
        if req.snapshotMode != nil {
            report["snapshot_saved"] = false
        }
        let loader = VZLinuxBootLoader(kernelURL: URL(fileURLWithPath: req.kernel))
        loader.initialRamdiskURL = URL(fileURLWithPath: req.initrd)
        // Stay halted on panic; negative panic timeouts reboot immediately.
        loader.commandLine = "console=hvc0 rdinit=/init quiet panic=0"
        let config = VZVirtualMachineConfiguration()
        config.bootLoader = loader
        config.cpuCount = 1
        config.memorySize = req.memoryMiB * 1024 * 1024
        let port = VZVirtioConsoleDeviceSerialPortConfiguration()
        port.attachment = VZFileHandleSerialPortAttachment(
            fileHandleForReading: nil, fileHandleForWriting: serial.fileHandleForWriting)
        config.serialPorts = [port]
        if let disk = req.rootDisk, let mode = req.diskMode, let payload = req.payload {
            do {
                let attachment = try VZDiskImageStorageDeviceAttachment(url: URL(fileURLWithPath: disk), readOnly: false)
                config.storageDevices = [VZVirtioBlockDeviceConfiguration(attachment: attachment)]
                report["storage_devices"] = 1
                loader.commandLine += " cocoon.token=\(req.token) cocoon.mode=\(mode) cocoon.payload=\(payload)"
            } catch {
                report["error"] = error.localizedDescription
                finish("setup_failed", 2)
                return
            }
        }
        if let project = req.project {
            do {
                let input = try FileHandle(forReadingFrom: URL(fileURLWithPath: project))
                defer { try? input.close() }
                let data = try input.read(upToCount: 16385) ?? Data()
                guard data.count <= 16384, let requests = try JSONSerialization.jsonObject(with: data) as? [[String: Any]],
                      !requests.isEmpty, requests.count <= 16 else { throw NSError(domain: "project request limit", code: 1) }
                projectRequests = requests
                config.socketDevices = [VZVirtioSocketDeviceConfiguration()]
                report["vsock_devices"] = 1
            } catch {
                report["error"] = error.localizedDescription
                finish("setup_failed", 2)
                return
            }
        }
        // No network, graphics or shared directories. Optional project vsock only.
        do { try config.validate() } catch {
            let e = error as NSError
            report["error"] = ["domain": e.domain, "code": e.code, "message": e.localizedDescription]
            report["diagnostics"] = hostDiagnostics()
            finish("blocked", 2)
            return
        }
        guard installSerialReader() else { return }
        let machine = VZVirtualMachine(configuration: config, queue: queue)
        vm = machine
        machine.delegate = self
        installCancellation()
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
        // Drain without retaining further bytes once shutdown is underway.
        guard !stopping else { return }
        guard bytes.count + data.count <= 65536 else { stop("serial_limit"); return }
        bytes.append(data)
        guard let req = request else { return }
        let text = String(decoding: bytes, as: UTF8.self).replacingOccurrences(of: "\r\n", with: "\n")
        if text.contains("Kernel panic - not syncing:") {
            // Untrusted diagnostic hint; only the framework can confirm stopping.
            report["guest_panic_observed"] = true
            stop("guest_panic")
            return
        }
        if report["guest_ready"] as? Bool != true && text.contains("COCOON_READY:" + req.token + "\n") {
            report["guest_ready"] = true
            report["guest_ready_ms"] = elapsed()
        }
        if req.project != nil && !projectStarted && text.contains("COCOON_HELPER_READY\n") {
            projectStarted = true
            connectProject()
        }
        if report["guest_done"] as? Bool != true && text.contains("COCOON_DONE:" + req.token + "\n") {
            report["guest_done"] = true
            report["guest_done_ms"] = elapsed()
            stop("guest_done")
        }
    }

    func connectProject() {
        guard let socket = vm?.socketDevices.first as? VZVirtioSocketDevice else { stop("missing_vsock"); return }
        socket.connect(toPort: 1024) { result in
            switch result {
            case .failure(let error):
                self.report["transport_error"] = error.localizedDescription
                self.stop("transport_error")
            case .success(let connection):
                let requests = self.projectRequests
                DispatchQueue.global().async {
                    let result = Result { try exchangeProject(connection, requests: requests) }
                    self.queue.async {
                        guard !self.stopping else { return }
                        switch result {
                        case .success(let responses):
                            self.report["guest_transport"] = responses
                            self.stop("project_complete")
                        case .failure(let error):
                            self.report["transport_error"] = error.localizedDescription
                            self.stop("transport_error")
                        }
                    }
                }
            }
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
    if CommandLine.arguments == [CommandLine.arguments[0], "--doctor"] {
        runner.report["schema"] = "cocoon-vz-doctor-v1"
        runner.report["diagnostics"] = hostDiagnostics()
        runner.report["guest_boot_tested"] = false
        runner.finish("diagnostic_only", VZVirtualMachine.isSupported ? 0 : 2)
    }
    guard CommandLine.arguments.count == 2 else { throw NSError(domain: "usage: cocoon-vz REQUEST.json", code: 1) }
    let url = URL(fileURLWithPath: CommandLine.arguments[1])
    let handle = try FileHandle(forReadingFrom: url)
    let data = try handle.read(upToCount: 16385) ?? Data()
    try handle.close()
    guard data.count <= 16384 else { throw NSError(domain: "request too large", code: 1) }
    let fields = try JSONSerialization.jsonObject(with: data) as? [String: Any]
    let allowed = Set(["kernel", "initrd", "token", "memoryMiB", "seconds"])
    let optional = Set(["rootDisk", "diskMode", "payload", "project", "snapshotMode", "snapshotPath", "restorePath"])
    guard let fields = fields, Set(fields.keys).isSuperset(of: allowed),
          Set(fields.keys).isSubset(of: allowed.union(optional)) else {
        throw NSError(domain: "unknown or missing request fields", code: 1)
    }
    let req = try JSONDecoder().decode(Request.self, from: data)
    guard req.token.count == 32, req.token.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
          (128...4096).contains(req.memoryMiB), (1...60).contains(req.seconds) else {
        throw NSError(domain: "invalid experimental request", code: 1)
    }
    if let disk = req.rootDisk {
        guard disk.hasPrefix("/"), let mode = req.diskMode, ["write", "read", "project"].contains(mode),
              let payload = req.payload, payload.count == 32,
              payload.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
            throw NSError(domain: "invalid disk fixture request", code: 1)
        }
    } else if req.diskMode != nil || req.payload != nil || fields.keys.contains("rootDisk") {
        throw NSError(domain: "incomplete disk fixture request", code: 1)
    }
    guard (req.diskMode == "project") == (req.project != nil), req.project == nil || req.project!.hasPrefix("/") else {
        throw NSError(domain: "invalid project fixture request", code: 1)
    }
    if let mode = req.snapshotMode {
        let path = mode == "save" ? req.snapshotPath : req.restorePath
        guard ["save", "restore"].contains(mode), let path,
              path.hasPrefix("/"), path.count <= 4096,
              path != req.kernel, path != req.initrd, path != req.rootDisk else {
            throw NSError(domain: "invalid snapshot request", code: 1)
        }
        guard (mode == "save") == (req.snapshotPath != nil),
              (mode == "restore") == (req.restorePath != nil) else {
            throw NSError(domain: "ambiguous snapshot request", code: 1)
        }
    } else if req.snapshotPath != nil || req.restorePath != nil {
        throw NSError(domain: "snapshot path requires a mode", code: 1)
    }
    runner.queue.async { runner.begin(req) }
    dispatchMain()
} catch {
    runner.report["error"] = error.localizedDescription
    runner.finish("invalid_request", 2)
}
