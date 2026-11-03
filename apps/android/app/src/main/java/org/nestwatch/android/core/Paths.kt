// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.core

import org.nestwatch.android.R
import org.nestwatch.ffi.NetworkPath
import org.nestwatch.ffi.RelayStatus

/** Tile path label (FR-LS-3, rendezvous.md FR-RV-4). */
fun pathLabel(path: NetworkPath): Int = when (path) {
    NetworkPath.LAN -> R.string.path_lan
    NetworkPath.VPN -> R.string.path_vpn
    NetworkPath.DIRECT -> R.string.path_direct
    NetworkPath.RELAYED -> R.string.path_relayed
}

/** How a relay status is shown: the pill's word and its colour role. */
enum class RelayLook { Off, Connecting, Online, Error }

/** Maps the station's relay registration to its pill (rendezvous.md FR-RV-1). */
fun relayLook(status: RelayStatus): RelayLook = when (status) {
    is RelayStatus.Off -> RelayLook.Off
    is RelayStatus.Connecting -> RelayLook.Connecting
    is RelayStatus.Online -> RelayLook.Online
    is RelayStatus.Error -> RelayLook.Error
}

/** The pill text of a [RelayLook]. */
fun relayLabel(look: RelayLook): Int = when (look) {
    RelayLook.Off -> R.string.relay_status_off
    RelayLook.Connecting -> R.string.relay_status_connecting
    RelayLook.Online -> R.string.relay_status_online
    RelayLook.Error -> R.string.relay_status_error
}
