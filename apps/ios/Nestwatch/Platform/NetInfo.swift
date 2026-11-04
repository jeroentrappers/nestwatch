// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation
import Network

/// The device's interface addresses for the station's invitation endpoints
/// (protocol.md §3.5); the core orders and filters them (VPN interfaces
/// such as `utun3` are recognised by name and address).
enum NetInfo {
    static func interfaces() -> [InterfaceAddress] {
        var out: [InterfaceAddress] = []
        var head: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&head) == 0, let first = head else { return out }
        defer { freeifaddrs(head) }
        for p in sequence(first: first, next: { $0.pointee.ifa_next }) {
            let ifa = p.pointee
            guard let addr = ifa.ifa_addr, (ifa.ifa_flags & UInt32(IFF_UP)) != 0,
                  (ifa.ifa_flags & UInt32(IFF_LOOPBACK)) == 0 else { continue }
            let family = Int32(addr.pointee.sa_family)
            guard family == AF_INET || family == AF_INET6 else { continue }
            guard let ip = numericHost(addr) else { continue }
            out.append(InterfaceAddress(name: String(cString: ifa.ifa_name), ip: ip))
        }
        return out
    }

    /// The numeric host of a socket address, without a zone suffix.
    static func numericHost(_ addr: UnsafePointer<sockaddr>) -> String? {
        var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
        let len = socklen_t(addr.pointee.sa_family == AF_INET6 ? MemoryLayout<sockaddr_in6>.size : MemoryLayout<sockaddr_in>.size)
        guard getnameinfo(addr, len, &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 else { return nil }
        let s = String(cString: host)
        return s.split(separator: "%", maxSplits: 1).first.map(String.init)
    }

    /// The numeric host inside a `sockaddr` blob (Bonjour results).
    static func numericHost(_ data: Data) -> String? {
        data.withUnsafeBytes { raw -> String? in
            guard let base = raw.baseAddress, raw.count >= MemoryLayout<sockaddr>.size else { return nil }
            return numericHost(base.assumingMemoryBound(to: sockaddr.self))
        }
    }
}

/// Tells the listener about network changes (rendezvous.md §5.2): reconnects
/// retry at once and relayed sessions try a direct path again.
final class NetworkWatch {
    private let monitor = NWPathMonitor()
    private var last: NWPath?

    init(onChange: @escaping () -> Void) {
        monitor.pathUpdateHandler = { [weak self] path in
            guard let self else { return }
            defer { self.last = path }
            // The first report is the current state, not a change.
            guard let last = self.last else { return }
            if last.status != path.status || last.availableInterfaces.map(\.name) != path.availableInterfaces.map(\.name) {
                onChange()
            }
        }
    }

    func start() { monitor.start(queue: DispatchQueue(label: "nestwatch.network")) }
    func stop() { monitor.cancel() }
}
