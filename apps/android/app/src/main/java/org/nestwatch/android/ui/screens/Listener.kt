// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.ui.screens

import android.Manifest
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.nestwatch.android.R
import org.nestwatch.android.core.Nestwatch
import org.nestwatch.android.core.pathLabel
import org.nestwatch.android.service.ListenerService
import org.nestwatch.android.ui.hasPermission
import org.nestwatch.android.ui.notificationPermission
import org.nestwatch.android.ui.rememberPermissionAsk
import org.nestwatch.android.ui.startListening
import org.nestwatch.android.ui.components.Caption
import org.nestwatch.android.ui.components.HoldToTalk
import org.nestwatch.android.ui.components.LevelMeter
import org.nestwatch.android.ui.components.NestwatchCard
import org.nestwatch.android.ui.components.Pill
import org.nestwatch.android.ui.components.PillKind
import org.nestwatch.android.ui.components.PrimaryButton
import org.nestwatch.android.ui.components.SecondaryButton
import org.nestwatch.android.ui.theme.nestwatch
import org.nestwatch.ffi.Connection
import org.nestwatch.ffi.ListenMode
import org.nestwatch.ffi.StationTile
import org.nestwatch.ffi.TalkState
import kotlin.math.roundToInt

/** Listener home (README §5.6): stacked station tiles on a phone. */
@Composable
fun ListenerHome(
    onAdd: () -> Unit,
    onStation: (String) -> Unit,
    onCamera: (String) -> Unit,
    onAbout: () -> Unit,
    onSwitchRole: () -> Unit,
) {
    val context = LocalContext.current
    val state by Nestwatch.listenerState.collectAsStateWithLifecycle()
    val serviceRunning by ListenerService.isRunning.collectAsStateWithLifecycle()
    val askNotifications = rememberPermissionAsk(
        notificationPermission,
        R.string.perm_notifications_title,
        R.string.perm_notifications,
    ) { }
    LaunchedEffect(Unit) {
        if (!serviceRunning) startListening(context)
        if (notificationPermission.any { !context.hasPermission(it) }) askNotifications.ask()
    }
    var menu by remember { mutableStateOf(false) }
    Page(
        title = stringResource(R.string.app_name),
        onBack = null,
        actions = {
            TextButton(onClick = onAbout) { Text(stringResource(R.string.about), color = nestwatch.text2) }
            TextButton(onClick = { menu = !menu }) { Text(stringResource(R.string.settings), color = nestwatch.text2) }
        },
    ) {
        if (menu) {
            ThemeCard()
            NestwatchCard {
                Text(stringResource(R.string.change_role), style = MaterialTheme.typography.titleMedium, color = nestwatch.text1)
                Caption(stringResource(R.string.change_role_hint))
                SecondaryButton(stringResource(R.string.change_role), onSwitchRole)
            }
        }
        val stations = state?.stations.orEmpty()
        if (stations.isEmpty()) {
            NestwatchCard { Text(stringResource(R.string.no_stations), color = nestwatch.text2) }
        }
        val solo = state?.solo
        stations.forEach { t ->
            StationTileView(
                t,
                solo = solo,
                onSettings = { onStation(t.id) },
                onCamera = { onCamera(t.id) },
            )
        }
        PrimaryButton(stringResource(R.string.add_station), onAdd, Modifier.fillMaxWidth())
        if (stations.isNotEmpty()) Caption(stringResource(R.string.keeps_listening))
        BatteryNotice()
    }
}

/** Status word and pill flavour of a tile (never colour alone). */
@Composable
fun tileStatus(t: StationTile): Pair<String, PillKind> = when (val c = t.connection) {
    is Connection.Live -> when {
        t.talk is TalkState.Granted -> stringResource(R.string.status_you_talk) to PillKind.Voice
        t.talker != null -> stringResource(R.string.status_talking, t.talker) to PillKind.Voice
        t.open && t.prefs.listen.mode == ListenMode.ACTIVATED -> stringResource(R.string.status_sound) to PillKind.Sound
        t.prefs.listen.mode == ListenMode.CONTINUOUS -> stringResource(R.string.status_live) to PillKind.Calm
        else -> stringResource(R.string.status_quiet) to PillKind.Calm
    }
    is Connection.Connecting -> stringResource(R.string.status_connecting) to PillKind.Warn
    is Connection.Reconnecting -> stringResource(R.string.status_reconnecting) to PillKind.Warn
    is Connection.NeedsPairing -> stringResource(R.string.status_needs_pairing) to PillKind.Warn
    is Connection.Idle -> stringResource(R.string.status_paused) to PillKind.Neutral
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun StationTileView(t: StationTile, solo: String?, onSettings: () -> Unit, onCamera: () -> Unit) {
    val context = LocalContext.current
    val (status, kind) = tileStatus(t)
    val live = t.connection is Connection.Live
    val sound = live && t.open && t.prefs.listen.mode == ListenMode.ACTIVATED
    var talkRequested by remember { mutableStateOf(false) }
    val askMic = rememberPermissionAsk(
        listOf(Manifest.permission.RECORD_AUDIO),
        R.string.perm_mic_title,
        R.string.perm_mic_talk,
    ) { }
    NestwatchCard(alert = sound) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                t.name.ifBlank { stringResource(R.string.unnamed) },
                Modifier.weight(1f),
                style = MaterialTheme.typography.titleMedium,
                color = nestwatch.text1,
            )
            Pill(status, kind)
        }
        // Level meter with this listener's threshold mark.
        val levelText = t.levelDbfs?.let {
            stringResource(if (live) R.string.level_now else R.string.level_last, "${it.roundToInt()} dB")
        }
            ?: stringResource(R.string.no_level)
        val thr = t.prefs.listen.thresholdDbfs
        val meterLabel = stringResource(R.string.ring_label)
        Column(Modifier.semantics(mergeDescendants = true) { contentDescription = "$meterLabel: $levelText" }) {
            LevelMeter(if (live) t.levelDbfs else null, if (t.prefs.listen.mode == ListenMode.ACTIVATED) thr else null, sound)
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Caption(levelText)
            Caption(
                if (t.prefs.listen.mode == ListenMode.ACTIVATED) stringResource(R.string.opens_above, "${thr.roundToInt()} dB")
                else stringResource(R.string.mode_always_on),
            )
            t.battery?.let {
                Caption(
                    if (t.charging == true) stringResource(R.string.battery_charging, it.toInt())
                    else stringResource(R.string.battery, it.toInt()),
                    color = if (it.toInt() < 15 && t.charging != true) nestwatch.amber else nestwatch.text3,
                )
            }
            t.path?.let { Caption(stringResource(pathLabel(it))) }
            if (t.cameraOn) Caption(stringResource(R.string.camera_on), color = nestwatch.honey)
        }
        when (val c = t.connection) {
            is Connection.Reconnecting -> Caption(
                stringResource(R.string.attempt, c.attempt.toInt()) + " · " +
                    if (c.tried.isEmpty()) stringResource(R.string.trying)
                    else pluralStringResource(R.plurals.trying_addresses, c.tried.size, c.tried.size),
                color = nestwatch.amber,
            )
            is Connection.Connecting -> Caption(
                if (c.tried.isEmpty()) stringResource(R.string.trying)
                else pluralStringResource(R.plurals.trying_addresses, c.tried.size, c.tried.size),
            )
            is Connection.NeedsPairing -> Caption(stringResource(R.string.needs_pairing_hint), color = nestwatch.amber)
            is Connection.Idle -> {
                Caption(stringResource(R.string.paused_hint))
                SecondaryButton(stringResource(R.string.resume), { Nestwatch.withListener { it.follow(t.id) } })
            }
            else -> {}
        }
        if (live && t.canTalk) {
            val talkLabel = when (val tk = t.talk) {
                is TalkState.Granted -> stringResource(R.string.talking)
                is TalkState.Requested -> stringResource(R.string.talk_wait)
                is TalkState.Busy -> tk.by?.let { stringResource(R.string.talk_busy, it) } ?: stringResource(R.string.talk_busy_someone)
                is TalkState.Disabled -> stringResource(R.string.talk_disabled)
                is TalkState.Idle -> stringResource(R.string.hold_to_talk)
            }
            val enabled = t.talk !is TalkState.Disabled && t.talk !is TalkState.Busy
            HoldToTalk(
                active = t.talk is TalkState.Granted || talkRequested,
                enabled = enabled,
                label = talkLabel,
                onTalk = { on ->
                    if (on && !context.hasPermission(Manifest.permission.RECORD_AUDIO)) {
                        askMic.ask()
                        return@HoldToTalk
                    }
                    talkRequested = on
                    Nestwatch.withListener { it.talk(t.id, on) }
                },
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            val muted = t.prefs.muted
            SecondaryButton(
                stringResource(if (muted) R.string.unmute else R.string.mute),
                { Nestwatch.withListener { it.setMuted(t.id, !muted) } },
                Modifier.weight(1f),
            )
            val isSolo = solo == t.id
            SecondaryButton(
                stringResource(if (isSolo) R.string.unsolo else R.string.solo),
                { Nestwatch.withListener { it.setSolo(if (isSolo) null else t.id) } },
                Modifier.weight(1f),
            )
            if (live && t.canVideo) {
                SecondaryButton(stringResource(R.string.camera), onCamera, Modifier.weight(1f))
            }
        }
        TextButton(onClick = onSettings) {
            Text(stringResource(R.string.station_settings_of, t.name), color = nestwatch.lavender)
        }
    }
}
