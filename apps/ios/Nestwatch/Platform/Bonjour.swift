// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// DNS-SD service type (protocol.md §3.4), as listed in `NSBonjourServices`.
let bonjourType = "_nestwatch._tcp."

/// Advertises the station through Bonjour (platforms.md §2.1, IOS-1) with the
/// TXT data of `StationSnapshot.mdnsTxt`. A TXT change (pairing opened or
/// closed) updates the record in place; a new name or port republishes.
/// Main thread only.
final class BonjourAdvertiser: NSObject, NetServiceDelegate {
    private var service: NetService?
    private var name = ""
    private var port: UInt16 = 0
    private var txt: [TxtEntry] = []

    func advertise(name: String, port: UInt16, txt: [TxtEntry]) {
        guard port != 0 else { return }
        let instance = String(name.prefix(63)).isEmpty ? "Nestwatch" : String(name.prefix(63))
        if let s = service, instance == self.name, port == self.port {
            if txt != self.txt {
                self.txt = txt
                s.setTXTRecord(Self.record(txt))
            }
            return
        }
        stop()
        self.name = instance
        self.port = port
        self.txt = txt
        let s = NetService(domain: "local.", type: bonjourType, name: instance, port: Int32(port))
        s.delegate = self
        s.setTXTRecord(Self.record(txt))
        s.publish()
        service = s
    }

    func stop() {
        service?.stop()
        service = nil
        name = ""
        port = 0
    }

    static func record(_ txt: [TxtEntry]) -> Data {
        NetService.data(fromTXTRecord: Dictionary(txt.map { ($0.key, Data($0.value.utf8)) }) { a, _ in a })
    }

    func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
        NSLog("Nestwatch: Bonjour publish failed: %@", errorDict.description)
    }
}

/// Browses for stations through Bonjour and reports resolved services to
/// the listener engine (`Listener.addDiscovered`), keeping TXT changes
/// current. Main thread only.
final class BonjourBrowser: NSObject, NetServiceBrowserDelegate, NetServiceDelegate {
    private let browser = NetServiceBrowser()
    private var services: [String: NetService] = [:]
    private let onFound: (DiscoveredService) -> Void
    private let onLost: (String) -> Void

    init(onFound: @escaping (DiscoveredService) -> Void, onLost: @escaping (String) -> Void) {
        self.onFound = onFound
        self.onLost = onLost
        super.init()
        browser.delegate = self
    }

    func start() { browser.searchForServices(ofType: bonjourType, inDomain: "local.") }

    func stop() {
        browser.stop()
        for s in services.values {
            s.stopMonitoring()
            s.stop()
        }
        services = [:]
    }

    static func fullname(_ s: NetService) -> String { "\(s.name).\(bonjourType)local." }

    func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
        let key = Self.fullname(service)
        services[key]?.stop()
        services[key] = service
        service.delegate = self
        service.resolve(withTimeout: 10)
        service.startMonitoring()
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didRemove service: NetService, moreComing: Bool) {
        let key = Self.fullname(service)
        services[key]?.stop()
        services[key] = nil
        onLost(key)
    }

    func netServiceDidResolveAddress(_ sender: NetService) { report(sender, txt: sender.txtRecordData()) }

    func netService(_ sender: NetService, didUpdateTXTRecord data: Data) { report(sender, txt: data) }

    private func report(_ s: NetService, txt: Data?) {
        let addrs = (s.addresses ?? []).compactMap(NetInfo.numericHost)
        guard !addrs.isEmpty, s.port > 0 else { return }
        let dict = txt.map(NetService.dictionary(fromTXTRecord:)) ?? [:]
        let entries = dict.compactMap { k, v in String(data: v, encoding: .utf8).map { TxtEntry(key: k, value: $0) } }
        onFound(DiscoveredService(
            fullname: Self.fullname(s),
            name: s.name,
            host: s.hostName ?? "",
            addrs: addrs,
            port: UInt16(s.port),
            txt: entries.sorted { $0.key < $1.key }
        ))
    }
}
