// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import org.lull.android.R
import org.lull.android.ui.theme.lull

/** A screen with a title row, scrolling content and safe-area padding. */
@Composable
fun Page(
    title: String,
    onBack: (() -> Unit)?,
    actions: @Composable RowScope.() -> Unit = {},
    content: @Composable () -> Unit,
) {
    Column(
        Modifier
            .fillMaxSize()
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .imePadding(),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .padding(horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (onBack != null) {
                TextButton(onClick = onBack) { Text("‹ " + stringResource(R.string.back), color = lull.text2) }
            }
            Text(
                title,
                style = MaterialTheme.typography.titleLarge,
                color = lull.text1,
                modifier = Modifier
                    .weight(1f)
                    .padding(horizontal = 8.dp)
                    .semantics { heading() },
            )
            actions()
        }
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) { content() }
    }
}

/** A labelled switch row (≥ 48 dp target). */
@Composable
fun SwitchRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit, enabled: Boolean = true) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f), color = lull.text1)
        Switch(
            checked = checked,
            onCheckedChange = onChange,
            enabled = enabled,
            colors = SwitchDefaults.colors(
                checkedTrackColor = lull.lavender,
                checkedThumbColor = lull.onAccent,
                uncheckedTrackColor = lull.ink2,
                uncheckedBorderColor = lull.ink3,
            ),
        )
    }
}

/** A segmented control (`.segmented`). */
@Composable
fun <T> Segmented(options: List<Pair<T, String>>, selected: T, onSelect: (T) -> Unit, modifier: Modifier = Modifier) {
    SingleChoiceSegmentedButtonRow(modifier.fillMaxWidth()) {
        options.forEachIndexed { i, (value, label) ->
            SegmentedButton(
                selected = value == selected,
                onClick = { onSelect(value) },
                shape = SegmentedButtonDefaults.itemShape(i, options.size),
                colors = SegmentedButtonDefaults.colors(
                    activeContainerColor = lull.lavenderSoft,
                    activeContentColor = lull.text1,
                    inactiveContainerColor = lull.ink2,
                    inactiveContentColor = lull.text2,
                    activeBorderColor = lull.lavender,
                    inactiveBorderColor = lull.ink3,
                ),
            ) { Text(label, maxLines = 1) }
        }
    }
}

/** "just now", "5 min ago", ... */
@Composable
fun ago(unixMs: Long, nowMs: Long = System.currentTimeMillis()): String {
    val mins = ((nowMs - unixMs).coerceAtLeast(0)) / 60_000
    return when {
        mins < 1 -> stringResource(R.string.ago_now)
        mins < 60 -> stringResource(R.string.ago_minutes, mins.toInt())
        mins < 24 * 60 -> stringResource(R.string.ago_hours, (mins / 60).toInt())
        mins < 48 * 60 -> stringResource(R.string.ago_yesterday)
        else -> (mins / (24 * 60)).toInt().let { pluralStringResource(R.plurals.ago_days, it, it) }
    }
}

/** `m:ss` for countdowns. */
fun clock(ms: Long): String {
    val s = (ms / 1000).coerceAtLeast(0)
    return "%d:%02d".format(s / 60, s % 60)
}
