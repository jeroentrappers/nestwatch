// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.ui.screens

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
import org.nestwatch.android.R
import org.nestwatch.android.core.Nestwatch
import org.nestwatch.android.core.THRESHOLD_MAX
import org.nestwatch.android.core.THRESHOLD_MIN
import org.nestwatch.android.core.clampThreshold
import org.nestwatch.android.ui.components.CalibrationChart
import org.nestwatch.android.ui.components.Caption
import org.nestwatch.android.ui.components.DangerButton
import org.nestwatch.android.ui.components.NestwatchCard
import org.nestwatch.android.ui.components.SecondaryButton
import org.nestwatch.android.ui.theme.nestwatch
import org.nestwatch.ffi.AlertSettings
import org.nestwatch.ffi.Connection
import org.nestwatch.ffi.ListenMode
import org.nestwatch.ffi.ListenSettingsInfo
import org.nestwatch.ffi.isValidRoute
import kotlin.math.roundToInt

private val HOLD_CHOICES = listOf(2_000, 4_000, 8_000, 15_000, 30_000)

/** Station settings on the listener (README §5.7, FR-AU-2, FR-AU-6). */
@Composable
fun ListenerStationSettings(id: String, onBack: () -> Unit) {
    val state by Nestwatch.listenerState.collectAsStateWithLifecycle()
    val tick by Nestwatch.historyVersion.collectAsStateWithLifecycle()
    val t = state?.stations?.firstOrNull { it.id == id }
    if (t == null) {
        Page(stringResource(R.string.settings), onBack) {}
        return
    }
    val listen = t.prefs.listen
    fun setListen(l: ListenSettingsInfo) = Nestwatch.withListener { it.setListen(id, l) }
    var forgetting by remember { mutableStateOf(false) }

    Page(stringResource(R.string.station_settings_of, t.name), onBack) {
        NestwatchCard {
            Text(stringResource(R.string.listen_mode), style = MaterialTheme.typography.titleMedium, color = nestwatch.text1)
            Segmented(
                listOf(
                    ListenMode.ACTIVATED to stringResource(R.string.mode_sound),
                    ListenMode.CONTINUOUS to stringResource(R.string.mode_always),
                ),
                listen.mode,
                { m -> setListen(listen.copy(mode = m)) },
            )
            val points = remember(tick, id) { Nestwatch.history(id) }
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
                Text(stringResource(R.string.threshold), Modifier.weight(1f), color = nestwatch.text1)
                Text(thrText, color = nestwatch.honey)
            }
            Slider(
                value = threshold,
                onValueChange = { threshold = clampThreshold(it) },
                onValueChangeFinished = { setListen(listen.copy(thresholdDbfs = threshold)) },
                valueRange = THRESHOLD_MIN..THRESHOLD_MAX,
                modifier = Modifier.semantics { stateDescription = thrText },
                colors = SliderDefaults.colors(thumbColor = nestwatch.honey, activeTrackColor = nestwatch.honey),
            )
            if (listen.mode == ListenMode.ACTIVATED) {
                Text(stringResource(R.string.hold_time), color = nestwatch.text1)
                Segmented(
                    HOLD_CHOICES.map { ms -> ms.toUInt() to "${ms / 1000} s" },
                    HOLD_CHOICES.minBy { kotlin.math.abs(it - listen.holdMs.toInt()) }.toUInt(),
                    { ms -> setListen(listen.copy(holdMs = ms)) },
                )
                val secs = (listen.holdMs.toInt() / 1000)
                Caption(pluralStringResource(R.plurals.seconds, secs, secs))
            }
        }
        NestwatchCard {
            var vol by remember(t.prefs.volume) { mutableStateOf(t.prefs.volume) }
            val volText = stringResource(R.string.percent, (vol * 100).roundToInt())
            Row {
                Text(stringResource(R.string.volume), Modifier.weight(1f), color = nestwatch.text1)
                Text(volText, color = nestwatch.text2)
            }
            Slider(
                value = vol,
                onValueChange = { vol = it },
                onValueChangeFinished = { Nestwatch.withListener { it.setVolume(id, vol) } },
                modifier = Modifier.semantics { contentDescription = volText },
                colors = SliderDefaults.colors(thumbColor = nestwatch.lavender, activeTrackColor = nestwatch.lavender),
            )
            SwitchRow(stringResource(R.string.mute), t.prefs.muted, { m -> Nestwatch.withListener { it.setMuted(id, m) } })
            val a = t.prefs.alerts
            fun setAlerts(n: AlertSettings) = Nestwatch.withListener { it.setAlerts(id, n) }
            SwitchRow(stringResource(R.string.alert_sound_toggle), a.sound, { setAlerts(a.copy(sound = it)) })
            SwitchRow(stringResource(R.string.alert_unreachable_toggle), a.unreachable, { setAlerts(a.copy(unreachable = it)) })
            SwitchRow(stringResource(R.string.alert_battery_toggle), a.battery, { setAlerts(a.copy(battery = it)) })
        }
        NestwatchCard {
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
                Nestwatch.withListener { it.addRoute(id, address.trim()) }
                address = ""
                added = true
            }, enabled = valid)
            if (added) Caption(stringResource(R.string.address_added), color = nestwatch.sage)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (t.prefs.follow) {
                SecondaryButton(stringResource(R.string.pause), { Nestwatch.withListener { it.unfollow(id) } }, Modifier.weight(1f))
            } else {
                SecondaryButton(stringResource(R.string.resume), { Nestwatch.withListener { it.follow(id) } }, Modifier.weight(1f))
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
                    Nestwatch.withListener { it.forget(id) }
                    onBack()
                }) { Text(stringResource(R.string.forget_yes), color = nestwatch.coral) }
            },
            dismissButton = { TextButton(onClick = { forgetting = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
