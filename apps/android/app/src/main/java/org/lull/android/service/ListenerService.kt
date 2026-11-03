// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.service

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioManager
import android.media.ToneGenerator
import android.os.Build
import android.util.Log
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.lull.android.R
import org.lull.android.audio.MicCapture
import org.lull.android.audio.SpeakerPlayback
import org.lull.android.core.Lull
import org.lull.android.platform.KeepAwake
import org.lull.android.platform.Notifications
import org.lull.android.platform.NsdBrowser
import org.lull.ffi.AlertKind
import org.lull.ffi.Connection
import org.lull.ffi.Listener
import org.lull.ffi.ListenerEventInfo

/**
 * Keeps a listener receiving and alerting with the screen off (FR-LS-4,
 * FR-AL-4, platforms.md §2.2): a foreground service of type
 * mediaPlayback (plain on API 23–28) that holds a partial wake lock and a
 * Wi-Fi lock, plays the mix, captures talk-back while granted, browses
 * NSD and turns alerts into notifications.
 */
class ListenerService : LifecycleService() {
    private lateinit var awake: KeepAwake
    private var playback: SpeakerPlayback? = null
    private var talkMic: MicCapture? = null
    private var browser: NsdBrowser? = null
    private var lastNote: String? = null

    override fun onCreate() {
        super.onCreate()
        awake = KeepAwake(this, "listener")
        val n = Notifications.ongoing(this, getString(R.string.notif_listener_title), getString(R.string.notif_listener_idle))
        val type = if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK else 0
        try {
            ServiceCompat.startForeground(this, Notifications.ID_LISTENER, n, type)
        } catch (e: Exception) {
            Log.w(TAG, "startForeground: ${e.javaClass.simpleName}")
        }
        val l = Lull.listener()
        if (l == null) {
            stopSelf()
            return
        }
        awake.acquire()
        running.value = true
        playback = SpeakerPlayback("playback") { frames -> l.pullPlayback(frames.toUInt()) }.also { it.start() }
        browser = NsdBrowser(
            this,
            onFound = { s -> runCatching { l.addDiscovered(s) } },
            onLost = { name -> runCatching { l.removeDiscovered(name) } },
        ).also { it.start() }
        lifecycleScope.launch { Lull.listenerEvents.collect { onEvent(l, it) } }
        lifecycleScope.launch {
            Lull.listenerState.collect { s ->
                s ?: return@collect
                val followed = s.stations.count { it.prefs.follow }
                val live = s.stations.count { it.connection == Connection.Live }
                val text = if (followed == 0) {
                    getString(R.string.notif_listener_idle)
                } else {
                    resources.getQuantityString(R.plurals.notif_listener_text, followed, followed, live)
                }
                if (text != lastNote) {
                    lastNote = text
                    Notifications.update(
                        this@ListenerService,
                        Notifications.ID_LISTENER,
                        Notifications.ongoing(this@ListenerService, getString(R.string.notif_listener_title), text),
                    )
                }
            }
        }
    }

    private fun stationName(id: String): String =
        Lull.listenerState.value?.stations?.firstOrNull { it.id == id }?.name ?: getString(R.string.unnamed)

    private fun onEvent(l: Listener, e: ListenerEventInfo) {
        when (e) {
            is ListenerEventInfo.Alert -> alert(e.stationId, e.alert)
            is ListenerEventInfo.MicrophoneWanted -> {
                if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) ==
                    PackageManager.PERMISSION_GRANTED && talkMic == null
                ) {
                    talkMic = MicCapture("talk-mic") { pcm, rate -> l.pushTalkCapture(pcm, rate.toUInt(), 1u) }
                        .also { if (!it.start()) Log.w(TAG, "no microphone for talk-back") }
                }
            }
            is ListenerEventInfo.MicrophoneNotWanted -> {
                talkMic?.stop()
                talkMic = null
            }
            else -> {}
        }
    }

    private fun alert(id: String, kind: AlertKind) {
        val name = stationName(id)
        val (channel, title, slot) = when (kind) {
            is AlertKind.SoundStarted -> Triple(Notifications.CH_SOUND, getString(R.string.alert_sound, name), 0)
            is AlertKind.Unreachable -> Triple(Notifications.CH_CONNECTION, getString(R.string.alert_unreachable, name), 1)
            is AlertKind.Reachable -> Triple(Notifications.CH_CONNECTION, getString(R.string.alert_reachable, name), 1)
            is AlertKind.BatteryLow ->
                Triple(Notifications.CH_BATTERY, getString(R.string.alert_battery, name, kind.percent.toInt()), 2)
        }
        val nid = 1000 + (id.hashCode() and 0xffff) * 4 + slot
        if (Notifications.canPost(this)) {
            Notifications.alert(this, channel, nid, title, getString(R.string.alert_body))
        } else if (channel != Notifications.CH_BATTERY) {
            // Without notification permission an alert still makes a sound,
            // even while muted (FR-AL-1, FR-AL-4).
            runCatching {
                val tone = ToneGenerator(AudioManager.STREAM_ALARM, 80)
                tone.startTone(ToneGenerator.TONE_PROP_BEEP2, 400)
                lifecycleScope.launch {
                    kotlinx.coroutines.delay(600)
                    tone.release()
                }
            }
        }
    }

    override fun onDestroy() {
        running.value = false
        playback?.stop()
        talkMic?.stop()
        browser?.stop()
        if (::awake.isInitialized) awake.release()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "LullListener"
        private val running = MutableStateFlow(false)

        /** Whether the service runs. */
        val isRunning: StateFlow<Boolean> = running.asStateFlow()

        fun start(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, ListenerService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, ListenerService::class.java))
        }
    }
}
