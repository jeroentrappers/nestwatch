// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.platform

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.lull.ffi.DiscoveredService
import org.lull.ffi.TxtEntry
import java.net.InetAddress
import java.util.concurrent.Executors

/** DNS-SD service type (protocol.md §3.4). */
private const val SERVICE_TYPE = "_lull._tcp"
private const val TAG = "LullNsd"

/**
 * Advertises the station through Android NSD (platforms.md §2.2). NSD
 * cannot update TXT records in place, so a change (pairing opened or
 * closed, rename) re-registers the service.
 */
class NsdAdvertiser(context: Context) {
    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val main = Handler(Looper.getMainLooper())
    private var current: Triple<String, Int, List<TxtEntry>>? = null
    private var registered: NsdManager.RegistrationListener? = null
    private var pending: Triple<String, Int, List<TxtEntry>>? = null
    private var busy = false

    /** Advertises `name` on `port` with `txt`; repeats are ignored. */
    fun advertise(name: String, port: Int, txt: List<TxtEntry>) = main.post {
        val want = Triple(name, port, txt)
        if (want == current) {
            pending = null
            return@post
        }
        pending = want
        pump()
    }

    fun stop() = main.post {
        pending = null
        current = null
        busy = false
        registered?.let { l -> runCatching { nsd.unregisterService(l) } }
        registered = null
    }

    private fun pump() {
        if (busy) return
        val want = pending ?: return
        val old = registered
        if (old != null) {
            busy = true
            registered = null
            runCatching { nsd.unregisterService(old) }.onFailure { busy = false }
            // onServiceUnregistered continues.
            if (busy) return
        }
        pending = null
        val info = NsdServiceInfo().apply {
            serviceName = want.first.take(63).ifBlank { "Lull" }
            serviceType = SERVICE_TYPE
            port = want.second
            want.third.forEach { setAttribute(it.key, it.value) }
        }
        val l = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(s: NsdServiceInfo) {
                main.post { busy = false; pump() }
            }

            override fun onRegistrationFailed(s: NsdServiceInfo, error: Int) {
                Log.w(TAG, "registration failed: $error")
                main.post {
                    if (registered === this) {
                        registered = null
                        current = null
                    }
                    busy = false
                    pump()
                }
            }

            override fun onServiceUnregistered(s: NsdServiceInfo) {
                main.post { busy = false; pump() }
            }

            override fun onUnregistrationFailed(s: NsdServiceInfo, error: Int) {
                main.post { busy = false; pump() }
            }
        }
        busy = true
        registered = l
        current = want
        try {
            nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, l)
        } catch (e: Exception) {
            Log.w(TAG, "register: ${e.javaClass.simpleName}")
            registered = null
            busy = false
        }
    }
}

/**
 * Browses for stations through Android NSD and reports resolved services
 * (platforms.md §2.2) while holding a [Multicast] lock. Resolves run one at
 * a time (NSD rejects concurrent resolves before API 34).
 */
class NsdBrowser(
    context: Context,
    private val onFound: (DiscoveredService) -> Unit,
    private val onLost: (String) -> Unit,
) {
    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val multicast = Multicast(context)
    private val main = Handler(Looper.getMainLooper())
    private val queue = ArrayDeque<NsdServiceInfo>()
    private var resolving = false
    private var discovery: NsdManager.DiscoveryListener? = null
    private val executor = Executors.newSingleThreadExecutor()

    fun start() = main.post {
        if (discovery != null) return@post
        multicast.acquire()
        val l = object : NsdManager.DiscoveryListener {
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w(TAG, "discovery failed: $errorCode")
                main.post { if (discovery === this) discovery = null }
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}

            override fun onServiceFound(s: NsdServiceInfo) {
                main.post {
                    queue.addLast(s)
                    resolveNext()
                }
            }

            override fun onServiceLost(s: NsdServiceInfo) {
                val name = fullname(s)
                executor.execute { onLost(name) }
            }
        }
        discovery = l
        try {
            nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, l)
        } catch (e: Exception) {
            Log.w(TAG, "discover: ${e.javaClass.simpleName}")
            discovery = null
        }
    }

    fun stop() = main.post {
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        discovery = null
        queue.clear()
        multicast.release()
    }

    private fun fullname(s: NsdServiceInfo) = "${s.serviceName}.$SERVICE_TYPE.local."

    @Suppress("DEPRECATION")
    private fun resolveNext() {
        if (resolving || discovery == null) return
        val s = queue.removeFirstOrNull() ?: return
        resolving = true
        val l = object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
                main.post {
                    resolving = false
                    resolveNext()
                }
            }

            override fun onServiceResolved(info: NsdServiceInfo) {
                val addrs: List<InetAddress> = if (Build.VERSION.SDK_INT >= 34) {
                    info.hostAddresses
                } else {
                    listOfNotNull(info.host)
                }
                val txt = info.attributes.mapNotNull { (k, v) ->
                    v?.let { TxtEntry(k, String(it, Charsets.UTF_8)) }
                }
                val found = DiscoveredService(
                    fullname = fullname(info),
                    name = info.serviceName,
                    host = "",
                    addrs = addrs.mapNotNull { a -> a.hostAddress?.substringBefore('%') },
                    port = info.port.toUShort(),
                    txt = txt,
                )
                executor.execute { onFound(found) }
                main.post {
                    resolving = false
                    resolveNext()
                }
            }
        }
        try {
            nsd.resolveService(s, l)
        } catch (e: Exception) {
            resolving = false
            main.post { resolveNext() }
        }
    }
}
