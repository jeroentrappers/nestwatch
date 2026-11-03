// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.screens

import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import org.lull.android.R
import org.lull.android.ui.components.Caption
import org.lull.android.ui.components.SecondaryButton
import org.lull.android.ui.theme.lull
import org.lull.ffi.parseInvitation
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Decodes QR codes from the luminance plane of CameraX frames with ZXing
 * core (pure Java, no Google Play services). Only Lull invitations are
 * accepted.
 */
class QrDecoder {
    private val reader = MultiFormatReader().apply {
        setHints(
            mapOf(
                DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
                DecodeHintType.TRY_HARDER to true,
            ),
        )
    }

    /** Decodes a luminance plane; null if no QR code is found. */
    fun decode(y: ByteArray, rowStride: Int, width: Int, height: Int): String? {
        val src = PlanarYUVLuminanceSource(y, rowStride, height, 0, 0, width, height, false)
        return try {
            reader.decodeWithState(BinaryBitmap(HybridBinarizer(src))).text
        } catch (_: NotFoundException) {
            null
        } catch (_: Exception) {
            null
        } finally {
            reader.reset()
        }
    }

    fun decode(img: ImageProxy): String? {
        val plane = img.planes.firstOrNull() ?: return null
        val buf = plane.buffer
        val bytes = ByteArray(buf.remaining())
        buf.get(bytes)
        // The last row may be shorter than the stride.
        val stride = plane.rowStride
        val needed = stride * (img.height - 1) + img.width
        if (bytes.size < needed) return null
        val y = if (bytes.size >= stride * img.height) bytes else bytes.copyOf(stride * img.height)
        return decode(y, stride, img.width, img.height)
    }
}

/** Full-screen scanner; calls [onResult] once with a valid invitation. */
@Composable
fun QrScanner(onResult: (String) -> Unit, onCancel: () -> Unit) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    var notLull by remember { mutableStateOf(false) }
    val done = remember { AtomicBoolean(false) }
    val executor = remember { Executors.newSingleThreadExecutor() }
    val previewView = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }

    DisposableEffect(owner) {
        val future = ProcessCameraProvider.getInstance(context)
        var provider: ProcessCameraProvider? = null
        future.addListener({
            val p = runCatching { future.get() }.getOrNull() ?: return@addListener
            provider = p
            val preview = Preview.Builder().build().also { it.surfaceProvider = previewView.surfaceProvider }
            val decoder = QrDecoder()
            val analysis = ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
            analysis.setAnalyzer(executor) { img ->
                img.use {
                    if (done.get()) return@use
                    val text = decoder.decode(it) ?: return@use
                    if (runCatching { parseInvitation(text) }.isSuccess) {
                        if (done.compareAndSet(false, true)) {
                            ContextCompat.getMainExecutor(context).execute { onResult(text) }
                        }
                    } else {
                        notLull = true
                    }
                }
            }
            val selector = if (p.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA)) CameraSelector.DEFAULT_BACK_CAMERA
            else CameraSelector.DEFAULT_FRONT_CAMERA
            runCatching {
                p.unbindAll()
                p.bindToLifecycle(owner, selector, preview, analysis)
            }
        }, ContextCompat.getMainExecutor(context))
        onDispose {
            provider?.unbindAll()
            executor.shutdown()
        }
    }

    Page(stringResource(R.string.scan_qr), onCancel) {
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(3f / 4f)
                .clip(RoundedCornerShape(24.dp)),
        ) {
            AndroidView({ previewView }, Modifier.fillMaxWidth().aspectRatio(3f / 4f))
        }
        Caption(stringResource(R.string.scan_hint))
        if (notLull) Text(stringResource(R.string.scan_not_lull), color = lull.amber)
        SecondaryButton(stringResource(R.string.cancel), onCancel, Modifier.fillMaxWidth())
    }
}
