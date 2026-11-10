import Foundation
import Virtualization
import Darwin

// Only a byte transport. These are guest assertions, never lifecycle evidence.
func exchangeProject(_ connection: VZVirtioSocketConnection, requests: [[String: Any]]) throws -> [[String: Any]] {
    let fd = connection.fileDescriptor
    var timeout = timeval(tv_sec: 3, tv_usec: 0)
    for option in [SO_RCVTIMEO, SO_SNDTIMEO] {
        guard setsockopt(fd, SOL_SOCKET, option, &timeout, socklen_t(MemoryLayout<timeval>.size)) == 0 else {
            throw NSError(domain: "vsock timeout setup", code: 1)
        }
    }
    var noSignal: Int32 = 1
    guard setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSignal, socklen_t(MemoryLayout<Int32>.size)) == 0 else {
        throw NSError(domain: "vsock signal setup", code: 1)
    }
    func readExact(_ size: Int) throws -> Data {
        var data = Data(count: size)
        var offset = 0
        while offset < size {
            let count = data.withUnsafeMutableBytes { ptr in
                Darwin.read(fd, ptr.baseAddress!.advanced(by: offset), size - offset)
            }
            if count < 0 && errno == EINTR { continue }
            guard count > 0 else { throw NSError(domain: "vsock incomplete response; no replay", code: 1) }
            offset += count
        }
        return data
    }
    var responses: [[String: Any]] = []
    var total = 0
    for request in requests {
        let body = try JSONSerialization.data(withJSONObject: request, options: [.sortedKeys])
        guard body.count <= 32768 else { throw NSError(domain: "vsock request limit", code: 1) }
        var size = UInt32(body.count).bigEndian
        var frame = withUnsafeBytes(of: &size) { Data($0) }
        frame.append(body)
        var offset = 0
        while offset < frame.count {
            let count = frame.withUnsafeBytes { ptr in Darwin.write(fd, ptr.baseAddress!.advanced(by: offset), frame.count - offset) }
            if count < 0 && errno == EINTR { continue }
            guard count > 0 else { throw NSError(domain: "vsock write failed; outcome unknown", code: 1) }
            offset += count
        }
        let header = try readExact(4)
        let length = header.reduce(0) { ($0 << 8) | Int($1) }
        guard length > 0 && length <= 32768 && total + length <= 131072 else { throw NSError(domain: "vsock response limit", code: 1) }
        total += length
        let responseData = try readExact(length)
        guard let response = try JSONSerialization.jsonObject(with: responseData) as? [String: Any],
              let identity = response["identity"] as? NSDictionary,
              let expected = request["identity"] as? NSDictionary, identity == expected else {
            throw NSError(domain: "vsock correlation mismatch", code: 1)
        }
        responses.append(response)
        guard let result = response["result"] as? [String: Any], result["Ok"] != nil else { break }
    }
    return responses
}
