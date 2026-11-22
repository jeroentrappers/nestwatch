// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.material3.AlertDialog
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
import androidx.compose.ui.platform.LocalLocale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.time.ZoneId
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.nestwatch.android.R
import org.nestwatch.android.core.LogEntry
import org.nestwatch.android.core.Nestwatch
import org.nestwatch.android.core.groupNights
import org.nestwatch.android.core.toLogEntry
import org.nestwatch.android.ui.components.Caption
import org.nestwatch.android.ui.components.DangerButton
import org.nestwatch.android.ui.components.NestwatchCard
import org.nestwatch.android.ui.components.Pill
import org.nestwatch.android.ui.components.PillKind
import org.nestwatch.android.ui.formatDuration
import org.nestwatch.android.ui.formatNightDate
import org.nestwatch.android.ui.formatTime
import org.nestwatch.android.ui.rememberNow
import org.nestwatch.android.ui.theme.nestwatch
import org.nestwatch.ffi.NestwatchException

/** A station's night log on this listener (product.md FR-SL-4). */
@Composable
fun NightLogScreen(id: String, onBack: () -> Unit) {
    val state by Nestwatch.listenerState.collectAsStateWithLifecycle()
    val t = state?.stations?.firstOrNull { it.id == id }
    val name = t?.name?.ifBlank { null } ?: stringResource(R.string.unnamed)
    val now = rememberNow(60_000)
    var entries by remember(id) { mutableStateOf<List<LogEntry>?>(null) }
    val ongoing = entries?.any { it.endMs == null } == true
    // Re-read on every log change, and each minute while a wake-up goes on.
    LaunchedEffect(id, t?.logRevision, if (ongoing) now else 0L) {
        entries = withContext(Dispatchers.Default) {
            val l = Nestwatch.listenerEngine ?: return@withContext emptyList()
            try {
                l.nightLog(id).map { it.toLogEntry() }
            } catch (_: NestwatchException) {
                emptyList()
            }
        }
    }
    var clearing by remember { mutableStateOf(false) }
    val locale = LocalLocale.current.platformLocale
    val zone = ZoneId.systemDefault()

    Page(stringResource(R.string.night_log_of, name), onBack) {
        val list = entries
        if (list != null && list.isEmpty()) {
            NestwatchCard { Text(stringResource(R.string.night_empty), color = nestwatch.text2) }
        }
        if (!list.isNullOrEmpty()) {
            groupNights(list, now, zone).forEach { night ->
                NestwatchCard {
                    Text(
                        stringResource(R.string.night_of, formatNightDate(locale, night.date, zone)),
                        style = MaterialTheme.typography.titleMedium,
                        color = nestwatch.text1,
                        modifier = Modifier.semantics { heading() },
                    )
                    val count = pluralStringResource(R.plurals.night_wake_ups, night.entries.size, night.entries.size)
                    val quiet = night.longestQuietMs?.let {
                        stringResource(R.string.night_longest_quiet, formatDuration(LocalResources.current, it))
                    }
                    Caption(listOfNotNull(count, quiet).joinToString(" · "), color = nestwatch.text2)
                    night.entries.forEach { e -> EntryRow(e, now) }
                }
            }
        }
        Caption(stringResource(R.string.night_footer))
        if (!list.isNullOrEmpty()) DangerButton(stringResource(R.string.night_clear), { clearing = true })
    }
    if (clearing) {
        AlertDialog(
            onDismissRequest = { clearing = false },
            text = { Text(stringResource(R.string.night_clear_confirm, name)) },
            confirmButton = {
                TextButton(onClick = {
                    clearing = false
                    Nestwatch.withListener { it.clearNightLog(id) }
                    entries = emptyList()
                }) { Text(stringResource(R.string.night_clear_yes), color = nestwatch.coral) }
            },
            dismissButton = { TextButton(onClick = { clearing = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun EntryRow(e: LogEntry, nowMs: Long) {
    val context = LocalContext.current
    val muted = e.notTheBaby
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
        itemVerticalAlignment = Alignment.CenterVertically,
    ) {
        var text = stringResource(
            R.string.night_entry,
            formatTime(context, e.startMs),
            formatDuration(LocalResources.current, e.durationMs(nowMs)),
        )
        if (e.endMs == null) text += " · " + stringResource(R.string.night_going_on)
        Text(
            text,
            color = if (muted) nestwatch.text3 else nestwatch.text1,
            style = MaterialTheme.typography.bodyLarge,
        )
        if (e.alarmed) Pill(stringResource(R.string.night_badge_alarm), PillKind.Sound, pulse = false)
        if (e.notTheBaby) Pill(stringResource(R.string.night_badge_not_baby), PillKind.Neutral)
    }
}
