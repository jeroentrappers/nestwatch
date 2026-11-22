// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import kotlinx.coroutines.flow.MutableStateFlow
import org.nestwatch.android.platform.AlertActionReceiver
import org.nestwatch.android.platform.AlertActions
import org.nestwatch.android.ui.NestwatchRoot

/** The single activity; everything else is Compose (design/README.md §5). */
class MainActivity : ComponentActivity() {
    /** An invitation opened from a `nestwatch://invite` link (FR-PAIR-1). */
    val pendingInvite = MutableStateFlow<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        takeInvite(intent)
        if (savedInstanceState == null) takeAcknowledge(intent)
        setContent { NestwatchRoot(this) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        takeInvite(intent)
        takeAcknowledge(intent)
    }

    /**
     * Tapping a sound alert or alarm notification acknowledges that
     * station (FR-AL-6). Opening the app any other way does not.
     */
    private fun takeAcknowledge(intent: Intent?) {
        if (intent?.action != AlertActionReceiver.ACTION_ACKNOWLEDGE) return
        val id = intent.getStringExtra(AlertActionReceiver.EXTRA_STATION) ?: return
        AlertActions.acknowledge(this, id)
        // Handled once: a later recreation must not acknowledge again.
        intent.action = null
        intent.removeExtra(AlertActionReceiver.EXTRA_STATION)
    }

    private fun takeInvite(intent: Intent?) {
        val data = intent?.dataString ?: return
        if (intent.action == Intent.ACTION_VIEW && data.startsWith("nestwatch://", ignoreCase = true)) {
            pendingInvite.value = data
        }
    }

    /**
     * Station night mode (FR-ST-4): lowest backlight and screen kept on
     * while dimmed; normal otherwise.
     */
    fun setDimmed(dimmed: Boolean) {
        val lp = window.attributes
        lp.screenBrightness = if (dimmed) 0.01f else WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE
        window.attributes = lp
        if (dimmed) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }
}
