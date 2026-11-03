// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.core

import android.content.Context
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Which role this installation plays (design/README.md §5, screen 1). */
enum class Role { None, Station, Listener }

/** Colour theme: follow the system, or force night or day. */
enum class ThemeChoice { System, Night, Day }

/** UI preferences; nothing secret, so plain shared preferences. */
data class UiPrefs(
    val role: Role = Role.None,
    /** The medical disclaimer was acknowledged by choosing a role (UI-6). */
    val disclaimerAccepted: Boolean = false,
    val theme: ThemeChoice = ThemeChoice.System,
    /** The battery optimisation notice was dismissed. */
    val batteryNoticeDismissed: Boolean = false,
    /** The station was running when the app last stopped. */
    val stationWasRunning: Boolean = false,
)

/** Loads and stores [UiPrefs] and publishes them as a flow. */
class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("nestwatch-ui", Context.MODE_PRIVATE)
    private val state = MutableStateFlow(load())
    val flow: StateFlow<UiPrefs> = state.asStateFlow()
    val value: UiPrefs get() = state.value

    private fun load() = UiPrefs(
        role = runCatching { Role.valueOf(sp.getString("role", null) ?: "None") }.getOrDefault(Role.None),
        disclaimerAccepted = sp.getBoolean("disclaimer", false),
        theme = runCatching { ThemeChoice.valueOf(sp.getString("theme", null) ?: "System") }
            .getOrDefault(ThemeChoice.System),
        batteryNoticeDismissed = sp.getBoolean("battery-notice", false),
        stationWasRunning = sp.getBoolean("station-running", false),
    )

    fun update(f: (UiPrefs) -> UiPrefs) {
        val p = synchronized(this) {
            val p = f(state.value)
            state.value = p
            p
        }
        sp.edit {
            putString("role", p.role.name)
            putBoolean("disclaimer", p.disclaimerAccepted)
            putString("theme", p.theme.name)
            putBoolean("battery-notice", p.batteryNoticeDismissed)
            putBoolean("station-running", p.stationWasRunning)
        }
    }
}
