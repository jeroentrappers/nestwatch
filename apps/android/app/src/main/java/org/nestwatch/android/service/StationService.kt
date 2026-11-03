// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.service

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.os.Build
import android.util.Log
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.nestwatch.android.R
import org.nestwatch.android.audio.MicCapture
import org.nestwatch.android.audio.SpeakerPlayback
import org.nestwatch.android.camera.StationCamera
import org.nestwatch.android.core.Nestwatch
import org.nestwatch.android.core.StationRun
import org.nestwatch.android.platform.BatteryWatch
import org.nestwatch.android.platform.KeepAwake
import org.nestwatch.android.platform.Notifications
import org.nestwatch.android.platform.NsdAdvertiser
import org.nestwatch.ffi.CameraAvailability
import org.nestwatch.ffi.Station
import org.nestwatch.ffi.StationEventInfo

/**
 * Keeps a station running with the screen off (FR-ST-5, platforms.md
 * §2.2): a foreground service of type microphone (plus camera when the
 * permission is granted; plain on API 23–28) that holds a partial wake
 * lock and a Wi-Fi lock, captures the microphone, plays talk-back, runs
 * the camera on demand, reports the battery and advertises over NSD.
 */
class StationService : LifecycleService() {
    private lateinit var awake: KeepAwake
    private var mic: MicCapture? = null
    private var talkback: SpeakerPlayback? = null
    private var camera: StationCamera? = null
    private var battery: BatteryWatch? = null
    private var nsd: NsdAdvertiser? = null
    private var lastNote: String? = null

    override fun onCreate() {
        super.onCreate()
        awake = KeepAwake(this, "station")
        startInForeground()
        awake.acquire()
        running.value = true
        lifecycleScope.launch {
            val s = Nestwatch.startStation()
            if (s == null) {
                stopSelf()
                return@launch
            }
            setUp(s)
        }
    }

    private fun granted(p: String) =
        ContextCompat.checkSelfPermission(this, p) == PackageManager.PERMISSION_GRANTED

    private fun startInForeground() {
        // Types exist from API 30 (camera, microphone); plain below.
        var types = 0
        if (Build.VERSION.SDK_INT >= 30 && granted(Manifest.permission.RECORD_AUDIO)) {
            types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        }
        // The camera type needs the permission when the service starts
        // (API 34); without it the station reports the camera as denied.
        if (Build.VERSION.SDK_INT >= 30 && granted(Manifest.permission.CAMERA)) {
            types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        }
        val n = Notifications.ongoing(this, getString(R.string.notif_station_title), getString(R.string.notif_station_starting))
        try {
            ServiceCompat.startForeground(this, Notifications.ID_STATION, n, types)
        } catch (e: Exception) {
            Log.w(TAG, "startForeground: ${e.javaClass.simpleName}")
            ServiceCompat.startForeground(this, Notifications.ID_STATION, n, 0)
        }
    }

    private fun setUp(s: Station) {
        val m = MicCapture("station-mic") { pcm, rate -> s.pushCaptureI16(pcm, rate.toUInt(), 1u) }
        if (!granted(Manifest.permission.RECORD_AUDIO) || !m.start()) {
            Log.w(TAG, "station runs without a microphone")
        }
        mic = m
        val hasCamera = packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)
        s.setCameraAvailability(
            when {
                !hasCamera -> CameraAvailability.UNAVAILABLE
                !granted(Manifest.permission.CAMERA) -> CameraAvailability.DENIED
                else -> CameraAvailability.AVAILABLE
            },
        )
        camera = StationCamera(this, this, sink = { img -> s.pushCameraFrame(img) }, onUnavailable = {
            s.setCameraAvailability(CameraAvailability.UNAVAILABLE)
        })
        battery = BatteryWatch(this) { p, c -> s.setBattery(p?.toUByte(), c) }.also { it.start() }
        val adv = NsdAdvertiser(this)
        nsd = adv
        lifecycleScope.launch {
            Nestwatch.station.collect { run ->
                if (run !is StationRun.Running) return@collect
                val snap = run.snapshot
                adv.advertise(snap.name, snap.tcpPort.toInt(), snap.mdnsTxt)
                val text = when (val n = snap.listeners.size) {
                    0 -> getString(R.string.nobody_listening)
                    else -> resources.getQuantityString(R.plurals.listening_count, n, n)
                } + if (snap.cameraOn) " · " + getString(R.string.camera_on) else ""
                if (text != lastNote) {
                    lastNote = text
                    Notifications.update(
                        this@StationService,
                        Notifications.ID_STATION,
                        Notifications.ongoing(this@StationService, getString(R.string.notif_station_title), text),
                    )
                }
            }
        }
        lifecycleScope.launch {
            Nestwatch.stationEvents.collect { e ->
                withContext(Dispatchers.Main) { onEvent(s, e) }
            }
        }
    }

    private fun onEvent(s: Station, e: StationEventInfo) {
        when (e) {
            is StationEventInfo.CameraWanted ->
                if (granted(Manifest.permission.CAMERA)) camera?.start(e.request)
                else s.setCameraAvailability(CameraAvailability.DENIED)
            is StationEventInfo.CameraNotWanted -> camera?.stop()
            is StationEventInfo.TalkStarted -> {
                if (talkback == null) {
                    talkback = SpeakerPlayback("talkback", AudioAttributes.USAGE_MEDIA) { n ->
                        s.pullTalkback(n.toUInt()).pcm
                    }.also { it.start() }
                }
            }
            is StationEventInfo.TalkEnded -> {
                talkback?.stop()
                talkback = null
            }
            else -> {}
        }
    }

    override fun onDestroy() {
        running.value = false
        mic?.stop()
        talkback?.stop()
        camera?.close()
        battery?.stop()
        nsd?.stop()
        awake.release()
        Nestwatch.scope.launch { Nestwatch.stopStation() }
        super.onDestroy()
    }

    companion object {
        private const val TAG = "NestwatchStation"
        private val running = MutableStateFlow(false)

        /** Whether the service runs. */
        val isRunning: StateFlow<Boolean> = running.asStateFlow()

        fun start(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, StationService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, StationService::class.java))
        }
    }
}
