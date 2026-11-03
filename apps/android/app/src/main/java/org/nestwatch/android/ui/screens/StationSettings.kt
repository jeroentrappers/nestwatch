// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.nestwatch.android.R
import org.nestwatch.android.core.Nestwatch
import org.nestwatch.android.core.StationRun
import org.nestwatch.android.core.ThemeChoice
import org.nestwatch.android.ui.components.Caption
import org.nestwatch.android.ui.components.DangerButton
import org.nestwatch.android.ui.components.NestwatchCard
import org.nestwatch.android.ui.components.SecondaryButton
import org.nestwatch.android.ui.theme.MonoCode
import org.nestwatch.android.ui.theme.nestwatch
import org.nestwatch.ffi.CameraAvailability
import org.nestwatch.ffi.Gain
import org.nestwatch.ffi.TrustedListenerInfo
import kotlin.math.roundToInt

/** Station settings (FR-ST-7, FR-ST-8, FR-TB-3, FR-AU-5) and app settings. */
@Composable
fun StationSettingsScreen(onBack: () -> Unit, onSwitchRole: () -> Unit) {
    val run by Nestwatch.station.collectAsStateWithLifecycle()
    val s = (run as? StationRun.Running)?.snapshot
    Page(stringResource(R.string.station_settings), onBack) {
        if (s != null) {
            NestwatchCard {
                var name by remember(s.name) { mutableStateOf(s.name) }
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it.take(64) },
                    label = { Text(stringResource(R.string.station_name)) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { Nestwatch.withStation { it.rename(name) } }),
                )
                if (name.trim() != s.name && name.isNotBlank()) {
                    SecondaryButton(stringResource(R.string.save), { Nestwatch.withStation { it.rename(name) } })
                }
                SwitchRow(stringResource(R.string.camera_allowed), s.settings.cameraEnabled, { on ->
                    Nestwatch.withStation { it.setCameraEnabled(on) }
                })
                when (s.cameraAvailability) {
                    CameraAvailability.UNAVAILABLE -> Caption(stringResource(R.string.camera_unavailable_here))
                    CameraAvailability.DENIED -> Caption(stringResource(R.string.camera_denied_here), color = nestwatch.amber)
                    CameraAvailability.AVAILABLE -> {}
                }
                SwitchRow(stringResource(R.string.talk_allowed), s.settings.talkEnabled, { on ->
                    Nestwatch.withStation { it.setTalkEnabled(on) }
                })
                Text(stringResource(R.string.talkback_volume), color = nestwatch.text1)
                var vol by remember(s.settings.talkbackVolume) { mutableStateOf(s.settings.talkbackVolume) }
                Slider(
                    value = vol,
                    onValueChange = { vol = it },
                    onValueChangeFinished = { Nestwatch.withStation { it.setTalkbackVolume(vol) } },
                    colors = SliderDefaults.colors(thumbColor = nestwatch.lavender, activeTrackColor = nestwatch.lavender),
                )
                Caption(stringResource(R.string.percent, (vol * 100).roundToInt()))
                Text(stringResource(R.string.mic_sensitivity), color = nestwatch.text1)
                Segmented(
                    listOf(
                        Gain.AUTO to stringResource(R.string.gain_auto),
                        Gain.DB0 to "0 dB",
                        Gain.DB6 to "+6",
                        Gain.DB12 to "+12",
                        Gain.DB18 to "+18",
                    ),
                    s.settings.gain,
                    { g -> Nestwatch.withStation { it.setGain(g) } },
                )
                s.battery?.let { Caption(stringResource(R.string.battery_here, it.toInt())) }
            }
            TrustedListeners(s.deviceId)
            if (s.endpoints.isNotEmpty()) {
                NestwatchCard {
                    Text(stringResource(R.string.addresses), style = MaterialTheme.typography.titleMedium, color = nestwatch.text1)
                    s.endpoints.forEach { Text(it, style = MonoCode.copy(letterSpacing = MonoCode.letterSpacing * 0.3f), color = nestwatch.text2) }
                }
            }
        }
        ThemeCard()
        NestwatchCard {
            Text(stringResource(R.string.change_role), style = MaterialTheme.typography.titleMedium, color = nestwatch.text1)
            Caption(stringResource(R.string.change_role_hint))
            DangerButton(stringResource(R.string.change_role), onSwitchRole)
        }
    }
}

@Composable
private fun TrustedListeners(key: String) {
    val run by Nestwatch.station.collectAsStateWithLifecycle()
    // Re-read the trust list whenever the station state changes.
    val trusted: List<TrustedListenerInfo> = remember(run, key) {
        Nestwatch.stationEngine?.trustedListeners() ?: emptyList()
    }
    var confirm by remember { mutableStateOf<TrustedListenerInfo?>(null) }
    NestwatchCard {
        Text(stringResource(R.string.trusted_listeners), style = MaterialTheme.typography.titleMedium, color = nestwatch.text1)
        if (trusted.isEmpty()) Caption(stringResource(R.string.no_trusted))
        trusted.forEach { t ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(t.name.ifBlank { stringResource(R.string.unnamed) }, color = nestwatch.text1)
                    Caption(
                        stringResource(
                            R.string.paired_seen,
                            ago(t.pairedUnixMs.toLong()),
                            ago(t.lastSeenUnixMs.toLong()),
                        ),
                    )
                }
                TextButton(onClick = { confirm = t }) { Text(stringResource(R.string.revoke), color = nestwatch.coral) }
            }
        }
    }
    confirm?.let { t ->
        AlertDialog(
            onDismissRequest = { confirm = null },
            text = { Text(stringResource(R.string.revoke_confirm, t.name)) },
            confirmButton = {
                TextButton(onClick = {
                    Nestwatch.withStation { it.revoke(t.id) }
                    confirm = null
                }) { Text(stringResource(R.string.revoke), color = nestwatch.coral) }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

/** Theme choice: system, night or day. */
@Composable
fun ThemeCard() {
    val prefs by Nestwatch.prefs.flow.collectAsStateWithLifecycle()
    NestwatchCard {
        Text(stringResource(R.string.theme), style = MaterialTheme.typography.titleMedium, color = nestwatch.text1)
        Segmented(
            listOf(
                ThemeChoice.System to stringResource(R.string.theme_system),
                ThemeChoice.Night to stringResource(R.string.theme_night),
                ThemeChoice.Day to stringResource(R.string.theme_day),
            ),
            prefs.theme,
            { t -> Nestwatch.prefs.update { it.copy(theme = t) } },
        )
    }
}
