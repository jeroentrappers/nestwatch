// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.screens

import android.Manifest
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import org.lull.android.R
import org.lull.android.core.Lull
import org.lull.android.core.StationRun
import org.lull.android.platform.BatteryOptimisation
import org.lull.android.ui.hasPermission
import org.lull.android.ui.notificationPermission
import org.lull.android.ui.openAppSettings
import org.lull.android.ui.rememberPermissionAsk
import org.lull.android.ui.requestBatteryExemption
import org.lull.android.ui.startStation
import org.lull.android.ui.stopStation
import org.lull.android.ui.components.Caption
import org.lull.android.ui.components.CodeBox
import org.lull.android.ui.components.DangerButton
import org.lull.android.ui.components.LevelRing
import org.lull.android.ui.components.LullCard
import org.lull.android.ui.components.Pill
import org.lull.android.ui.components.PillKind
import org.lull.android.ui.components.PrimaryButton
import org.lull.android.ui.components.QrImage
import org.lull.android.ui.components.SecondaryButton
import org.lull.android.ui.theme.MonoCode
import org.lull.android.ui.theme.lull
import org.lull.ffi.ListenMode
import org.lull.ffi.StationSnapshot
import org.lull.ffi.groupedId
import org.lull.ffi.qrCode

/** The station screen (README §5.2, §5.3): state first, then actions. */
@Composable
fun StationHome(dimmed: Boolean, onDim: (Boolean) -> Unit, onSettings: () -> Unit, onAbout: () -> Unit) {
    val run by Lull.station.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var micDenied by remember { mutableStateOf(false) }
    // Microphone (required), then camera (optional) and notifications.
    val askOptional = rememberPermissionAsk(
        listOf(Manifest.permission.CAMERA) + notificationPermission,
        R.string.perm_camera_title,
        R.string.perm_camera_station,
    ) { startStation(context) }
    val askMic = rememberPermissionAsk(
        listOf(Manifest.permission.RECORD_AUDIO),
        R.string.perm_mic_title,
        R.string.perm_mic_station,
    ) { ok ->
        micDenied = !ok
        if (ok) askOptional.ask()
    }
    val start = { askMic.ask() }

    val r = run
    if (dimmed && r is StationRun.Running) {
        DimmedStation(r.snapshot) { onDim(false) }
        return
    }
    Page(
        title = (r as? StationRun.Running)?.snapshot?.name ?: stringResource(R.string.app_name),
        onBack = null,
        actions = {
            TextButton(onClick = onAbout) { Text(stringResource(R.string.about), color = lull.text2) }
            TextButton(onClick = onSettings) { Text(stringResource(R.string.settings), color = lull.text2) }
        },
    ) {
        when (r) {
            StationRun.Stopped -> LullCard {
                Text(stringResource(R.string.station_stopped), color = lull.text1)
                if (micDenied) {
                    Text(stringResource(R.string.perm_denied_mic), color = lull.amber)
                    SecondaryButton(stringResource(R.string.open_settings), { openAppSettings(context) })
                }
                PrimaryButton(stringResource(R.string.station_start), start, Modifier.fillMaxWidth())
            }
            StationRun.Starting -> Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(color = lull.honey)
                Spacer(Modifier.padding(8.dp))
                Text(stringResource(R.string.station_starting), color = lull.text2)
            }
            is StationRun.Failed -> LullCard {
                Text(stringResource(R.string.station_failed), color = lull.coral)
                Caption(r.reason)
                PrimaryButton(stringResource(R.string.retry), start)
            }
            is StationRun.Running -> RunningStation(r.snapshot, onDim = { onDim(true) })
        }
        BatteryNotice()
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun StatusPills(s: StationSnapshot) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val n = s.listeners.size
        Pill(
            if (n == 0) stringResource(R.string.nobody_listening) else pluralStringResource(R.plurals.listening_count, n, n),
            if (n == 0) PillKind.Neutral else PillKind.Calm,
        )
        Pill(
            stringResource(if (s.cameraOn) R.string.camera_on else R.string.camera_off),
            if (s.cameraOn) PillKind.Cam else PillKind.Neutral,
        )
        s.talker?.let { Pill(stringResource(R.string.talking_through, it), PillKind.Voice) }
        if (!s.settings.talkEnabled) Pill(stringResource(R.string.talk_off), PillKind.Neutral)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RunningStation(s: StationSnapshot, onDim: () -> Unit) {
    val context = LocalContext.current
    var pairing by remember { mutableStateOf(false) }
    StatusPills(s)
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        LevelRing(s.levelDbfs, s.strictestThresholdDbfs, s.soundOpen)
    }
    LullCard {
        Text(stringResource(R.string.listeners_heading), style = MaterialTheme.typography.titleMedium, color = lull.text1)
        if (s.listeners.isEmpty()) Caption(stringResource(R.string.no_listeners))
        s.listeners.forEach { l ->
            Column {
                Text(l.name.ifBlank { stringResource(R.string.unnamed) }, color = lull.text1)
                val bits = buildList {
                    add(stringResource(if (l.mode == ListenMode.ACTIVATED) R.string.mode_sound_activated else R.string.mode_always_on))
                    if (l.web) add(stringResource(R.string.via_browser))
                    if (l.open) add(stringResource(R.string.hearing))
                    if (l.talking) add(stringResource(R.string.talking_now))
                    if (l.watching) add(stringResource(R.string.watching_now))
                }
                Caption(bits.joinToString(" · "))
            }
        }
    }
    PrimaryButton(stringResource(R.string.pair_listener), { pairing = true }, Modifier.fillMaxWidth())
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        SecondaryButton(stringResource(R.string.dim_screen), onDim, Modifier.weight(1f))
        DangerButton(stringResource(R.string.station_stop), { stopStation(context) }, Modifier.weight(1f))
    }
    Caption(stringResource(R.string.station_keeps_running))
    if (pairing) {
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(
            onDismissRequest = { pairing = false },
            sheetState = sheet,
            containerColor = lull.ink1,
        ) { PairingSheet(s, onClose = { pairing = false }) }
    }
}

/** The pairing sheet (README §5.4): QR, code, time left, verification numbers. */
@Composable
private fun PairingSheet(s: StationSnapshot, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    var failed by remember { mutableStateOf(false) }
    var seen by remember { mutableStateOf(false) }
    val justPaired by Lull.justPaired.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) {
        if (s.pairing == null) {
            try {
                Lull.stationEngine?.openPairing()
            } catch (_: Exception) {
                failed = true
            }
        }
    }
    Column(
        Modifier
            .fillMaxWidth()
            .verticalScroll(rememberScrollState())
            .navigationBarsPadding()
            .padding(horizontal = 24.dp, vertical = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(stringResource(R.string.pair_listener), style = MaterialTheme.typography.titleLarge, color = lull.text1)
        val p = s.pairing
        if (p != null) androidx.compose.runtime.SideEffect { seen = true }
        when {
            p != null -> {
                Text(stringResource(R.string.pair_intro), color = lull.text2, textAlign = TextAlign.Center)
                val qr = remember(p.inviteUri) { qrCode(p.inviteUri) }
                if (qr != null) {
                    QrImage(qr, stringResource(R.string.qr_label), Modifier.widthIn(max = 280.dp).fillMaxWidth())
                }
                Caption(stringResource(R.string.pair_code_intro))
                CodeBox(p.code)
                Caption(stringResource(R.string.pair_station_id))
                Text(groupedId(s.deviceId), style = MonoCode, color = lull.text2)
                val left = (p.maxPairings - p.pairings).toInt().coerceAtLeast(0)
                Caption(
                    stringResource(
                        R.string.pair_open_for,
                        clock(p.remainingMs.toLong()),
                        pluralStringResource(R.plurals.pair_devices_left, left, left),
                    ),
                )
                if (justPaired.isNotEmpty()) {
                    LullCard {
                        Text(stringResource(R.string.pair_just_paired), color = lull.text1)
                        justPaired.forEach { (name, number) ->
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(name, Modifier.weight(1f), color = lull.text1)
                                CodeBox(number)
                            }
                        }
                    }
                }
                DangerButton(stringResource(R.string.pair_close), {
                    Lull.withStation { it.closePairing() }
                    onClose()
                })
            }
            failed -> Text(stringResource(R.string.err_generic), color = lull.coral)
            seen -> {
                Text(stringResource(R.string.pair_closed), color = lull.text2)
                PrimaryButton(stringResource(R.string.pair_listener), {
                    scope.launch { runCatching { Lull.stationEngine?.openPairing() } }
                })
            }
            else -> Text(stringResource(R.string.pair_opening), color = lull.text2)
        }
        Spacer(Modifier.height(8.dp))
        SecondaryButton(stringResource(R.string.close), onClose, Modifier.fillMaxWidth())
        Spacer(Modifier.height(16.dp))
    }
}

/** Dimmed station (README §5.3): ring and listener count only; tap to brighten. */
@Composable
private fun DimmedStation(s: StationSnapshot, onBrighten: () -> Unit) {
    val label = stringResource(R.string.dimmed_label)
    Box(
        Modifier
            .fillMaxSize()
            .background(lull.ink0)
            .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onBrighten)
            .semantics { contentDescription = label }
            .windowInsetsPadding(WindowInsets.safeDrawing),
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(16.dp)) {
            LevelRing(s.levelDbfs, s.strictestThresholdDbfs, s.soundOpen, size = 160.dp, showNumbers = false)
            val n = s.listeners.size
            Text(
                if (n == 0) stringResource(R.string.nobody_listening) else pluralStringResource(R.plurals.listening_count, n, n),
                color = lull.text2,
            )
            // The camera indicator stays visible while dimmed (FR-CAM-2).
            if (s.cameraOn) Pill(stringResource(R.string.camera_on), PillKind.Cam)
            s.talker?.let { Pill(stringResource(R.string.talking_through, it), PillKind.Voice) }
            Caption(stringResource(R.string.tap_to_brighten))
        }
    }
}

/** Offer the battery optimisation exemption, explaining why (platforms.md §2.2). */
@Composable
fun BatteryNotice() {
    val context = LocalContext.current
    val prefs by Lull.prefs.flow.collectAsStateWithLifecycle()
    var exempt by remember { mutableStateOf(BatteryOptimisation.isExempt(context)) }
    androidx.lifecycle.compose.LifecycleResumeEffect(Unit) {
        exempt = BatteryOptimisation.isExempt(context)
        onPauseOrDispose {}
    }
    if (exempt || prefs.batteryNoticeDismissed) return
    LullCard {
        Text(stringResource(R.string.battery_title), style = MaterialTheme.typography.titleMedium, color = lull.amber)
        Text(stringResource(R.string.battery_body), color = lull.text2)
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            PrimaryButton(stringResource(R.string.battery_allow), { requestBatteryExemption(context) })
            TextButton(onClick = { Lull.prefs.update { it.copy(batteryNoticeDismissed = true) } }) {
                Text(stringResource(R.string.battery_dismiss), color = lull.text2)
            }
        }
    }
}

/** Whether the microphone is granted (for status). */
@Composable
fun micGranted(): Boolean = LocalContext.current.hasPermission(Manifest.permission.RECORD_AUDIO)
