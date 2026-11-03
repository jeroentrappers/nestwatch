// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.nestwatch.android.core.ThemeChoice

/**
 * The Nestwatch tokens (design/nestwatch.css, design/README.md §3) for Compose:
 * `--ink-0..3` → background, surface, surfaceContainer, outlineVariant;
 * `--honey` → primary; `--lavender` → tertiary; sage, coral and amber
 * live here as custom colours (README §6).
 */
@Immutable
data class NestwatchColors(
    val ink0: Color,
    val ink1: Color,
    val ink2: Color,
    val ink3: Color,
    val text1: Color,
    val text2: Color,
    val text3: Color,
    val honey: Color,
    val lavender: Color,
    val sage: Color,
    val coral: Color,
    val amber: Color,
    val dark: Boolean,
) {
    val honeySoft get() = honey.copy(alpha = if (dark) 0.14f else 0.12f)
    val lavenderSoft get() = lavender.copy(alpha = if (dark) 0.16f else 0.12f)
    val sageSoft get() = sage.copy(alpha = if (dark) 0.14f else 0.12f)
    val coralSoft get() = coral.copy(alpha = if (dark) 0.16f else 0.12f)
    val amberSoft get() = amber.copy(alpha = if (dark) 0.14f else 0.12f)
    /** Text on a filled honey or lavender button. */
    val onAccent get() = if (dark) ink0 else Color(0xFFFFFFFF)
}

val Night = NestwatchColors(
    ink0 = Color(0xFF0F1017), ink1 = Color(0xFF171923), ink2 = Color(0xFF212433), ink3 = Color(0xFF2D3145),
    text1 = Color(0xFFF3EFE8), text2 = Color(0xFFB9B4C8), text3 = Color(0xFF8A86A0),
    honey = Color(0xFFF4BD7E), lavender = Color(0xFFB3A6FF), sage = Color(0xFF93D4B4),
    coral = Color(0xFFFF8F80), amber = Color(0xFFFFD36B), dark = true,
)

val Day = NestwatchColors(
    ink0 = Color(0xFFFAF6EF), ink1 = Color(0xFFFFFFFF), ink2 = Color(0xFFF1ECE3), ink3 = Color(0xFFE2DBCF),
    text1 = Color(0xFF22202B), text2 = Color(0xFF57536A), text3 = Color(0xFF7D7890),
    honey = Color(0xFFB8681C), lavender = Color(0xFF5B4BD1), sage = Color(0xFF2F8A62),
    coral = Color(0xFFD0473A), amber = Color(0xFF9A6A00), dark = false,
)

/** Station night mode (`.dimmed`): near-black with desaturated warm text. */
val Dimmed = Night.copy(
    ink0 = Color(0xFF000000), ink1 = Color(0xFF07070A), ink2 = Color(0xFF0D0D12), ink3 = Color(0xFF16161D),
    text1 = Color(0xFF6D5A44), text2 = Color(0xFF4E4234), text3 = Color(0xFF3A3128),
    honey = Color(0xFF7A5A36), sage = Color(0xFF3F5E4F), coral = Color(0xFF7C4A43), lavender = Color(0xFF4F4A7A),
    amber = Color(0xFF7A5A36),
)

val LocalNestwatch = staticCompositionLocalOf { Night }

private fun scheme(c: NestwatchColors): ColorScheme {
    val base = if (c.dark) darkColorScheme() else lightColorScheme()
    return base.copy(
        primary = c.honey,
        onPrimary = c.onAccent,
        primaryContainer = c.honeySoft,
        onPrimaryContainer = c.text1,
        secondary = c.sage,
        onSecondary = c.onAccent,
        tertiary = c.lavender,
        onTertiary = c.onAccent,
        tertiaryContainer = c.lavenderSoft,
        onTertiaryContainer = c.text1,
        error = c.coral,
        onError = c.onAccent,
        background = c.ink0,
        onBackground = c.text1,
        surface = c.ink1,
        onSurface = c.text1,
        surfaceVariant = c.ink2,
        onSurfaceVariant = c.text2,
        surfaceContainerLowest = c.ink0,
        surfaceContainerLow = c.ink1,
        surfaceContainer = c.ink2,
        surfaceContainerHigh = c.ink2,
        surfaceContainerHighest = c.ink3,
        outline = c.text3,
        outlineVariant = c.ink3,
        inverseSurface = c.text1,
        inverseOnSurface = c.ink0,
    )
}

/** Type scale 12 / 14 / 16 / 20 / 28 / 44 with the system font (README §3.2, §6). */
private val NestwatchTypography = Typography().let { t ->
    val f = FontFamily.Default
    fun s(size: Int, weight: FontWeight = FontWeight.Normal, line: Float = 1.45f) =
        TextStyle(fontFamily = f, fontSize = size.sp, fontWeight = weight, lineHeight = (size * line).sp)
    t.copy(
        displayLarge = s(44, FontWeight.SemiBold, 1.15f),
        displayMedium = s(44, FontWeight.SemiBold, 1.15f),
        displaySmall = s(28, FontWeight.SemiBold, 1.2f),
        headlineLarge = s(28, FontWeight.SemiBold, 1.2f),
        headlineMedium = s(28, FontWeight.SemiBold, 1.2f),
        headlineSmall = s(20, FontWeight.SemiBold, 1.3f),
        titleLarge = s(20, FontWeight.SemiBold, 1.3f),
        titleMedium = s(16, FontWeight.SemiBold),
        titleSmall = s(14, FontWeight.SemiBold),
        bodyLarge = s(16),
        bodyMedium = s(14),
        bodySmall = s(12),
        labelLarge = s(16, FontWeight.Medium),
        labelMedium = s(14, FontWeight.Medium),
        labelSmall = s(12, FontWeight.Medium),
    )
}

/** Radii 10 / 16 / 24 (README §3.3). */
private val NestwatchShapes = Shapes(
    extraSmall = RoundedCornerShape(10.dp),
    small = RoundedCornerShape(10.dp),
    medium = RoundedCornerShape(16.dp),
    large = RoundedCornerShape(24.dp),
    extraLarge = RoundedCornerShape(24.dp),
)

/** Monospace with wide tracking for codes and numbers (README §3.2). */
val MonoCode = TextStyle(fontFamily = FontFamily.Monospace, letterSpacing = 3.sp, fontWeight = FontWeight.SemiBold)

@Composable
fun resolveColors(choice: ThemeChoice, dimmed: Boolean): NestwatchColors = when {
    dimmed -> Dimmed
    choice == ThemeChoice.Night -> Night
    choice == ThemeChoice.Day -> Day
    isSystemInDarkTheme() -> Night
    else -> Day
}

@Composable
fun NestwatchTheme(colors: NestwatchColors, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalNestwatch provides colors) {
        MaterialTheme(colorScheme = scheme(colors), typography = NestwatchTypography, shapes = NestwatchShapes, content = content)
    }
}

/** Shorthand for the current Nestwatch colours. */
val nestwatch: NestwatchColors
    @Composable get() = LocalNestwatch.current
