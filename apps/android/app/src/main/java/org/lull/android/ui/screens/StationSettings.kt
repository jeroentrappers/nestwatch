// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.screens

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
import org.lull.android.R
import org.lull.android.core.Lull
import org.lull.android.core.StationRun
import org.lull.android.core.ThemeChoice
import org.lull.android.ui.components.Caption
import org.lull.android.ui.components.DangerButton
import org.lull.android.ui.components.LullCard
import org.lull.android.ui.components.SecondaryButton
import org.lull.android.ui.theme.MonoCode
import org.lull.android.ui.theme.lull
import org.lull.ffi.CameraAvailability
import org.lull.ffi.Gain
import org.lull.ffi.TrustedListenerInfo
import kotlin.math.roundToInt

/** Station settings (FR-ST-7, FR-ST-8, FR-TB-3, FR-AU-5) and app settings. */
@Composable
fun StationSettingsScreen(onBack: () -> Unit, onSwitchRole: () -> Unit) {
    val run by Lull.station.collectAsStateWithLifecycle()
    val s = (run as? StationRun.Running)?.snapshot
    Page(stringResource(R.string.station_settings), onBack) {
        if (s != null) {
            LullCard {
                var name by remember(s.name) { mutableStateOf(s.name) }
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it.take(64) },
                    label = { Text(stringResource(R.string.station_name)) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { Lull.withStation { it.rename(name) } }),
                )
                if (name.trim() != s.name && name.isNotBlank()) {
                    SecondaryButton(stringResource(R.string.save), { Lull.withStation { it.rename(name) } })
                }
                SwitchRow(stringResource(R.string.camera_allowed), s.settings.cameraEnabled, { on ->
                    Lull.withStation { it.setCameraEnabled(on) }
                })
                when (s.cameraAvailability) {
                    CameraAvailability.UNAVAILABLE -> Caption(stringResource(R.string.camera_unavailable_here))
                    CameraAvailability.DENIED -> Caption(stringResource(R.string.camera_denied_here), color = lull.amber)
                    CameraAvailability.AVAILABLE -> {}
                }
                SwitchRow(stringResource(R.string.talk_allowed), s.settings.talkEnabled, { on ->
                    Lull.withStation { it.setTalkEnabled(on) }
                })
                Text(stringResource(R.string.talkback_volume), color = lull.text1)
                var vol by remember(s.settings.talkbackVolume) { mutableStateOf(s.settings.talkbackVolume) }
                Slider(
                    value = vol,
                    onValueChange = { vol = it },
                    onValueChangeFinished = { Lull.withStation { it.setTalkbackVolume(vol) } },
                    colors = SliderDefaults.colors(thumbColor = lull.lavender, activeTrackColor = lull.lavender),
                )
                Caption(stringResource(R.string.percent, (vol * 100).roundToInt()))
                Text(stringResource(R.string.mic_sensitivity), color = lull.text1)
                Segmented(
                    listOf(
                        Gain.AUTO to stringResource(R.string.gain_auto),
                        Gain.DB0 to "0 dB",
                        Gain.DB6 to "+6",
                        Gain.DB12 to "+12",
                        Gain.DB18 to "+18",
                    ),
                    s.settings.gain,
                    { g -> Lull.withStation { it.setGain(g) } },
                )
                s.battery?.let { Caption(stringResource(R.string.battery_here, it.toInt())) }
            }
            TrustedListeners(s.deviceId)
            if (s.endpoints.isNotEmpty()) {
                LullCard {
                    Text(stringResource(R.string.addresses), style = MaterialTheme.typography.titleMedium, color = lull.text1)
                    s.endpoints.forEach { Text(it, style = MonoCode.copy(letterSpacing = MonoCode.letterSpacing * 0.3f), color = lull.text2) }
                }
            }
        }
        ThemeCard()
        LullCard {
            Text(stringResource(R.string.change_role), style = MaterialTheme.typography.titleMedium, color = lull.text1)
            Caption(stringResource(R.string.change_role_hint))
            DangerButton(stringResource(R.string.change_role), onSwitchRole)
        }
    }
}

@Composable
private fun TrustedListeners(key: String) {
    val run by Lull.station.collectAsStateWithLifecycle()
    // Re-read the trust list whenever the station state changes.
    val trusted: List<TrustedListenerInfo> = remember(run, key) {
        Lull.stationEngine?.trustedListeners() ?: emptyList()
    }
    var confirm by remember { mutableStateOf<TrustedListenerInfo?>(null) }
    LullCard {
        Text(stringResource(R.string.trusted_listeners), style = MaterialTheme.typography.titleMedium, color = lull.text1)
        if (trusted.isEmpty()) Caption(stringResource(R.string.no_trusted))
        trusted.forEach { t ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(t.name.ifBlank { stringResource(R.string.unnamed) }, color = lull.text1)
                    Caption(
                        stringResource(
                            R.string.paired_seen,
                            ago(t.pairedUnixMs.toLong()),
                            ago(t.lastSeenUnixMs.toLong()),
                        ),
                    )
                }
                TextButton(onClick = { confirm = t }) { Text(stringResource(R.string.revoke), color = lull.coral) }
            }
        }
    }
    confirm?.let { t ->
        AlertDialog(
            onDismissRequest = { confirm = null },
            text = { Text(stringResource(R.string.revoke_confirm, t.name)) },
            confirmButton = {
                TextButton(onClick = {
                    Lull.withStation { it.revoke(t.id) }
                    confirm = null
                }) { Text(stringResource(R.string.revoke), color = lull.coral) }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

/** Theme choice: system, night or day. */
@Composable
fun ThemeCard() {
    val prefs by Lull.prefs.flow.collectAsStateWithLifecycle()
    LullCard {
        Text(stringResource(R.string.theme), style = MaterialTheme.typography.titleMedium, color = lull.text1)
        Segmented(
            listOf(
                ThemeChoice.System to stringResource(R.string.theme_system),
                ThemeChoice.Night to stringResource(R.string.theme_night),
                ThemeChoice.Day to stringResource(R.string.theme_day),
            ),
            prefs.theme,
            { t -> Lull.prefs.update { it.copy(theme = t) } },
        )
    }
}
