// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.platform

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log

/**
 * Reports network changes (an interface comes or goes, Wi-Fi to mobile) so
 * the listener reconnects at once and relayed sessions try a direct path
 * again (FR-LS-5, rendezvous.md §5.2 item 5).
 */
class NetworkWatch(context: Context, private val onChange: () -> Unit) {
    private val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private var registered = false
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = onChange()
        override fun onLost(network: Network) = onChange()
    }

    fun start() {
        if (registered) return
        try {
            val req = NetworkRequest.Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                .build()
            cm.registerNetworkCallback(req, callback)
            registered = true
        } catch (e: Exception) {
            Log.w("NestwatchNet", "network callback: ${e.javaClass.simpleName}")
        }
    }

    fun stop() {
        if (!registered) return
        registered = false
        runCatching { cm.unregisterNetworkCallback(callback) }
    }
}
