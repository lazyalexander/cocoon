import Foundation
import Virtualization
import Darwin

func writeControl(_ fd: Int32, _ data: Data) throws {
    var offset = 0
    while offset < data.count {
        let count = data.withUnsafeBytes { ptr in
            Darwin.write(fd, ptr.baseAddress!.advanced(by: offset), data.count - offset)
        }
        if count < 0 && errno == EINTR { continue }
        guard count > 0 else { throw NSError(domain: "control write failed", code: 1) }
        offset += count
    }
}

func readControl(_ fd: Int32, _ size: Int) throws -> Data {
    var data = Data(count: size)
    var offset = 0
    while offset < size {
        let count = data.withUnsafeMutableBytes { ptr in
            Darwin.read(fd, ptr.baseAddress!.advanced(by: offset), size - offset)
        }
        if count < 0 && errno == EINTR { continue }
        guard count > 0 else { throw NSError(domain: "control frame incomplete", code: 1) }
        offset += count
    }
    return data
}

func controlLog(_ message: String) {
    let data = Data(("control: " + message + "\n").prefix(4096).utf8)
    FileHandle.standardError.write(data)
}

func installControlChannel(path: String, queue: DispatchQueue,
                           machine: @escaping () -> VZVirtualMachine?) throws -> DispatchSourceRead {
    guard path.utf8.count < 100 else { throw NSError(domain: "control path too long", code: 1) }
    let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { throw NSError(domain: "control socket", code: 1) }
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    let pathBytes = Array(path.utf8) + [0]
    withUnsafeMutableBytes(of: &address) { raw in
        let offset = MemoryLayout<sa_family_t>.size + MemoryLayout<UInt8>.size
        raw.baseAddress!.advanced(by: offset).copyMemory(from: pathBytes, byteCount: pathBytes.count)
    }
    address.sun_len = UInt8(MemoryLayout<sa_family_t>.size + MemoryLayout<UInt8>.size + pathBytes.count)
    let length = socklen_t(MemoryLayout<sa_family_t>.size + MemoryLayout<UInt8>.size + pathBytes.count)
    let bound = withUnsafePointer(to: &address) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(fd, $0, length) }
    }
    guard bound == 0, Darwin.listen(fd, 16) == 0 else {
        let error = errno
        Darwin.close(fd)
        throw NSError(domain: "control bind/listen", code: Int(error))
    }
    guard Darwin.chmod(path, 0o600) == 0 else {
        let error = errno
        Darwin.close(fd)
        Darwin.unlink(path)
        throw NSError(domain: "control permissions", code: Int(error))
    }
    let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
    source.setEventHandler {
        controlLog("accept event")
        let connection = Darwin.accept(fd, nil, nil)
        guard connection >= 0 else { return }
        controlLog("accepted")
        DispatchQueue.global().async {
            defer { Darwin.close(connection) }
            do {
                let header = try readControl(connection, 4)
                let length = header.reduce(0) { ($0 << 8) | Int($1) }
                guard length > 0 && length <= 32768 else { throw NSError(domain: "control frame limit", code: 1) }
                let body = try readControl(connection, length)
                controlLog("request read")
                guard let request = try JSONSerialization.jsonObject(with: body) as? [String: Any],
                      let socket = machine()?.socketDevices.first as? VZVirtioSocketDevice else {
                    throw NSError(domain: "invalid control request", code: 1)
                }
                controlLog("connecting")
                let semaphore = DispatchSemaphore(value: 0)
                var connected: VZVirtioSocketConnection?
                var connectError: Error?
                queue.async {
                    socket.connect(toPort: 1024) { result in
                        switch result {
                        case .success(let channel): connected = channel
                        case .failure(let error): connectError = error
                        }
                        semaphore.signal()
                    }
                }
                guard semaphore.wait(timeout: .now() + 3) == .success else {
                    throw NSError(domain: "vsock connect timeout", code: 1)
                }
                guard let channel = connected else {
                    throw connectError ?? NSError(domain: "vsock connect failed", code: 1)
                }
                controlLog("connected")
                defer { channel.close() }
                let responses = try exchangeProject(channel, requests: [request])
                controlLog("exchanged")
                guard let response = responses.first else { throw NSError(domain: "empty vsock response", code: 1) }
                let data = try JSONSerialization.data(withJSONObject: response)
                var size = UInt32(data.count).bigEndian
                var frame = withUnsafeBytes(of: &size) { Data($0) }
                frame.append(data)
                try writeControl(connection, frame)
                controlLog("responded")
            } catch {
                controlLog(error.localizedDescription)
            }
        }
    }
    source.setCancelHandler {
        Darwin.close(fd)
        Darwin.unlink(path)
    }
    source.resume()
    return source
}
