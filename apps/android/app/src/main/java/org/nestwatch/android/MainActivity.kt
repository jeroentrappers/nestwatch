// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import kotlinx.coroutines.flow.MutableStateFlow
import org.nestwatch.android.ui.NestwatchRoot

/** The single activity; everything else is Compose (design/README.md §5). */
class MainActivity : ComponentActivity() {
    /** An invitation opened from a `nestwatch://invite` link (FR-PAIR-1). */
    val pendingInvite = MutableStateFlow<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        takeInvite(intent)
        setContent { NestwatchRoot(this) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        takeInvite(intent)
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
