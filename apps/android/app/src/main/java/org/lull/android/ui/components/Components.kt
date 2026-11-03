// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.ui.components

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.lull.android.R
import org.lull.android.core.fractionToDb
import org.lull.android.core.levelFraction
import org.lull.android.ui.theme.lull
import org.lull.ffi.QrMatrix
import kotlin.math.roundToInt

/** Status pill flavours (`.pill` + `.calm` `.sound` `.warn` `.voice` `.cam`). */
enum class PillKind { Calm, Sound, Warn, Voice, Cam, Neutral }

/** A status pill: always a word and a dot, never colour alone (NFR-A11Y-1). */
@Composable
fun Pill(text: String, kind: PillKind, modifier: Modifier = Modifier) {
    val c = lull
    val (fg, bg) = when (kind) {
        PillKind.Calm -> c.sage to c.sageSoft
        PillKind.Sound -> c.coral to c.coralSoft
        PillKind.Warn -> c.amber to c.amberSoft
        PillKind.Voice -> c.lavender to c.lavenderSoft
        PillKind.Cam -> c.honey to c.honeySoft
        PillKind.Neutral -> c.text2 to c.ink2
    }
    val alpha = if (kind == PillKind.Sound) {
        val t = rememberInfiniteTransition(label = "pulse")
        val a by t.animateFloat(0.35f, 1f, infiniteRepeatable(tween(600), RepeatMode.Reverse), label = "pulse")
        a
    } else {
        1f
    }
    Row(
        modifier
            .clip(RoundedCornerShape(999.dp))
            .background(bg)
            .padding(horizontal = 10.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Canvas(Modifier.size(8.dp)) { drawCircle(fg.copy(alpha = alpha)) }
        Spacer(Modifier.width(6.dp))
        Text(text, color = fg, style = MaterialTheme.typography.labelSmall)
    }
}

/** A card (`.card`); `alert` adds the coral halo while there is sound. */
@Composable
fun LullCard(modifier: Modifier = Modifier, alert: Boolean = false, content: @Composable ColumnScope.() -> Unit) {
    val c = lull
    val shape = RoundedCornerShape(24.dp)
    Column(
        modifier
            .fillMaxWidth()
            .then(if (alert) Modifier.border(2.dp, c.coral.copy(alpha = 0.7f), shape) else Modifier.border(1.dp, c.ink3, shape))
            .clip(shape)
            .background(c.ink1)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        content = content,
    )
}

@Composable
fun Caption(text: String, modifier: Modifier = Modifier, color: Color = lull.text3, align: TextAlign? = null) {
    Text(text, modifier = modifier, color = color, style = MaterialTheme.typography.bodySmall, textAlign = align)
}

@Composable
fun SectionTitle(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier = modifier.semantics { heading() }, style = MaterialTheme.typography.titleMedium, color = lull.text1)
}

@Composable
fun Logo(size: Dp, modifier: Modifier = Modifier) {
    Image(
        painterResource(R.drawable.lull_logo),
        contentDescription = stringResource(R.string.app_name),
        modifier = modifier.size(size),
    )
}

/** Primary action button (`.btn.primary`), at least 48 dp high. */
@Composable
fun PrimaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.heightIn(min = 48.dp),
        colors = ButtonDefaults.buttonColors(containerColor = lull.honey, contentColor = lull.onAccent),
    ) { Text(text, fontWeight = FontWeight.SemiBold) }
}

/** Secondary button (`.btn`). */
@Composable
fun SecondaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    OutlinedButton(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.heightIn(min = 48.dp),
        colors = ButtonDefaults.outlinedButtonColors(containerColor = lull.ink2, contentColor = lull.text1),
        border = androidx.compose.foundation.BorderStroke(1.dp, lull.ink3),
    ) { Text(text) }
}

/** Danger button (`.btn.danger`). */
@Composable
fun DangerButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    TextButton(
        onClick = onClick,
        modifier = modifier.heightIn(min = 48.dp),
        colors = ButtonDefaults.textButtonColors(containerColor = lull.coralSoft, contentColor = lull.coral),
    ) { Text(text) }
}

/**
 * The station's level ring (`.ring`): the arc fills with the level,
 * the tick marks the strictest listener threshold (UI-2), the centre shows
 * the number and "quiet" or "sound".
 */
@Composable
fun LevelRing(
    levelDb: Float,
    thresholdDb: Float?,
    sound: Boolean,
    modifier: Modifier = Modifier,
    size: Dp = 220.dp,
    showNumbers: Boolean = true,
) {
    val c = lull
    val target = levelFraction(levelDb)
    val f by animateFloatAsState(target, tween(160), label = "level")
    val label = stringResource(R.string.ring_label)
    val word = stringResource(if (sound) R.string.ring_sound else R.string.ring_quiet)
    val db = "${levelDb.roundToInt()} ${stringResource(R.string.db)}"
    Box(
        modifier
            .size(size)
            .clearAndSetSemantics { contentDescription = "$label: $db, $word" },
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.fillMaxSize()) {
            val stroke = this.size.minDimension * 0.07f
            val inset = stroke / 2 + 4f
            val arcSize = Size(this.size.width - inset * 2, this.size.height - inset * 2)
            val topLeft = Offset(inset, inset)
            drawArc(c.ink3, 135f, 270f, false, topLeft, arcSize, style = Stroke(stroke, cap = StrokeCap.Round))
            drawArc(
                if (sound) c.coral else c.sage, 135f, 270f * f, false, topLeft, arcSize,
                style = Stroke(stroke, cap = StrokeCap.Round),
            )
            thresholdDb?.let { t ->
                val angle = Math.toRadians((135.0 + 270.0 * levelFraction(t)))
                val r = arcSize.width / 2
                val center = Offset(this.size.width / 2, this.size.height / 2)
                val a = Offset(
                    center.x + ((r - stroke) * kotlin.math.cos(angle)).toFloat(),
                    center.y + ((r - stroke) * kotlin.math.sin(angle)).toFloat(),
                )
                val b = Offset(
                    center.x + ((r + stroke) * kotlin.math.cos(angle)).toFloat(),
                    center.y + ((r + stroke) * kotlin.math.sin(angle)).toFloat(),
                )
                drawLine(c.text1, a, b, strokeWidth = 4f, cap = StrokeCap.Round)
            }
        }
        if (showNumbers) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    "${levelDb.roundToInt()}",
                    style = MaterialTheme.typography.displayMedium,
                    color = c.text1,
                )
                Caption(stringResource(R.string.db) + " · " + word, color = if (sound) c.coral else c.text2)
            }
        }
    }
}

/** A listener tile's level meter (`.meter`) with this listener's threshold mark. */
@Composable
fun LevelMeter(levelDb: Float?, thresholdDb: Float?, sound: Boolean, modifier: Modifier = Modifier) {
    val c = lull
    val f by animateFloatAsState(levelDb?.let { levelFraction(it) } ?: 0f, tween(160), label = "meter")
    Canvas(
        modifier
            .fillMaxWidth()
            .height(12.dp),
    ) {
        val r = CornerRadius(size.height / 2)
        drawRoundRect(c.ink3, cornerRadius = r)
        if (levelDb != null) {
            drawRoundRect(if (sound) c.coral else c.sage, size = Size(size.width * f, size.height), cornerRadius = r)
        }
        thresholdDb?.let {
            val x = size.width * levelFraction(it)
            drawLine(c.text1, Offset(x, -2f), Offset(x, size.height + 2f), strokeWidth = 3f)
        }
    }
}

/** A QR code drawn from the core's module matrix with a light quiet zone. */
@Composable
fun QrImage(qr: QrMatrix, description: String, modifier: Modifier = Modifier) {
    val n = qr.size.toInt()
    Canvas(
        modifier
            .aspectRatio(1f)
            .clip(RoundedCornerShape(16.dp))
            .background(Color.White)
            .padding(12.dp)
            .semantics { contentDescription = description },
    ) {
        val cell = size.minDimension / n
        for (y in 0 until n) {
            var x = 0
            while (x < n) {
                if (qr.modules[y * n + x].toInt() == 1) {
                    val start = x
                    while (x < n && qr.modules[y * n + x].toInt() == 1) x++
                    drawRect(
                        Color(0xFF111111),
                        topLeft = Offset(start * cell, y * cell),
                        size = Size((x - start) * cell + 0.5f, cell + 0.5f),
                    )
                } else {
                    x++
                }
            }
        }
    }
}

/**
 * The calibration chart (`.calibrate`): the last minute of room level with
 * a threshold line the user drags (FR-AU-2, UI-5). A slider next to it
 * offers the same control to screen readers and keyboards.
 */
@Composable
fun CalibrationChart(
    points: List<Pair<Float, Float>>,
    thresholdDb: Float,
    onThreshold: (Float) -> Unit,
    activated: Boolean,
    modifier: Modifier = Modifier,
) {
    val c = lull
    Canvas(
        modifier
            .fillMaxWidth()
            .height(140.dp)
            .clip(RoundedCornerShape(16.dp))
            .background(c.ink2)
            .pointerInput(Unit) {
                detectTapGestures { o -> onThreshold(fractionToDb(1f - o.y / size.height)) }
            }
            .pointerInput(Unit) {
                detectDragGestures { change, _ -> onThreshold(fractionToDb(1f - change.position.y / size.height)) }
            }
            .clearAndSetSemantics {},
    ) {
        val h = size.height
        val w = size.width
        fun y(db: Float) = h * (1f - levelFraction(db))
        val ty = y(thresholdDb)
        if (activated) drawRect(c.coralSoft, topLeft = Offset(0f, 0f), size = Size(w, ty))
        if (points.size >= 2) {
            val p = Path()
            points.forEachIndexed { i, (x, db) ->
                if (i == 0) p.moveTo(x * w, y(db)) else p.lineTo(x * w, y(db))
            }
            drawPath(p, c.sage, style = Stroke(3f, cap = StrokeCap.Round))
        }
        drawLine(
            c.honey, Offset(0f, ty), Offset(w, ty), strokeWidth = 4f,
            pathEffect = if (activated) null else PathEffect.dashPathEffect(floatArrayOf(12f, 10f)),
        )
        drawCircle(c.honey, radius = 10f, center = Offset(w - 16f, ty))
    }
}

/**
 * Hold to talk (`.btn.talk`): the largest control on a tile. Pressing
 * requests talk-back, releasing ends it; screen readers toggle it with a
 * double tap.
 */
@Composable
fun HoldToTalk(
    active: Boolean,
    enabled: Boolean,
    label: String,
    onTalk: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val c = lull
    val bg = when {
        !enabled -> c.ink2
        active -> c.lavender
        else -> c.lavenderSoft
    }
    val fg = when {
        !enabled -> c.text3
        active -> c.onAccent
        else -> c.lavender
    }
    val state = stringResource(if (active) R.string.talk_state_on else R.string.talk_state_off)
    Box(
        modifier
            .fillMaxWidth()
            .heightIn(min = 64.dp)
            .clip(RoundedCornerShape(16.dp))
            .background(bg)
            .semantics(mergeDescendants = true) {
                role = Role.Button
                stateDescription = state
            }
            .pointerInput(enabled) {
                if (!enabled) return@pointerInput
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    onTalk(true)
                    waitForUpOrCancellation()
                    onTalk(false)
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = fg, style = MaterialTheme.typography.titleMedium)
    }
}

/** A small rounded square with a mono value, e.g. the verification number. */
@Composable
fun CodeBox(text: String, modifier: Modifier = Modifier) {
    Box(
        modifier
            .clip(RoundedCornerShape(16.dp))
            .background(lull.ink2)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = org.lull.android.ui.theme.MonoCode.copy(fontSize = MaterialTheme.typography.headlineSmall.fontSize), color = lull.text1)
    }
}

/** A small dot shown on a camera indicator (FR-CAM-2). */
@Composable
fun Dot(color: Color, size: Dp = 10.dp) {
    Box(Modifier.size(size).clip(CircleShape).background(color))
}
