// SPDX-License-Identifier: AGPL-3.0-or-later
// Free functions of the generated bindings whose names the app's own types
// shadow (the bindings compile into this module).

/// `startStation` of nestwatch-ffi.
func ffiStartStation(options: StationOptions, storage: Storage, observer: StationObserver) async throws -> Station {
    try await startStation(options: options, storage: storage, observer: observer)
}
