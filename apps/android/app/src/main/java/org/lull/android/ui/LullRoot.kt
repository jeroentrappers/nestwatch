// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.core.content.ContextCompat
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.lull.android.MainActivity
import org.lull.android.R
import org.lull.android.core.Lull
import org.lull.android.core.Role
import org.lull.android.service.ListenerService
import org.lull.android.service.StationService
import org.lull.android.ui.screens.AboutScreen
import org.lull.android.ui.screens.AddStationScreen
import org.lull.android.ui.screens.CameraScreen
import org.lull.android.ui.screens.ListenerHome
import org.lull.android.ui.screens.ListenerStationSettings
import org.lull.android.ui.screens.StationHome
import org.lull.android.ui.screens.StationSettingsScreen
import org.lull.android.ui.screens.WelcomeScreen
import org.lull.android.ui.theme.LullTheme
import org.lull.android.ui.theme.resolveColors
import org.lull.ffi.LullException

/** The screens of design/README.md §5. */
sealed interface Screen {
    data object Welcome : Screen
    data object Station : Screen
    data object StationSettings : Screen
    data object Listener : Screen
    data class StationPrefs(val id: String) : Screen
    data class Camera(val id: String) : Screen
    data class AddStation(val invite: String? = null) : Screen
    data object About : Screen
}

/** Simple back stack navigation. */
class Nav(start: Screen) {
    val stack = mutableStateListOf(start)
    val top: Screen get() = stack.last()
    fun push(s: Screen) {
        stack.add(s)
    }
    fun pop(): Boolean {
        if (stack.size <= 1) return false
        stack.removeAt(stack.lastIndex)
        return true
    }
    fun reset(s: Screen) {
        stack.clear()
        stack.add(s)
    }
}

fun Context.hasPermission(p: String) = ContextCompat.checkSelfPermission(this, p) == PackageManager.PERMISSION_GRANTED

/** Starts the station service (permissions were asked before). */
fun startStation(context: Context) {
    Lull.prefs.update { it.copy(stationWasRunning = true) }
    StationService.start(context)
}

fun stopStation(context: Context) {
    Lull.prefs.update { it.copy(stationWasRunning = false) }
    StationService.stop(context)
}

/** Starts the listener engine and its service. */
fun startListening(context: Context) {
    Lull.listener()
    ListenerService.start(context)
}

fun stopListening(context: Context) {
    ListenerService.stop(context)
    Lull.stopListener()
}

/** Opens the app's system settings page (after a permanent denial). */
fun openAppSettings(context: Context) {
    context.startActivity(
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
    )
}

/**
 * Asks for the battery optimisation exemption (platforms.md §2.2). A baby
 * monitor must keep running with the screen off, which is an accepted use
 * of this request.
 */
@android.annotation.SuppressLint("BatteryLife")
fun requestBatteryExemption(context: Context) {
    runCatching {
        context.startActivity(
            Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, "package:${context.packageName}".toUri())
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }.onFailure {
        runCatching {
            context.startActivity(
                Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }
    }
}

/** A user-facing message for an engine error. */
@Composable
fun errorText(e: Throwable, invalid: Int = R.string.err_generic): String = stringResource(
    when (e) {
        is LullException.Invalid -> if (e.reason.contains("station id")) R.string.err_invalid_id else invalid
        is LullException.Unreachable -> R.string.err_unreachable
        is LullException.Refused -> R.string.err_refused
        is LullException.Rejected -> R.string.err_rejected
        is LullException.TooManyStations -> R.string.err_too_many
        is LullException.NotConnected -> R.string.err_not_connected
        else -> R.string.err_generic
    },
)

/**
 * A permission request with an explanation first (security.md PRIV-4).
 * Call `ask()` to show the explanation; `onResult` gets whether all
 * permissions were granted.
 */
class PermissionAsk(val ask: () -> Unit)

@Composable
fun rememberPermissionAsk(
    permissions: List<String>,
    title: Int,
    body: Int,
    onResult: (Boolean) -> Unit,
): PermissionAsk {
    val context = LocalContext.current
    var explaining by remember { mutableStateOf(false) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { res ->
        onResult(permissions.all { res[it] == true || context.hasPermission(it) })
    }
    if (explaining) {
        AlertDialog(
            onDismissRequest = { explaining = false; onResult(false) },
            title = { Text(stringResource(title)) },
            text = { Text(stringResource(body)) },
            confirmButton = {
                TextButton(onClick = {
                    explaining = false
                    launcher.launch(permissions.toTypedArray())
                }) { Text(stringResource(R.string.perm_continue)) }
            },
            dismissButton = {
                TextButton(onClick = { explaining = false; onResult(false) }) {
                    Text(stringResource(R.string.perm_not_now))
                }
            },
        )
    }
    return remember(permissions) {
        PermissionAsk {
            if (permissions.all { context.hasPermission(it) }) onResult(true) else explaining = true
        }
    }
}

/** Notification permission on API 33+ (platforms.md §2.2). */
val notificationPermission: List<String> =
    if (Build.VERSION.SDK_INT >= 33) listOf(Manifest.permission.POST_NOTIFICATIONS) else emptyList()

@Composable
fun LullRoot(activity: MainActivity) {
    val prefs by Lull.prefs.flow.collectAsStateWithLifecycle()
    val nav = remember {
        Nav(
            when (Lull.prefs.value.role) {
                Role.None -> Screen.Welcome
                Role.Station -> Screen.Station
                Role.Listener -> Screen.Listener
            },
        )
    }
    var dimmed by remember { mutableStateOf(false) }
    val context = LocalContext.current

    // Keep the engines running for the chosen role across process restarts.
    LaunchedEffect(Unit) {
        when (Lull.prefs.value.role) {
            Role.Listener -> startListening(context)
            Role.Station -> if (Lull.prefs.value.stationWasRunning && context.hasPermission(Manifest.permission.RECORD_AUDIO)) {
                startStation(context)
            }
            Role.None -> {}
        }
    }
    val invite by activity.pendingInvite.collectAsStateWithLifecycle()
    LaunchedEffect(invite, prefs.role) {
        val i = invite ?: return@LaunchedEffect
        if (prefs.role == Role.Listener) {
            activity.pendingInvite.value = null
            nav.push(Screen.AddStation(i))
        }
    }
    LaunchedEffect(dimmed) { activity.setDimmed(dimmed) }

    BackHandler(enabled = dimmed || nav.stack.size > 1) {
        if (dimmed) dimmed = false else nav.pop()
    }

    val colors = resolveColors(prefs.theme, dimmed)
    // Status and navigation bar icons follow the resolved theme, not the system one.
    androidx.compose.runtime.SideEffect {
        val c = androidx.core.view.WindowCompat.getInsetsController(activity.window, activity.window.decorView)
        c.isAppearanceLightStatusBars = !colors.dark
        c.isAppearanceLightNavigationBars = !colors.dark
    }
    LullTheme(colors) {
        Surface(Modifier.fillMaxSize()) {
            when (val s = nav.top) {
                Screen.Welcome -> WelcomeScreen(onRole = { role ->
                    Lull.prefs.update { it.copy(role = role, disclaimerAccepted = true) }
                    nav.reset(if (role == Role.Station) Screen.Station else Screen.Listener)
                })
                Screen.Station -> StationHome(
                    dimmed = dimmed,
                    onDim = { dimmed = it },
                    onSettings = { nav.push(Screen.StationSettings) },
                    onAbout = { nav.push(Screen.About) },
                )
                Screen.StationSettings -> StationSettingsScreen(
                    onBack = { nav.pop() },
                    onSwitchRole = {
                        stopStation(context)
                        Lull.prefs.update { it.copy(role = Role.None) }
                        nav.reset(Screen.Welcome)
                    },
                )
                Screen.Listener -> ListenerHome(
                    onAdd = { nav.push(Screen.AddStation()) },
                    onStation = { nav.push(Screen.StationPrefs(it)) },
                    onCamera = { nav.push(Screen.Camera(it)) },
                    onAbout = { nav.push(Screen.About) },
                    onSwitchRole = {
                        stopListening(context)
                        Lull.prefs.update { it.copy(role = Role.None) }
                        nav.reset(Screen.Welcome)
                    },
                )
                is Screen.StationPrefs -> ListenerStationSettings(s.id, onBack = { nav.pop() })
                is Screen.Camera -> CameraScreen(s.id, onBack = { nav.pop() })
                is Screen.AddStation -> AddStationScreen(s.invite, onDone = { nav.pop() })
                Screen.About -> AboutScreen(onBack = { nav.pop() })
            }
        }
    }
}
