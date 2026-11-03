// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.screens

import android.Manifest
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import org.lull.android.R
import org.lull.android.core.Lull
import org.lull.android.ui.errorText
import org.lull.android.ui.rememberPermissionAsk
import org.lull.android.ui.components.Caption
import org.lull.android.ui.components.CodeBox
import org.lull.android.ui.components.LullCard
import org.lull.android.ui.components.Pill
import org.lull.android.ui.components.PillKind
import org.lull.android.ui.components.PrimaryButton
import org.lull.android.ui.theme.lull
import org.lull.ffi.LullException
import org.lull.ffi.NearbyStation
import org.lull.ffi.PairResult
import org.lull.ffi.isValidRoute
import org.lull.ffi.parseDeviceId
import org.lull.ffi.parseInvitation

/** Add a station (README §5.5): QR scan, nearby, code, link, manual address. */
@Composable
fun AddStationScreen(initialInvite: String?, onDone: () -> Unit) {
    val state by Lull.listenerState.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    var scanning by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<Throwable?>(null) }
    var errorIsInvite by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<PairResult?>(null) }
    var removed by remember { mutableStateOf(false) }
    var invite by remember { mutableStateOf(initialInvite ?: "") }
    var code by remember { mutableStateOf("") }
    var stationId by remember { mutableStateOf("") }
    var chosen by remember { mutableStateOf<NearbyStation?>(null) }
    var address by remember { mutableStateOf("") }
    var mode by remember { mutableStateOf(if (initialInvite != null) 1 else 0) }

    val askCamera = rememberPermissionAsk(
        listOf(Manifest.permission.CAMERA),
        R.string.perm_camera_title,
        R.string.perm_camera_scan,
    ) { ok -> scanning = ok }

    fun routes(): List<String> = buildList {
        address.trim().takeIf { it.isNotEmpty() }?.let { add(it) }
    }

    fun pairInvite(text: String) {
        val l = Lull.listener() ?: return
        busy = true
        error = null
        scope.launch {
            try {
                result = l.pairInvitation(text.trim(), routes())
            } catch (e: LullException) {
                error = e
                errorIsInvite = true
            } finally {
                busy = false
            }
        }
    }

    fun pairCode() {
        val l = Lull.listener() ?: return
        busy = true
        error = null
        scope.launch {
            try {
                val id = chosen?.id ?: parseDeviceId(stationId)
                val r = routes().ifEmpty { chosen?.endpoints.orEmpty() }
                result = l.pairCode(code.trim().uppercase(), id, r)
            } catch (e: LullException) {
                error = e
                errorIsInvite = false
            } finally {
                busy = false
            }
        }
    }

    LaunchedEffect(initialInvite) { if (!initialInvite.isNullOrBlank()) pairInvite(initialInvite) }

    if (scanning) {
        QrScanner(
            onResult = { text ->
                scanning = false
                invite = text
                mode = 1
                pairInvite(text)
            },
            onCancel = { scanning = false },
        )
        return
    }

    Page(stringResource(R.string.add_station), onDone) {
        LullCard {
            PrimaryButton(stringResource(R.string.scan_qr), { askCamera.ask() }, Modifier.fillMaxWidth())
            Caption(stringResource(R.string.scan_hint))
        }
        // Nearby stations found over NSD.
        LullCard {
            Text(stringResource(R.string.nearby), style = MaterialTheme.typography.titleMedium, color = lull.text1)
            val nearby = state?.nearby.orEmpty()
            if (nearby.isEmpty()) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    CircularProgressIndicator(Modifier.heightIn(max = 16.dp), strokeWidth = 2.dp, color = lull.text3)
                    Caption(stringResource(R.string.nearby_searching))
                }
            }
            nearby.forEach { n ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = 48.dp)
                        .clickable(enabled = n.pairing && !n.known, role = Role.Button) {
                            chosen = n
                            stationId = n.id
                            mode = 0
                        },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(n.name, Modifier.weight(1f), color = lull.text1)
                    when {
                        n.known -> Pill(stringResource(R.string.already_paired), PillKind.Neutral)
                        n.pairing -> Pill(stringResource(R.string.pairing_open), PillKind.Calm)
                        else -> Pill(stringResource(R.string.not_pairing), PillKind.Neutral)
                    }
                }
            }
        }
        LullCard {
            Segmented(
                listOf(0 to stringResource(R.string.by_code), 1 to stringResource(R.string.by_link)),
                mode,
                { mode = it },
            )
            if (mode == 0) {
                OutlinedTextField(
                    value = code,
                    onValueChange = { code = it.take(16) },
                    label = {
                        Text(chosen?.let { stringResource(R.string.code_field_for, it.name) } ?: stringResource(R.string.code_field))
                    },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(
                        capitalization = KeyboardCapitalization.Characters,
                        keyboardType = KeyboardType.Ascii,
                        autoCorrectEnabled = false,
                    ),
                    textStyle = org.lull.android.ui.theme.MonoCode,
                    modifier = Modifier.fillMaxWidth(),
                )
                if (chosen == null) {
                    OutlinedTextField(
                        value = stationId,
                        onValueChange = { stationId = it.take(24) },
                        label = { Text(stringResource(R.string.station_id_field)) },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii, autoCorrectEnabled = false),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            } else {
                OutlinedTextField(
                    value = invite,
                    onValueChange = { invite = it },
                    label = { Text(stringResource(R.string.invite_field)) },
                    supportingText = { Text(stringResource(R.string.invite_hint)) },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            val addrValid = address.isBlank() || isValidRoute(address.trim())
            OutlinedTextField(
                value = address,
                onValueChange = { address = it },
                label = { Text(stringResource(R.string.address_field)) },
                supportingText = {
                    Text(if (addrValid) stringResource(R.string.address_hint) else stringResource(R.string.err_invalid_address))
                },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
                modifier = Modifier.fillMaxWidth(),
            )
            if (busy) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    CircularProgressIndicator(Modifier.heightIn(max = 20.dp), strokeWidth = 2.dp, color = lull.honey)
                    Text(stringResource(R.string.pairing), color = lull.text2)
                }
            } else if (mode == 0) {
                PrimaryButton(
                    stringResource(R.string.code_submit),
                    { pairCode() },
                    Modifier.fillMaxWidth(),
                    enabled = code.isNotBlank() && (chosen != null || stationId.isNotBlank()) && addrValid,
                )
            } else {
                PrimaryButton(
                    stringResource(R.string.invite_submit),
                    {
                        if (runCatching { parseInvitation(invite.trim()) }.isFailure) {
                            error = LullException.Invalid("invite")
                            errorIsInvite = true
                        } else {
                            pairInvite(invite)
                        }
                    },
                    Modifier.fillMaxWidth(),
                    enabled = invite.isNotBlank() && addrValid,
                )
            }
            error?.let {
                Text(
                    errorText(it, if (errorIsInvite) R.string.err_invalid_invite else R.string.err_invalid_code),
                    color = lull.coral,
                )
            }
            if (removed) Text(stringResource(R.string.verify_removed), color = lull.amber)
        }
    }

    // Verification number check (FR-PAIR-5).
    result?.let { r ->
        AlertDialog(
            onDismissRequest = {},
            title = { Text(stringResource(R.string.verify_prompt)) },
            text = {
                Column(
                    Modifier.fillMaxWidth(),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(r.name, color = lull.text1)
                    CodeBox(r.verification)
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    result = null
                    onDone()
                }) { Text(stringResource(R.string.verify_same)) }
            },
            dismissButton = {
                TextButton(onClick = {
                    Lull.withListener { it.forget(r.stationId) }
                    result = null
                    removed = true
                }) { Text(stringResource(R.string.verify_different), color = lull.coral) }
            },
        )
    }
}
