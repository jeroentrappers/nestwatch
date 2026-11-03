// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.ui.screens

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.nestwatch.android.R
import org.nestwatch.android.core.Nestwatch
import org.nestwatch.android.core.RelayLook
import org.nestwatch.android.core.relayLabel
import org.nestwatch.android.core.relayLook
import org.nestwatch.android.ui.components.Caption
import org.nestwatch.android.ui.components.Pill
import org.nestwatch.android.ui.components.PillKind
import org.nestwatch.android.ui.components.NestwatchCard
import org.nestwatch.android.ui.components.SecondaryButton
import org.nestwatch.android.ui.theme.nestwatch
import org.nestwatch.ffi.NestwatchException
import org.nestwatch.ffi.RelayStatus
import org.nestwatch.ffi.StationSnapshot

/** The relay registration as a status pill (rendezvous.md FR-RV-1). */
@Composable
fun RelayPill(status: RelayStatus) {
    val look = relayLook(status)
    Pill(
        stringResource(R.string.relay_pill, stringResource(relayLabel(look))),
        when (look) {
            RelayLook.Off -> PillKind.Neutral
            RelayLook.Connecting -> PillKind.Warn
            RelayLook.Online -> PillKind.Calm
            RelayLook.Error -> PillKind.Sound
        },
    )
}

/** What the relay server can and cannot see (rendezvous.md §7). */
@Composable
fun RelayExplanation(server: String) {
    Caption(stringResource(R.string.relay_server_is, server.removePrefix("https://")), color = nestwatch.text2)
    Caption(stringResource(R.string.relay_explain), color = nestwatch.text2)
    Caption(stringResource(R.string.relay_can_see))
    Caption(stringResource(R.string.relay_cannot_see))
}

/**
 * Relay settings (FR-RV-1, FR-RV-6): the toggle with the explanation, the
 * server address, the status and "reset relay identity".
 */
@Composable
fun RelayCard(s: StationSnapshot) {
    var server by remember(s.settings.relayServer) { mutableStateOf(s.settings.relayServer) }
    var error by remember { mutableStateOf(false) }
    var confirmReset by remember { mutableStateOf(false) }
    val save = {
        val st = Nestwatch.stationEngine
        error = try {
            st?.setRelayServer(server.trim())
            false
        } catch (_: NestwatchException) {
            true
        }
    }
    NestwatchCard {
        SwitchRow(stringResource(R.string.relay_title), s.settings.relayEnabled, { on ->
            Nestwatch.withStation { it.setRelayEnabled(on) }
        })
        RelayExplanation(s.settings.relayServer)
        RelayPill(s.relay)
        (s.relay as? RelayStatus.Error)?.let { Caption(it.message, color = nestwatch.amber) }
        OutlinedTextField(
            value = server,
            onValueChange = {
                server = it.take(255)
                error = false
            },
            label = { Text(stringResource(R.string.relay_server)) },
            supportingText = {
                Text(stringResource(if (error) R.string.relay_server_invalid else R.string.relay_server_hint))
            },
            isError = error,
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { save() }),
        )
        if (server.trim() != s.settings.relayServer && server.isNotBlank()) {
            SecondaryButton(stringResource(R.string.save), { save() })
        }
        TextButton(onClick = { confirmReset = true }) {
            Text(stringResource(R.string.relay_reset), color = nestwatch.coral)
        }
    }
    if (confirmReset) {
        AlertDialog(
            onDismissRequest = { confirmReset = false },
            text = { Text(stringResource(R.string.relay_reset_confirm)) },
            confirmButton = {
                TextButton(onClick = {
                    Nestwatch.withStation { it.resetRelayIdentity() }
                    confirmReset = false
                }) { Text(stringResource(R.string.relay_reset_do), color = nestwatch.coral) }
            },
            dismissButton = { TextButton(onClick = { confirmReset = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

/** Asked once when the station first runs (FR-RV-1): off unless the user opts in. */
@Composable
fun RelaySetupPrompt(s: StationSnapshot) {
    val prefs by Nestwatch.prefs.flow.collectAsStateWithLifecycle()
    if (prefs.relayAsked) return
    val done = { on: Boolean ->
        if (on) Nestwatch.withStation { it.setRelayEnabled(true) }
        Nestwatch.prefs.update { it.copy(relayAsked = true) }
    }
    AlertDialog(
        onDismissRequest = { done(false) },
        title = { Text(stringResource(R.string.relay_setup_title), style = MaterialTheme.typography.titleMedium) },
        text = {
            Column {
                RelayExplanation(s.settings.relayServer)
                Caption(stringResource(R.string.relay_setup_later))
            }
        },
        confirmButton = { TextButton(onClick = { done(true) }) { Text(stringResource(R.string.relay_setup_yes)) } },
        dismissButton = { TextButton(onClick = { done(false) }) { Text(stringResource(R.string.relay_setup_no)) } },
    )
}
