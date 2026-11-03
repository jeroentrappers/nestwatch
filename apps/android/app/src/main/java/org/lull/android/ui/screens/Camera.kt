// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.screens

import android.Manifest
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.lull.android.R
import org.lull.android.core.Lull
import org.lull.android.ui.components.Caption
import org.lull.android.ui.components.HoldToTalk
import org.lull.android.ui.components.Pill
import org.lull.android.ui.components.PillKind
import org.lull.android.ui.hasPermission
import org.lull.android.ui.rememberPermissionAsk
import org.lull.android.ui.theme.lull
import org.lull.ffi.CameraFacing
import org.lull.ffi.Connection
import org.lull.ffi.TalkState
import org.lull.ffi.VideoState

/** Camera view (README §5.8, FR-CAM-1, FR-CAM-4): live image, night, switch, talk. */
@Composable
fun CameraScreen(id: String, onBack: () -> Unit) {
    val context = LocalContext.current
    val state by Lull.listenerState.collectAsStateWithLifecycle()
    val frames by Lull.frames.collectAsStateWithLifecycle()
    val t = state?.stations?.firstOrNull { it.id == id }
    var facing by remember { mutableStateOf(CameraFacing.BACK) }
    var night by remember { mutableStateOf(false) }
    var image by remember { mutableStateOf<ImageBitmap?>(null) }
    val askMic = rememberPermissionAsk(listOf(Manifest.permission.RECORD_AUDIO), R.string.perm_mic_title, R.string.perm_mic_talk) {}

    // The view is open exactly while this screen is shown (FR-CAM-1).
    DisposableEffect(id) {
        Lull.withListener { it.requestVideo(id, 640u, 480u, 10u) }
        onDispose { Lull.withListener { it.stopVideo(id) } }
    }
    val frame = frames[id]
    LaunchedEffect(frame?.frameId) {
        val f = frame ?: return@LaunchedEffect
        image = withContext(Dispatchers.Default) {
            BitmapFactory.decodeByteArray(f.jpeg, 0, f.jpeg.size)?.asImageBitmap()
        } ?: image
    }

    Page(stringResource(R.string.camera_of, t?.name ?: ""), onBack) {
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(4f / 3f)
                .clip(RoundedCornerShape(16.dp))
                .background(Color.Black),
            contentAlignment = Alignment.Center,
        ) {
            val img = image
            val video = t?.video
            if (img != null && video is VideoState.Streaming) {
                Image(
                    img,
                    contentDescription = stringResource(R.string.camera_alt, t.name),
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Fit,
                )
                Box(
                    Modifier
                        .align(Alignment.TopStart)
                        .padding(8.dp)
                        .clip(RoundedCornerShape(999.dp))
                        .background(Color.Black.copy(alpha = 0.7f)),
                ) {
                    Pill(stringResource(R.string.camera_live), PillKind.Cam)
                }
            } else {
                Text(
                    stringResource(
                        when (video) {
                            is VideoState.Disabled -> R.string.camera_disabled
                            is VideoState.Unavailable -> R.string.camera_unavailable
                            is VideoState.Denied -> R.string.camera_denied
                            is VideoState.Stopped -> R.string.camera_stopped
                            else -> R.string.camera_starting
                        },
                    ),
                    color = Color(0xFFB9B4C8),
                    modifier = Modifier.padding(16.dp),
                )
            }
        }
        (t?.video as? VideoState.Streaming)?.let {
            Caption(stringResource(R.string.frame_info, it.w.toInt(), it.h.toInt(), it.fps.toInt()))
        }
        SwitchRow(stringResource(R.string.night_image), night, { n ->
            night = n
            Lull.withListener { it.setVideoOptions(id, facing, n) }
        })
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            org.lull.android.ui.components.SecondaryButton(stringResource(R.string.switch_camera), {
                facing = if (facing == CameraFacing.BACK) CameraFacing.FRONT else CameraFacing.BACK
                Lull.withListener { it.setVideoOptions(id, facing, night) }
            }, Modifier.weight(1f))
        }
        if (t != null && t.connection is Connection.Live && t.canTalk) {
            HoldToTalk(
                active = t.talk is TalkState.Granted,
                enabled = t.talk !is TalkState.Disabled && t.talk !is TalkState.Busy,
                label = stringResource(if (t.talk is TalkState.Granted) R.string.talking else R.string.hold_to_talk),
                onTalk = { on ->
                    if (on && !context.hasPermission(Manifest.permission.RECORD_AUDIO)) {
                        askMic.ask()
                        return@HoldToTalk
                    }
                    Lull.withListener { it.talk(id, on) }
                },
            )
        }
    }
}
