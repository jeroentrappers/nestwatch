// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.platform

import org.nestwatch.ffi.InterfaceAddress
import java.net.Inet6Address
import java.net.NetworkInterface

/** Interface addresses for the station's invitation endpoints (protocol.md §3.5). */
object NetInfo {
    /**
     * Every address of every interface that is up, with its interface
     * name (`wlan0`, `tun0`, ...). The core orders them and drops loopback
     * and link-local addresses; VPN interfaces are recognised there.
     */
    fun interfaces(): List<InterfaceAddress> = try {
        NetworkInterface.getNetworkInterfaces()?.toList().orEmpty()
            .filter { runCatching { it.isUp && !it.isLoopback }.getOrDefault(false) }
            .flatMap { ni ->
                ni.inetAddresses.toList().mapNotNull { a ->
                    val ip = a.hostAddress ?: return@mapNotNull null
                    // Zone ids (`%wlan0`) cannot go into endpoints.
                    val clean = if (a is Inet6Address) ip.substringBefore('%') else ip
                    InterfaceAddress(name = ni.name, ip = clean)
                }
            }
    } catch (_: Exception) {
        emptyList()
    }
}
