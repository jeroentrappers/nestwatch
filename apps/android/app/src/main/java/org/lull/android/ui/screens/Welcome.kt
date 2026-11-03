// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.lull.android.BuildConfig
import org.lull.android.R
import org.lull.android.core.Lull
import org.lull.android.core.Role as LullRole
import org.lull.android.ui.components.Caption
import org.lull.android.ui.components.LullCard
import org.lull.android.ui.components.Logo
import org.lull.android.ui.theme.lull
import org.lull.ffi.coreVersion
import org.lull.ffi.groupedId

/** Welcome: name, promise, disclaimer and the role choice (README §5.1, UI-6). */
@Composable
fun WelcomeScreen(onRole: (LullRole) -> Unit) {
    Column(
        Modifier
            .fillMaxSize()
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Spacer(Modifier.heightIn(min = 24.dp))
        Logo(72.dp)
        Text(
            stringResource(R.string.app_name),
            style = MaterialTheme.typography.displaySmall,
            color = lull.text1,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            stringResource(R.string.promise),
            style = MaterialTheme.typography.bodyLarge,
            color = lull.text2,
            textAlign = TextAlign.Center,
        )
        LullCard {
            Text(stringResource(R.string.disclaimer), color = lull.amber, style = MaterialTheme.typography.bodyMedium)
            Caption(stringResource(R.string.disclaimer_ack))
        }
        RoleChoice(stringResource(R.string.role_station), stringResource(R.string.role_station_hint)) {
            onRole(LullRole.Station)
        }
        RoleChoice(stringResource(R.string.role_listener), stringResource(R.string.role_listener_hint)) {
            onRole(LullRole.Listener)
        }
    }
}

@Composable
private fun RoleChoice(title: String, hint: String, onClick: () -> Unit) {
    val shape = RoundedCornerShape(24.dp)
    Column(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(lull.ink1)
            .border(1.dp, lull.ink3, shape)
            .clickable(role = Role.Button, onClick = onClick)
            .heightIn(min = 72.dp)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium, color = lull.honey)
        Text(hint, style = MaterialTheme.typography.bodyMedium, color = lull.text2)
    }
}

/** About: privacy promise, disclaimer, version, device ID, licence. */
@Composable
fun AboutScreen(onBack: () -> Unit) {
    Page(stringResource(R.string.about), onBack) {
        LullCard {
            Logo(48.dp)
            Text(stringResource(R.string.about_body), color = lull.text1)
            Text(stringResource(R.string.disclaimer), color = lull.amber)
        }
        LullCard {
            Caption(stringResource(R.string.version))
            Text("${BuildConfig.VERSION_NAME} · ${coreVersion()}", color = lull.text1)
            val id = Lull.listenerEngine?.deviceId() ?: Lull.stationEngine?.deviceId()
            if (id != null) {
                Caption(stringResource(R.string.device_id))
                Text(groupedId(id), style = org.lull.android.ui.theme.MonoCode, color = lull.text1)
            }
            Caption(stringResource(R.string.license))
        }
    }
}
