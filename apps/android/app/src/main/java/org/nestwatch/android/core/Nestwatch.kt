// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.core

import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.nestwatch.android.platform.NetInfo
import org.nestwatch.ffi.Connection
import org.nestwatch.ffi.Listener
import org.nestwatch.ffi.ListenerEventInfo
import org.nestwatch.ffi.ListenerObserver
import org.nestwatch.ffi.ListenerOptions
import org.nestwatch.ffi.ListenerSnapshot
import org.nestwatch.ffi.NestwatchException
import org.nestwatch.ffi.Station
import org.nestwatch.ffi.StationEventInfo
import org.nestwatch.ffi.StationObserver
import org.nestwatch.ffi.StationOptions
import org.nestwatch.ffi.StationSnapshot
import org.nestwatch.ffi.VideoFrame

/** Station lifecycle as the UI sees it. */
sealed interface StationRun {
    data object Stopped : StationRun
    data object Starting : StationRun
    data class Running(val snapshot: StationSnapshot) : StationRun
    data class Failed(val reason: String) : StationRun
}

/**
 * Process-wide owner of the engines (architecture.md §5): starts and stops
 * the [Station] and [Listener] from nestwatch-ffi and republishes their
 * snapshots, events and video frames as flows for the UI and the
 * foreground services. Observer callbacks arrive on engine threads; they
 * only update flows.
 */
object Nestwatch {
    private const val TAG = "Nestwatch"

    lateinit var appContext: Context
        private set
    lateinit var prefs: Prefs
        private set
    val storage: KeystoreStorage by lazy { KeystoreStorage(appContext) }
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    fun init(context: Context) {
        appContext = context.applicationContext
        prefs = Prefs(appContext)
    }

    /** A default display name for this device. */
    fun deviceName(): String = Build.MODEL?.takeIf { it.isNotBlank() } ?: "Android"

    // ----- station ---------------------------------------------------------

    private val stationLock = Mutex()
    private val stationRun = MutableStateFlow<StationRun>(StationRun.Stopped)
    val station: StateFlow<StationRun> = stationRun.asStateFlow()
    private val stationEventFlow = MutableSharedFlow<StationEventInfo>(extraBufferCapacity = 64)
    val stationEvents: SharedFlow<StationEventInfo> = stationEventFlow.asSharedFlow()

    /** Listeners paired in the open window with their verification numbers. */
    private val justPairedFlow = MutableStateFlow<List<Pair<String, String>>>(emptyList())
    val justPaired: StateFlow<List<Pair<String, String>>> = justPairedFlow.asStateFlow()

    @Volatile
    var stationEngine: Station? = null
        private set

    private val stationObserver = object : StationObserver {
        override fun onState(state: StationSnapshot) {
            if (stationEngine != null) stationRun.value = StationRun.Running(state)
        }

        override fun onEvent(event: StationEventInfo) {
            when (event) {
                is StationEventInfo.Paired ->
                    justPairedFlow.update { it + (event.name to event.verification) }
                is StationEventInfo.PairingOpened -> justPairedFlow.value = emptyList()
                else -> {}
            }
            stationEventFlow.tryEmit(event)
        }
    }

    /** Starts the station engine (called by the station service). */
    suspend fun startStation(): Station? = stationLock.withLock {
        stationEngine?.let { return it }
        stationRun.value = StationRun.Starting
        try {
            val options = StationOptions(
                name = deviceName(),
                port = 0u,
                // Advertising goes through NSD (platforms.md §2.2).
                mdns = false,
                interfaces = NetInfo.interfaces(),
                extraEndpoints = emptyList(),
                loopback = false,
            )
            val s = org.nestwatch.ffi.startStation(options, storage, stationObserver)
            stationEngine = s
            stationRun.value = StationRun.Running(s.snapshot())
            s
        } catch (e: NestwatchException) {
            Log.w(TAG, "station failed to start: ${e.javaClass.simpleName}")
            stationRun.value = StationRun.Failed(e.message ?: e.javaClass.simpleName)
            null
        }
    }

    /** Stops the station engine. */
    suspend fun stopStation() = stationLock.withLock {
        val s = stationEngine ?: run {
            if (stationRun.value !is StationRun.Failed) stationRun.value = StationRun.Stopped
            return@withLock
        }
        stationEngine = null
        s.stop()
        s.destroy()
        justPairedFlow.value = emptyList()
        stationRun.value = StationRun.Stopped
    }

    // ----- listener --------------------------------------------------------

    private val listenerStateFlow = MutableStateFlow<ListenerSnapshot?>(null)
    val listenerState: StateFlow<ListenerSnapshot?> = listenerStateFlow.asStateFlow()
    private val listenerEventFlow = MutableSharedFlow<ListenerEventInfo>(extraBufferCapacity = 64)
    val listenerEvents: SharedFlow<ListenerEventInfo> = listenerEventFlow.asSharedFlow()
    private val framesFlow = MutableStateFlow<Map<String, VideoFrame>>(emptyMap())
    val frames: StateFlow<Map<String, VideoFrame>> = framesFlow.asStateFlow()

    private val histories = HashMap<String, LevelHistory>()
    private val historyTick = MutableStateFlow(0L)

    /** Bumps whenever a level history changes (at most every 200 ms). */
    val historyVersion: StateFlow<Long> = historyTick.asStateFlow()

    @Volatile
    var listenerEngine: Listener? = null
        private set

    private val listenerObserver = object : ListenerObserver {
        override fun onState(state: ListenerSnapshot) {
            listenerStateFlow.value = state
            recordLevels(state)
        }

        override fun onEvent(event: ListenerEventInfo) {
            if (event is ListenerEventInfo.Forgotten) {
                framesFlow.update { it - event.stationId }
                synchronized(histories) { histories.remove(event.stationId) }
            }
            listenerEventFlow.tryEmit(event)
        }

        override fun onVideoFrame(frame: VideoFrame) {
            framesFlow.update { it + (frame.stationId to frame) }
        }
    }

    private fun recordLevels(state: ListenerSnapshot) {
        val now = SystemClock.elapsedRealtime()
        var changed = false
        synchronized(histories) {
            for (t in state.stations) {
                val level = t.levelDbfs ?: continue
                if (t.connection != Connection.Live) continue
                val h = histories.getOrPut(t.id) { LevelHistory() }
                val before = h.size
                h.add(now, level)
                changed = changed || h.size != before
            }
        }
        if (changed) historyTick.value = now
    }

    /** Chart points of a station's last minute (age 0..1, dBFS). */
    fun history(stationId: String): List<Pair<Float, Float>> = synchronized(histories) {
        histories[stationId]?.series(SystemClock.elapsedRealtime()) ?: emptyList()
    }

    /** Starts the listener engine if needed and returns it. */
    @Synchronized
    fun listener(): Listener? {
        listenerEngine?.let { return it }
        return try {
            val l = Listener.start(
                ListenerOptions(name = deviceName(), discovery = false),
                storage,
                listenerObserver,
            )
            listenerEngine = l
            listenerStateFlow.value = l.snapshot()
            l
        } catch (e: NestwatchException) {
            Log.w(TAG, "listener failed to start: ${e.javaClass.simpleName}")
            null
        }
    }

    /** Stops the listener engine. */
    fun stopListener() {
        val l = synchronized(this) {
            val l = listenerEngine ?: return
            listenerEngine = null
            l
        }
        scope.launch {
            l.stop()
            l.destroy()
            listenerStateFlow.value = null
            framesFlow.value = emptyMap()
        }
    }

    /** Runs a listener intent, ignoring "not connected" style failures. */
    inline fun withListener(block: (Listener) -> Unit) {
        val l = listenerEngine ?: return
        try {
            block(l)
        } catch (e: NestwatchException) {
            Log.i("Nestwatch", "listener intent failed: ${e.javaClass.simpleName}")
        }
    }

    /** Runs a station intent if the station runs. */
    inline fun withStation(block: (Station) -> Unit) {
        val s = stationEngine ?: return
        try {
            block(s)
        } catch (e: NestwatchException) {
            Log.i("Nestwatch", "station intent failed: ${e.javaClass.simpleName}")
        }
    }
}
