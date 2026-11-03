// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.lull.android.R
import org.lull.android.core.Lull
import org.lull.android.core.THRESHOLD_MAX
import org.lull.android.core.THRESHOLD_MIN
import org.lull.android.core.clampThreshold
import org.lull.android.ui.components.CalibrationChart
import org.lull.android.ui.components.Caption
import org.lull.android.ui.components.DangerButton
import org.lull.android.ui.components.LullCard
import org.lull.android.ui.components.SecondaryButton
import org.lull.android.ui.theme.lull
import org.lull.ffi.AlertSettings
import org.lull.ffi.Connection
import org.lull.ffi.ListenMode
import org.lull.ffi.ListenSettingsInfo
import org.lull.ffi.isValidRoute
import kotlin.math.roundToInt

private val HOLD_CHOICES = listOf(2_000, 4_000, 8_000, 15_000, 30_000)

/** Station settings on the listener (README §5.7, FR-AU-2, FR-AU-6). */
@Composable
fun ListenerStationSettings(id: String, onBack: () -> Unit) {
    val state by Lull.listenerState.collectAsStateWithLifecycle()
    val tick by Lull.historyVersion.collectAsStateWithLifecycle()
    val t = state?.stations?.firstOrNull { it.id == id }
    if (t == null) {
        Page(stringResource(R.string.settings), onBack) {}
        return
    }
    val listen = t.prefs.listen
    fun setListen(l: ListenSettingsInfo) = Lull.withListener { it.setListen(id, l) }
    var forgetting by remember { mutableStateOf(false) }

    Page(stringResource(R.string.station_settings_of, t.name), onBack) {
        LullCard {
            Text(stringResource(R.string.listen_mode), style = MaterialTheme.typography.titleMedium, color = lull.text1)
            Segmented(
                listOf(
                    ListenMode.ACTIVATED to stringResource(R.string.mode_sound),
                    ListenMode.CONTINUOUS to stringResource(R.string.mode_always),
                ),
                listen.mode,
                { m -> setListen(listen.copy(mode = m)) },
            )
            val points = remember(tick, id) { Lull.history(id) }
            var threshold by remember(listen.thresholdDbfs) { mutableStateOf(listen.thresholdDbfs) }
            CalibrationChart(
                points = points,
                thresholdDb = threshold,
                onThreshold = { db ->
                    threshold = clampThreshold(db)
                    setListen(listen.copy(thresholdDbfs = threshold))
                },
                activated = listen.mode == ListenMode.ACTIVATED,
            )
            Caption(
                stringResource(if (listen.mode == ListenMode.ACTIVATED) R.string.calibrate_hint else R.string.calibrate_continuous),
            )
            val thrText = stringResource(R.string.threshold_value, threshold.roundToInt())
            Row {
                Text(stringResource(R.string.threshold), Modifier.weight(1f), color = lull.text1)
                Text(thrText, color = lull.honey)
            }
            Slider(
                value = threshold,
                onValueChange = { threshold = clampThreshold(it) },
                onValueChangeFinished = { setListen(listen.copy(thresholdDbfs = threshold)) },
                valueRange = THRESHOLD_MIN..THRESHOLD_MAX,
                modifier = Modifier.semantics { stateDescription = thrText },
                colors = SliderDefaults.colors(thumbColor = lull.honey, activeTrackColor = lull.honey),
            )
            if (listen.mode == ListenMode.ACTIVATED) {
                Text(stringResource(R.string.hold_time), color = lull.text1)
                Segmented(
                    HOLD_CHOICES.map { ms -> ms.toUInt() to "${ms / 1000} s" },
                    HOLD_CHOICES.minBy { kotlin.math.abs(it - listen.holdMs.toInt()) }.toUInt(),
                    { ms -> setListen(listen.copy(holdMs = ms)) },
                )
                val secs = (listen.holdMs.toInt() / 1000)
                Caption(pluralStringResource(R.plurals.seconds, secs, secs))
            }
        }
        LullCard {
            var vol by remember(t.prefs.volume) { mutableStateOf(t.prefs.volume) }
            val volText = stringResource(R.string.percent, (vol * 100).roundToInt())
            Row {
                Text(stringResource(R.string.volume), Modifier.weight(1f), color = lull.text1)
                Text(volText, color = lull.text2)
            }
            Slider(
                value = vol,
                onValueChange = { vol = it },
                onValueChangeFinished = { Lull.withListener { it.setVolume(id, vol) } },
                modifier = Modifier.semantics { contentDescription = volText },
                colors = SliderDefaults.colors(thumbColor = lull.lavender, activeTrackColor = lull.lavender),
            )
            SwitchRow(stringResource(R.string.mute), t.prefs.muted, { m -> Lull.withListener { it.setMuted(id, m) } })
            val a = t.prefs.alerts
            fun setAlerts(n: AlertSettings) = Lull.withListener { it.setAlerts(id, n) }
            SwitchRow(stringResource(R.string.alert_sound_toggle), a.sound, { setAlerts(a.copy(sound = it)) })
            SwitchRow(stringResource(R.string.alert_unreachable_toggle), a.unreachable, { setAlerts(a.copy(unreachable = it)) })
            SwitchRow(stringResource(R.string.alert_battery_toggle), a.battery, { setAlerts(a.copy(battery = it)) })
        }
        LullCard {
            t.route?.let { if (t.connection is Connection.Live) Caption(stringResource(R.string.connected_via, it)) }
            var address by remember { mutableStateOf("") }
            var added by remember { mutableStateOf(false) }
            val valid = address.isNotBlank() && isValidRoute(address.trim())
            OutlinedTextField(
                value = address,
                onValueChange = { address = it; added = false },
                label = { Text(stringResource(R.string.address_field)) },
                supportingText = {
                    Text(
                        if (address.isNotBlank() && !valid) stringResource(R.string.err_invalid_address)
                        else stringResource(R.string.address_hint),
                    )
                },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            SecondaryButton(stringResource(R.string.add_address), {
                Lull.withListener { it.addRoute(id, address.trim()) }
                address = ""
                added = true
            }, enabled = valid)
            if (added) Caption(stringResource(R.string.address_added), color = lull.sage)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (t.prefs.follow) {
                SecondaryButton(stringResource(R.string.pause), { Lull.withListener { it.unfollow(id) } }, Modifier.weight(1f))
            } else {
                SecondaryButton(stringResource(R.string.resume), { Lull.withListener { it.follow(id) } }, Modifier.weight(1f))
            }
            DangerButton(stringResource(R.string.forget), { forgetting = true }, Modifier.weight(1f))
        }
    }
    if (forgetting) {
        AlertDialog(
            onDismissRequest = { forgetting = false },
            text = { Text(stringResource(R.string.forget_confirm)) },
            confirmButton = {
                TextButton(onClick = {
                    forgetting = false
                    Lull.withListener { it.forget(id) }
                    onBack()
                }) { Text(stringResource(R.string.forget_yes), color = lull.coral) }
            },
            dismissButton = { TextButton(onClick = { forgetting = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
