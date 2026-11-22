// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.ui.screens

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.nestwatch.android.R
import org.nestwatch.android.platform.AlertActions
import org.nestwatch.android.ui.components.Caption
import org.nestwatch.android.ui.formatTime
import org.nestwatch.android.ui.roomStatusText
import org.nestwatch.android.ui.theme.nestwatch
import org.nestwatch.ffi.AlarmState
import org.nestwatch.ffi.Connection
import org.nestwatch.ffi.RoomStatus
import org.nestwatch.ffi.StationTile

/**
 * The alert part of a station tile (FR-AL-5..7, FR-SL-3): the banner
 * while an alert waits for an answer or the alarm rings, the paused notice
 * and the room status. Nothing here acknowledges by being shown.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TileAlerts(t: StationTile, name: String, nowMs: Long) {
    val context = LocalContext.current
    val res = LocalResources.current
    val live = t.connection is Connection.Live
    when (t.alarm) {
        is AlarmState.Ringing -> AlertBanner(
            ringing = true,
            title = stringResource(R.string.alarm_banner_title, name),
            body = stringResource(R.string.alarm_banner_body),
            stationId = t.id,
        )
        is AlarmState.Pending -> AlertBanner(
            ringing = false,
            title = stringResource(R.string.pending_banner_title, name),
            body = if (t.prefs.alerts.escalate) stringResource(R.string.pending_banner_body) else null,
            stationId = t.id,
        )
        is AlarmState.Off -> {}
    }
    t.pausedUntilUnixMs?.toLong()?.takeIf { it > nowMs }?.let { until ->
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
            itemVerticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                stringResource(R.string.alerts_paused_until, formatTime(context, until)),
                color = nestwatch.amber,
                style = MaterialTheme.typography.bodyMedium,
            )
            TextButton(onClick = { AlertActions.resume(t.id) }, modifier = Modifier.heightIn(min = 48.dp)) {
                Text(stringResource(R.string.resume_alerts), color = nestwatch.lavender)
            }
        }
    }
    if (!live) return
    val (awake, since) = when (val r = t.room) {
        is RoomStatus.Awake -> true to r.sinceUnixMs.toLong()
        is RoomStatus.Quiet -> false to r.sinceUnixMs.toLong()
        is RoomStatus.Unknown -> return
    }
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
        itemVerticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            roomStatusText(res, awake, nowMs - since),
            color = if (awake) nestwatch.honey else nestwatch.text2,
            style = MaterialTheme.typography.bodyMedium,
        )
        // FR-AL-7: the tile offers "Not the baby" while the room is awake
        // (the banner already offers it while an alert waits).
        if (awake && t.alarm is AlarmState.Off && t.pausedUntilUnixMs == null) {
            TextButton(onClick = { AlertActions.notTheBaby(context, t.id) }, modifier = Modifier.heightIn(min = 48.dp)) {
                Text(stringResource(R.string.action_not_baby), color = nestwatch.lavender)
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AlertBanner(ringing: Boolean, title: String, body: String?, stationId: String) {
    val context = LocalContext.current
    val c = nestwatch
    val shape = RoundedCornerShape(16.dp)
    val fg = if (ringing) c.onAccent else c.text1
    Column(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .then(if (ringing) Modifier.background(c.coral) else Modifier.background(c.coralSoft).border(1.dp, c.coral, shape))
            .padding(12.dp)
            .semantics(mergeDescendants = false) { liveRegion = if (ringing) LiveRegionMode.Assertive else LiveRegionMode.Polite },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Image(
                painterResource(R.drawable.ic_alarm),
                contentDescription = null,
                modifier = Modifier.size(if (ringing) 28.dp else 22.dp).padding(end = 6.dp),
                colorFilter = ColorFilter.tint(if (ringing) fg else c.coral),
            )
            Text(
                title,
                color = fg,
                style = if (ringing) MaterialTheme.typography.titleMedium else MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
            )
        }
        body?.let {
            if (ringing) Text(it, color = fg, style = MaterialTheme.typography.bodyMedium) else Caption(it, color = c.text2)
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = { AlertActions.acknowledge(context, stationId) },
                modifier = Modifier.heightIn(min = 48.dp),
                colors = if (ringing) {
                    ButtonDefaults.buttonColors(containerColor = c.ink1, contentColor = c.text1)
                } else {
                    ButtonDefaults.buttonColors(containerColor = c.coral, contentColor = c.onAccent)
                },
            ) { Text(stringResource(R.string.action_on_it), fontWeight = FontWeight.SemiBold) }
            OutlinedButton(
                onClick = { AlertActions.notTheBaby(context, stationId) },
                modifier = Modifier.heightIn(min = 48.dp),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = fg),
                border = androidx.compose.foundation.BorderStroke(1.dp, if (ringing) fg else c.coral),
            ) { Text(stringResource(R.string.action_not_baby)) }
        }
    }
}
