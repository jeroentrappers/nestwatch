// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Tile path label key (FR-LS-3, rendezvous.md FR-RV-4).
func pathLabel(_ path: NetworkPath) -> String {
    switch path {
    case .lan: return tr("path_lan")
    case .vpn: return tr("path_vpn")
    case .direct: return tr("path_direct")
    case .relayed: return tr("path_relayed")
    }
}

/// How a relay status is shown: the pill's word and its colour role.
enum RelayLook: Equatable {
    case off, connecting, online, error
}

/// Maps the station's relay registration to its pill (rendezvous.md FR-RV-1).
func relayLook(_ status: RelayStatus) -> RelayLook {
    switch status {
    case .off: return .off
    case .connecting: return .connecting
    case .online: return .online
    case .error: return .error
    }
}

func relayLabel(_ look: RelayLook) -> String {
    switch look {
    case .off: return tr("relay_status_off")
    case .connecting: return tr("relay_status_connecting")
    case .online: return tr("relay_status_online")
    case .error: return tr("relay_status_error")
    }
}

/// Status pill flavours (`.pill` + `.calm` `.sound` `.warn` `.voice` `.cam`).
enum PillKind: Equatable {
    case calm, sound, warn, voice, cam, neutral
}

/// Status word and pill flavour of a listener tile (never colour alone).
func tileStatus(_ t: StationTile) -> (text: String, kind: PillKind) {
    switch t.connection {
    case .live:
        if case .granted = t.talk { return (tr("status_you_talk"), .voice) }
        if let talker = t.talker { return (tr("status_talking", talker), .voice) }
        if t.open && t.prefs.listen.mode == .activated { return (tr("status_sound"), .sound) }
        if t.prefs.listen.mode == .continuous { return (tr("status_live"), .calm) }
        return (tr("status_quiet"), .calm)
    case .connecting: return (tr("status_connecting"), .warn)
    case .reconnecting: return (tr("status_reconnecting"), .warn)
    case .needsPairing: return (tr("status_needs_pairing"), .warn)
    case .idle: return (tr("status_paused"), .neutral)
    }
}

/// Whether a tile shows sound (the coral halo).
func tileHasSound(_ t: StationTile) -> Bool {
    if case .live = t.connection { return t.open && t.prefs.listen.mode == .activated }
    return false
}

/// The label of the hold-to-talk button for a talk state.
func talkLabel(_ talk: TalkState) -> String {
    switch talk {
    case .granted: return tr("talking")
    case .requested: return tr("talk_wait")
    case let .busy(by): return by.map { tr("talk_busy", $0) } ?? tr("talk_busy_someone")
    case .disabled: return tr("talk_disabled")
    case .idle: return tr("hold_to_talk")
    }
}

/// A user-facing message for an engine error.
func errorText(_ e: Error, invalid: String = "err_generic") -> String {
    guard let e = e as? NestwatchError else { return tr("err_generic") }
    switch e {
    case let .Invalid(reason): return tr(reason.contains("station id") ? "err_invalid_id" : invalid)
    case .Unreachable: return tr("err_unreachable")
    case .Refused: return tr("err_refused")
    case .Rejected: return tr("err_rejected")
    case .TooManyStations: return tr("err_too_many")
    case .NotConnected: return tr("err_not_connected")
    default: return tr("err_generic")
    }
}
