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
    let eventLog: String?
    let controlSocket: String?
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
    var controlSource: DispatchSourceRead?
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

    func emitEvent(_ name: String) {
        guard let path = request?.eventLog else { return }
        let url = URL(fileURLWithPath: path)
        do {
            let event: [String: Any] = ["event": name, "token": request?.token ?? "",
                                        "termination_confirmed": report["termination_confirmed"] as? Bool ?? false]
            let data = try JSONSerialization.data(withJSONObject: event, options: [.sortedKeys])
            if FileManager.default.fileExists(atPath: path) {
                let output = try FileHandle(forWritingTo: url)
                try output.seekToEnd()
                try output.write(contentsOf: data)
                try output.write(contentsOf: Data("\n".utf8))
                try output.synchronize()
                try output.close()
            } else {
                try (data + Data("\n".utf8)).write(to: url, options: [.withoutOverwriting])
            }
        } catch {
            report["event_error"] = error.localizedDescription
        }
    }

    func finish(_ status: String, _ code: Int32) {
        guard !finished else { return }
        finished = true
        serialSource?.cancel()
        controlSource?.cancel()
        signalSources.forEach { $0.cancel() }
        report["status"] = status
        report["runner_elapsed_ms"] = elapsed()
        report["serial_base64"] = bytes.base64EncodedString()
        report["guest_serial"] = String(decoding: bytes, as: UTF8.self)
        var usage = rusage()
        if getrusage(RUSAGE_SELF, &usage) == 0 {
            report["runner_peak_rss_bytes"] = usage.ru_maxrss
        }
        if report["termination_confirmed"] as? Bool == true {
            emitEvent("stopped")
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
        signal(SIGUSR1, SIG_IGN)
        let snapshot = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: queue)
        snapshot.setEventHandler { self.requestSnapshot() }
        signalSources.append(snapshot)
        snapshot.resume()
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
            report["snapshot_restored"] = false
        }
        let loader = VZLinuxBootLoader(kernelURL: URL(fileURLWithPath: req.kernel))
        loader.initialRamdiskURL = URL(fileURLWithPath: req.initrd)
        // Stay halted on panic; negative panic timeouts reboot immediately.
        loader.commandLine = "console=hvc0 rdinit=/init quiet panic=0"
        let config = VZVirtualMachineConfiguration()
        let platform = VZGenericPlatformConfiguration()
        do {
            if req.snapshotMode == "save", let path = req.snapshotPath {
                guard !FileManager.default.fileExists(atPath: path) else {
                    throw NSError(domain: "snapshot destination already exists", code: 1)
                }
                let identifier = VZGenericMachineIdentifier()
                try identifier.dataRepresentation.write(to: URL(fileURLWithPath: path + ".machine-id"),
                                                        options: [.withoutOverwriting])
                platform.machineIdentifier = identifier
            } else if req.snapshotMode == "restore", let path = req.restorePath {
                let data = try Data(contentsOf: URL(fileURLWithPath: path + ".machine-id"))
                guard let identifier = VZGenericMachineIdentifier(dataRepresentation: data) else {
                    throw NSError(domain: "invalid snapshot machine identifier", code: 1)
                }
                platform.machineIdentifier = identifier
            }
        } catch {
            report["snapshot_error"] = error.localizedDescription
            finish("snapshot_setup_failed", 2)
            return
        }
        config.platform = platform
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
        if req.diskMode == "agent" {
            config.socketDevices = [VZVirtioSocketDeviceConfiguration()]
            report["vsock_devices"] = 1
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
        if let control = req.controlSocket {
            do {
                controlSource = try installControlChannel(path: control, queue: queue) { [weak self] in self?.vm }
            } catch {
                report["error"] = error.localizedDescription
                finish("setup_failed", 2)
                return
            }
        }
        machine.delegate = self
        installCancellation()
        report["start_requested_ms"] = elapsed()
        queue.asyncAfter(deadline: .now() + req.seconds) { self.stop("deadline") }
        if req.snapshotMode == "restore", let path = req.restorePath {
            let claim = URL(fileURLWithPath: path + ".restore-claimed")
            do {
                try Data().write(to: claim, options: [.withoutOverwriting])
            } catch {
                report["snapshot_error"] = "snapshot restore was already claimed or cannot be claimed"
                finish("snapshot_restore_claimed", 2)
                return
            }
            report["snapshot_restore_claimed"] = true
            machine.restoreMachineStateFrom(url: URL(fileURLWithPath: path)) { error in
                guard self.continueSnapshotCallback() else { return }
                guard error == nil else {
                    self.report["snapshot_error"] = error!.localizedDescription
                    self.finish("snapshot_restore_failed", 2)
                    return
                }
                self.report["snapshot_restored"] = true
                self.report["snapshot_restore_ms"] = self.elapsed()
                machine.resume { result in
                    guard self.continueSnapshotCallback() else { return }
                    switch result {
                    case .success:
                        self.report["vm_running_ms"] = self.elapsed()
                        self.emitEvent("vm_resumed")
                        if self.stopping { self.performStop() }
                    case .failure(let error):
                        self.report["snapshot_error"] = error.localizedDescription
                        self.finish("snapshot_resume_failed", 2)
                    }
                }
            }
        } else {
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
        if req.snapshotMode == "save" && text.contains("COCOON_SNAPSHOT_READY:" + req.token + ":") {
            requestSnapshot()
        }
        if (req.project != nil || req.diskMode == "agent") && !projectStarted && text.contains("COCOON_HELPER_READY\n") {
            projectStarted = true
            if req.diskMode == "agent" {
                emitEvent("guest_ready")
            } else {
                connectProject()
            }
        }
        if report["guest_done"] as? Bool != true && text.contains("COCOON_DONE:" + req.token + "\n") {
            report["guest_done"] = true
            report["guest_done_ms"] = elapsed()
            stop("guest_done")
        }
    }

    // All callbacks run on the VM queue. Cancellation owns the next transition;
    // a completed framework operation cannot authorize another save or resume.
    func continueSnapshotCallback() -> Bool {
        guard !finished else { return false }
        if stopping {
            performStop()
            return false
        }
        return true
    }

    func requestSnapshot() {
        guard !stopping && !finished && report["snapshot_save_requested"] as? Bool != true else { return }
        report["snapshot_save_requested"] = true
        saveSnapshot()
    }

    func saveSnapshot() {
        guard let req = request, let path = req.snapshotPath, let machine = vm, machine.canPause else {
            report["snapshot_error"] = "VM cannot be paused"
            stop("snapshot_save_failed")
            return
        }
        let url = URL(fileURLWithPath: path)
        guard !FileManager.default.fileExists(atPath: url.path) else {
            report["snapshot_error"] = "snapshot destination already exists"
            stop("snapshot_save_failed")
            return
        }
        machine.pause { result in
            guard self.continueSnapshotCallback() else { return }
            switch result {
            case .failure(let error):
                self.report["snapshot_error"] = error.localizedDescription
                self.stop("snapshot_save_failed")
            case .success:
                machine.saveMachineStateTo(url: url) { error in
                    guard self.continueSnapshotCallback() else { return }
                    guard error == nil else {
                        self.report["snapshot_error"] = error!.localizedDescription
                        self.stop("snapshot_save_failed")
                        return
                    }
                    do {
                        if let restore = req.restorePath {
                            let identity = try Data(contentsOf: URL(fileURLWithPath: restore + ".machine-id"))
                            try identity.write(to: URL(fileURLWithPath: path + ".machine-id"), options: [.withoutOverwriting])
                        }
                        let output = try FileHandle(forWritingTo: url)
                        try output.synchronize()
                        try output.close()
                        self.report["snapshot_saved"] = true
                        self.report["snapshot_save_ms"] = self.elapsed()
                        self.emitEvent("snapshot_saved")
                        self.stop("snapshot_saved")
                    } catch {
                        self.report["snapshot_error"] = error.localizedDescription
                        self.stop("snapshot_save_failed")
                    }
                }
            }
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
    let optional = Set(["rootDisk", "diskMode", "payload", "project", "snapshotMode", "snapshotPath", "restorePath", "eventLog", "controlSocket"])
    guard let fields = fields, Set(fields.keys).isSuperset(of: allowed),
          Set(fields.keys).isSubset(of: allowed.union(optional)) else {
        throw NSError(domain: "unknown or missing request fields", code: 1)
    }
    let req = try JSONDecoder().decode(Request.self, from: data)
    guard req.token.count == 32, req.token.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
          (128...4096).contains(req.memoryMiB),
          (1...60).contains(req.seconds) || req.diskMode == "agent" && (1...3600).contains(req.seconds) else {
        throw NSError(domain: "invalid experimental request", code: 1)
    }
    if let disk = req.rootDisk {
        guard disk.hasPrefix("/"), let mode = req.diskMode, ["write", "read", "project", "snapshot", "agent"].contains(mode),
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
    if let events = req.eventLog {
        guard events.hasPrefix("/"), events.count <= 4096,
              events != req.kernel, events != req.initrd, events != req.rootDisk,
              events != req.snapshotPath, events != req.restorePath else {
            throw NSError(domain: "invalid event log", code: 1)
        }
    }
    if let control = req.controlSocket {
        guard req.diskMode == "agent", control.hasPrefix("/"), control.count <= 100,
              control != req.kernel, control != req.initrd, control != req.rootDisk,
              control != req.eventLog, control != req.snapshotPath, control != req.restorePath else {
            throw NSError(domain: "invalid control socket", code: 1)
        }
    }
    if let mode = req.snapshotMode {
        let path = mode == "save" ? req.snapshotPath : req.restorePath
        guard ["save", "restore"].contains(mode), let path,
              path.hasPrefix("/"), path.count <= 4096,
              path != req.kernel, path != req.initrd, path != req.rootDisk else {
            throw NSError(domain: "invalid snapshot request", code: 1)
        }
        guard (mode == "restore") == (req.restorePath != nil) else {
            throw NSError(domain: "ambiguous snapshot request", code: 1)
        }
        if mode == "save" && req.snapshotPath == nil || mode == "restore" && req.restorePath == nil {
            throw NSError(domain: "missing snapshot path", code: 1)
        }
        if let next = req.snapshotPath {
            guard next.hasPrefix("/"), next.count <= 4096, req.restorePath == nil || next != path,
                  next != req.kernel, next != req.initrd, next != req.rootDisk else {
                throw NSError(domain: "invalid next snapshot path", code: 1)
            }
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
