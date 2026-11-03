// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.camera

import android.content.Context
import android.os.SystemClock
import android.util.Log
import android.util.Size
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import org.nestwatch.ffi.CameraFacing
import org.nestwatch.ffi.CameraImage
import org.nestwatch.ffi.CameraRequest
import org.nestwatch.ffi.ImageFormat
import java.util.concurrent.Executors

private const val TAG = "NestwatchCamera"

/**
 * The station's on-demand camera (FR-CAM-1, media.md §5): CameraX image
 * analysis in RGBA, throttled to the requested frame rate and handed to
 * [sink] with its rotation. It runs only between [start] and [stop], i.e.
 * while the engine reports the camera as wanted.
 */
class StationCamera(
    private val context: Context,
    private val owner: LifecycleOwner,
    private val sink: (CameraImage) -> Unit,
    private val onUnavailable: () -> Unit,
) {
    private val executor = Executors.newSingleThreadExecutor()
    private var provider: ProcessCameraProvider? = null
    private var analysis: ImageAnalysis? = null
    private var current: CameraRequest? = null
    @Volatile private var minIntervalMs = 100L
    @Volatile private var lastFrameMs = 0L

    /** Starts or reconfigures the camera; call on the main thread. */
    fun start(req: CameraRequest) {
        minIntervalMs = 1000L / req.maxFps.toLong().coerceIn(1, 30)
        val prev = current
        current = req
        if (prev != null && prev.facing == req.facing && prev.maxW == req.maxW && prev.maxH == req.maxH && analysis != null) {
            return
        }
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener({
            val p = runCatching { future.get() }.getOrNull() ?: run {
                onUnavailable()
                return@addListener
            }
            provider = p
            val want = current ?: return@addListener
            bind(p, want)
        }, ContextCompat.getMainExecutor(context))
    }

    private fun bind(p: ProcessCameraProvider, req: CameraRequest) {
        p.unbindAll()
        val wanted = if (req.facing == CameraFacing.FRONT) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA
        val selector = when {
            runCatching { p.hasCamera(wanted) }.getOrDefault(false) -> wanted
            runCatching { p.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA) }.getOrDefault(false) -> CameraSelector.DEFAULT_BACK_CAMERA
            runCatching { p.hasCamera(CameraSelector.DEFAULT_FRONT_CAMERA) }.getOrDefault(false) -> CameraSelector.DEFAULT_FRONT_CAMERA
            else -> {
                onUnavailable()
                return
            }
        }
        val w = req.maxW.toInt().coerceIn(160, 1280)
        val h = req.maxH.toInt().coerceIn(120, 960)
        val a = ImageAnalysis.Builder()
            .setResolutionSelector(
                ResolutionSelector.Builder()
                    .setResolutionStrategy(
                        ResolutionStrategy(Size(w, h), ResolutionStrategy.FALLBACK_RULE_CLOSEST_LOWER_THEN_HIGHER),
                    )
                    .build(),
            )
            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
            .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_RGBA_8888)
            .build()
        a.setAnalyzer(executor) { img -> img.use { deliver(it) } }
        try {
            p.bindToLifecycle(owner, selector, a)
            analysis = a
        } catch (e: Exception) {
            Log.w(TAG, "bind failed: ${e.javaClass.simpleName}")
            analysis = null
            onUnavailable()
        }
    }

    private fun deliver(img: ImageProxy) {
        val now = SystemClock.elapsedRealtime()
        if (now - lastFrameMs < minIntervalMs) return
        lastFrameMs = now
        val plane = img.planes.firstOrNull() ?: return
        val w = img.width
        val h = img.height
        val rowBytes = w * 4
        val buf = plane.buffer
        val stride = plane.rowStride
        val out = ByteArray(rowBytes * h)
        if (stride == rowBytes && plane.pixelStride == 4) {
            buf.rewind()
            buf.get(out, 0, minOf(out.size, buf.remaining()))
        } else {
            for (y in 0 until h) {
                buf.position(y * stride)
                buf.get(out, y * rowBytes, minOf(rowBytes, buf.remaining()))
            }
        }
        sink(
            CameraImage(
                width = w.toUInt(),
                height = h.toUInt(),
                format = ImageFormat.RGBA,
                rotation = img.imageInfo.rotationDegrees.toUShort(),
                data = out,
            ),
        )
    }

    /** Releases the camera at once (FR-CAM-1). */
    fun stop() {
        current = null
        analysis?.clearAnalyzer()
        analysis = null
        provider?.unbindAll()
    }

    fun close() {
        stop()
        executor.shutdown()
    }
}
