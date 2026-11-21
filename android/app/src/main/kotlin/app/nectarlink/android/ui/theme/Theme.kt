// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.theme

import android.animation.ValueAnimator
import android.app.Activity
import android.graphics.Color as AndroidColor
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.core.graphics.ColorUtils
import androidx.core.view.WindowCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner

val LocalReducedMotion = compositionLocalOf { false }
val LocalGlassTheme = compositionLocalOf { false }
val LocalAmoledTheme = compositionLocalOf { false }
internal val LocalBloomHomeCardPalette = compositionLocalOf<Palette?> { null }

/** True when Android's "Remove animations" (animator duration scale 0) is enabled. */
@Composable
fun rememberReducedMotion(): Boolean {
    var reduced by remember { mutableStateOf(!ValueAnimator.areAnimatorsEnabled()) }
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                reduced = !ValueAnimator.areAnimatorsEnabled()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }
    return reduced
}

/** The user's look choices (see Preferences). */
@androidx.compose.runtime.Immutable
data class Appearance(
    val theme: String = "graphite",
    val mode: String = "system",
    val seed: String = Tokens.DEFAULT_SEED,
    /** Bloom/Glass takes its colors from the wallpaper (Android 12+). */
    val dynamicColor: Boolean = true,
    /** True #000000 OLED black surfaces when in dark mode. */
    val amoled: Boolean = false,
    /** Hive's comb breathes and pulses (still with reduced motion or battery saver). */
    val combMotion: Boolean = true,
    /** A Bloom preset for the Home device cards, or null to follow the theme. */
    val homeCardSeed: String? = null,
)

internal fun Appearance.bloomHomeCardPalette(dark: Boolean): Palette? {
    if (theme != "bloom") return null
    val preset = Tokens.bloomSeeds[homeCardSeed] ?: return null
    return if (dark) preset.dark else preset.light
}

private fun hslTone(hue: Float, sat: Float, lightness: Float): Color {
    val argb = ColorUtils.HSLToColor(
        floatArrayOf(
            hue.coerceIn(0f, 360f),
            sat.coerceIn(0f, 1f),
            lightness.coerceIn(0f, 1f),
        ),
    )
    return Color(argb)
}

private fun rgbColor(rgb: UInt): Color = Color(0xFF00_0000L or (rgb.toLong() and 0xFF_FFFFL))

/**
 * Builds a Material 3 [ColorScheme] from a `0xRRGGBB` media artwork seed color
 * using the shared Rust perceptual tonal palette engine (`app.nectarlink.core.themeSchemeFromSeed`),
 * ensuring identical light and dark schemes on Android and Desktop.
 */
private fun schemeFromArtworkSeed(rgb: UInt, dark: Boolean, error: Color): ColorScheme {
    val ffiScheme = runCatching { app.nectarlink.core.themeSchemeFromSeed(rgb) }.getOrNull()
    if (ffiScheme != null) {
        val role = if (dark) ffiScheme.dark else ffiScheme.light
        return Palette(
            primary = rgbColor(role.primary),
            onPrimary = rgbColor(role.onPrimary),
            primaryContainer = rgbColor(role.primaryContainer),
            onPrimaryContainer = rgbColor(role.onPrimaryContainer),
            secondaryContainer = rgbColor(role.secondaryContainer),
            onSecondaryContainer = rgbColor(role.onSecondaryContainer),
            surface = rgbColor(role.surface),
            surfaceContainerLow = rgbColor(role.surfaceContainerLow),
            surfaceContainer = rgbColor(role.surfaceContainer),
            surfaceContainerHigh = rgbColor(role.surfaceContainerHigh),
            surfaceContainerHighest = rgbColor(role.surfaceContainerHighest),
            onSurface = rgbColor(role.onSurface),
            onSurfaceVariant = rgbColor(role.onSurfaceVariant),
            outline = rgbColor(role.outline),
            outlineVariant = rgbColor(role.outlineVariant),
        ).toScheme(dark, error)
    }

    val r = ((rgb shr 16) and 0xFFu).toInt()
    val g = ((rgb shr 8) and 0xFFu).toInt()
    val b = (rgb and 0xFFu).toInt()
    val hsl = FloatArray(3)
    ColorUtils.RGBToHSL(r, g, b, hsl)
    val h = hsl[0]
    val s = hsl[1].coerceIn(0.36f, 0.88f)
    val seedLuma = (0.2126f * r + 0.7152f * g + 0.0722f * b) / 255f
    val secondaryHue = (h + 24f) % 360f
    val neutralS = (s * 0.24f).coerceIn(0.08f, 0.20f)
    val variantS = (s * 0.36f).coerceIn(0.14f, 0.30f)
    val contrastBoost = (seedLuma - 0.5f) * 0.04f

    val pal = if (dark) {
        Palette(
            primary = hslTone(h, (s * 0.92f).coerceAtLeast(0.55f), (0.78f + contrastBoost).coerceIn(0.74f, 0.84f)),
            onPrimary = hslTone(h, (s * 0.85f).coerceAtLeast(0.45f), 0.15f),
            primaryContainer = hslTone(h, (s * 0.80f).coerceAtLeast(0.45f), (0.27f + contrastBoost).coerceIn(0.22f, 0.32f)),
            onPrimaryContainer = hslTone(h, (s * 0.92f).coerceAtLeast(0.55f), 0.91f),
            secondaryContainer = hslTone(secondaryHue, (s * 0.54f).coerceAtLeast(0.30f), 0.26f),
            onSecondaryContainer = hslTone(secondaryHue, (s * 0.72f).coerceAtLeast(0.42f), 0.89f),
            surface = hslTone(h, neutralS, 0.07f),
            surfaceContainerLow = hslTone(h, neutralS, 0.05f),
            surfaceContainer = hslTone(h, neutralS, 0.11f),
            surfaceContainerHigh = hslTone(h, neutralS, 0.16f),
            surfaceContainerHighest = hslTone(h, neutralS, 0.21f),
            onSurface = hslTone(h, neutralS, 0.92f),
            onSurfaceVariant = hslTone(h, variantS, 0.79f),
            outline = hslTone(h, variantS, 0.56f),
            outlineVariant = hslTone(h, variantS, 0.28f),
        )
    } else {
        Palette(
            primary = hslTone(h, (s * 0.88f).coerceAtLeast(0.52f), (0.34f - contrastBoost).coerceIn(0.28f, 0.40f)),
            onPrimary = Color.White,
            primaryContainer = hslTone(h, (s * 0.88f).coerceAtLeast(0.50f), 0.88f),
            onPrimaryContainer = hslTone(h, (s * 0.85f).coerceAtLeast(0.45f), 0.13f),
            secondaryContainer = hslTone(secondaryHue, (s * 0.56f).coerceAtLeast(0.30f), 0.89f),
            onSecondaryContainer = hslTone(secondaryHue, (s * 0.62f).coerceAtLeast(0.34f), 0.14f),
            surface = hslTone(h, neutralS, 0.98f),
            surfaceContainerLow = hslTone(h, neutralS, 0.95f),
            surfaceContainer = hslTone(h, neutralS, 0.93f),
            surfaceContainerHigh = hslTone(h, neutralS, 0.90f),
            surfaceContainerHighest = hslTone(h, neutralS, 0.87f),
            onSurface = hslTone(h, neutralS, 0.10f),
            onSurfaceVariant = hslTone(h, variantS, 0.27f),
            outline = hslTone(h, variantS, 0.46f),
            outlineVariant = hslTone(h, variantS, 0.78f),
        )
    }
    return pal.toScheme(dark, error)
}

private fun Palette.toScheme(dark: Boolean, error: Color): ColorScheme {
    val base = if (dark) darkColorScheme() else lightColorScheme()
    return base.copy(
        primary = primary,
        onPrimary = onPrimary,
        primaryContainer = primaryContainer,
        onPrimaryContainer = onPrimaryContainer,
        secondary = primary,
        onSecondary = onPrimary,
        secondaryContainer = secondaryContainer,
        onSecondaryContainer = onSecondaryContainer,
        tertiary = primary,
        onTertiary = onPrimary,
        tertiaryContainer = secondaryContainer,
        onTertiaryContainer = onSecondaryContainer,
        background = surface,
        onBackground = onSurface,
        surface = surface,
        onSurface = onSurface,
        surfaceVariant = surfaceContainerHigh,
        onSurfaceVariant = onSurfaceVariant,
        surfaceContainerLowest = surface,
        surfaceContainerLow = surfaceContainerLow,
        surfaceContainer = surfaceContainer,
        surfaceContainerHigh = surfaceContainerHigh,
        surfaceContainerHighest = surfaceContainerHighest,
        outline = outline,
        outlineVariant = outlineVariant,
        error = error,
    )
}

/**
 * Applies adaptive surface translucency in Glass mode with a subtle uniform
 * hairline border and high-legibility modal containers.
 */
private fun ColorScheme.toGlassScheme(dark: Boolean): ColorScheme {
    val tintArgb = primary.let {
        android.graphics.Color.argb(
            (it.alpha * 255).toInt(),
            (it.red * 255).toInt(),
            (it.green * 255).toInt(),
            (it.blue * 255).toInt(),
        )
    }
    fun blendWithTint(c: Color, ratio: Float, alpha: Float): Color {
        val cArgb = android.graphics.Color.argb(
            255,
            (c.red * 255).toInt(),
            (c.green * 255).toInt(),
            (c.blue * 255).toInt(),
        )
        return Color(ColorUtils.blendARGB(cArgb, tintArgb, ratio)).copy(alpha = alpha)
    }
    return if (dark) {
        copy(
            surfaceContainerLowest = surface.copy(alpha = 0.52f),
            surfaceContainerLow = blendWithTint(surfaceContainerLow, 0.08f, 0.62f),
            surfaceContainer = blendWithTint(surfaceContainer, 0.10f, 0.72f),
            surfaceContainerHigh = blendWithTint(surfaceContainerHigh, 0.12f, 0.88f),
            surfaceContainerHighest = blendWithTint(surfaceContainerHighest, 0.14f, 0.94f),
            surfaceVariant = blendWithTint(surfaceVariant, 0.10f, 0.78f),
            primaryContainer = primaryContainer.copy(alpha = 0.82f),
            secondaryContainer = secondaryContainer.copy(alpha = 0.78f),
            tertiaryContainer = tertiaryContainer.copy(alpha = 0.80f),
            outlineVariant = Color.White.copy(alpha = 0.12f),
        )
    } else {
        copy(
            surfaceContainerLowest = Color.White.copy(alpha = 0.64f),
            surfaceContainerLow = blendWithTint(Color.White, 0.04f, 0.74f),
            surfaceContainer = blendWithTint(Color.White, 0.06f, 0.82f),
            surfaceContainerHigh = blendWithTint(surfaceContainerHigh, 0.10f, 0.92f),
            surfaceContainerHighest = blendWithTint(surfaceContainerHighest, 0.12f, 0.96f),
            surfaceVariant = blendWithTint(surfaceVariant, 0.08f, 0.84f),
            primaryContainer = primaryContainer.copy(alpha = 0.86f),
            secondaryContainer = secondaryContainer.copy(alpha = 0.84f),
            tertiaryContainer = tertiaryContainer.copy(alpha = 0.86f),
            outlineVariant = primary.copy(alpha = 0.18f),
        )
    }
}

/**
 * Stock accents that some skins (Samsung One UI with its "Color palette" option off)
 * report from `system_accent1_*` regardless of the wallpaper.
 */
private val STOCK_SYSTEM_ACCENTS = setOf(0xFF697E9B.toInt(), 0xFF006782.toInt())

/**
 * The wallpaper's primary color as an `0xRRGGBB` seed, kept current through
 * [android.app.WallpaperManager.OnColorsChangedListener]. Null when disabled or unavailable.
 */
@Composable
private fun rememberWallpaperSeed(enabled: Boolean): UInt? {
    val context = LocalContext.current
    var seed by remember { mutableStateOf<UInt?>(null) }
    DisposableEffect(context, enabled) {
        if (!enabled || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            seed = null
            return@DisposableEffect onDispose { }
        }
        val wm = context.getSystemService(android.app.WallpaperManager::class.java)
        fun read(colors: android.app.WallpaperColors?): UInt? =
            colors?.primaryColor?.toArgb()?.let { (it.toLong() and 0xFF_FFFFL).toUInt() }
        seed = runCatching {
            read(
                wm?.getWallpaperColors(android.app.WallpaperManager.FLAG_SYSTEM)
                    ?: wm?.getWallpaperColors(android.app.WallpaperManager.FLAG_LOCK),
            )
        }.getOrNull()
        val listener = android.app.WallpaperManager.OnColorsChangedListener { colors, which ->
            if (which and android.app.WallpaperManager.FLAG_SYSTEM != 0) {
                read(colors)?.let { seed = it }
            }
        }
        val handler = android.os.Handler(android.os.Looper.getMainLooper())
        runCatching { wm?.addOnColorsChangedListener(listener, handler) }
        onDispose { runCatching { wm?.removeOnColorsChangedListener(listener) } }
    }
    return seed
}

/**
 * Wallpaper colors: the system's Material You scheme when the device provides a real one,
 * otherwise a scheme built from the wallpaper's own primary color.
 */
@androidx.annotation.RequiresApi(Build.VERSION_CODES.S)
private fun wallpaperScheme(
    context: android.content.Context,
    dark: Boolean,
    error: Color,
    wallpaperSeed: UInt?,
): ColorScheme {
    val systemAccent = runCatching { context.getColor(android.R.color.system_accent1_500) }.getOrNull()
    val systemIsStock = systemAccent == null || systemAccent in STOCK_SYSTEM_ACCENTS
    if (systemIsStock && wallpaperSeed != null) {
        return schemeFromArtworkSeed(wallpaperSeed, dark, error)
    }
    return if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
}

/** Applies true #000000 OLED black surfaces and deep contrast containers when in dark + AMOLED mode. */
private fun ColorScheme.toAmoledScheme(glass: Boolean): ColorScheme = copy(
    background = Color.Black,
    surface = Color.Black,
    surfaceContainerLowest = Color.Black,
    surfaceContainerLow = Color(0xFF050608).copy(alpha = if (glass) 0.74f else 1f),
    surfaceContainer = Color(0xFF0A0B0E).copy(alpha = if (glass) 0.80f else 1f),
    surfaceContainerHigh = Color(0xFF111318).copy(alpha = if (glass) 0.90f else 1f),
    surfaceContainerHighest = Color(0xFF191C24).copy(alpha = if (glass) 0.95f else 1f),
    surfaceVariant = Color(0xFF12141A).copy(alpha = if (glass) 0.86f else 1f),
    outlineVariant = if (glass) Color.White.copy(alpha = 0.12f) else Color(0xFF22252E),
)

/** Shapes from the tokens: Glass and Bloom are smoothly rounded, Hive a little firmer, Graphite crisp. */
private fun shapes(graphite: Boolean, glass: Boolean, hive: Boolean) = when {
    hive -> Shapes(
        extraSmall = RoundedCornerShape(6.dp),
        small = RoundedCornerShape(10.dp),
        medium = RoundedCornerShape(14.dp),
        large = RoundedCornerShape(18.dp),
        extraLarge = RoundedCornerShape(24.dp),
    )
    graphite -> Shapes(
        extraSmall = RoundedCornerShape(2.dp),
        small = RoundedCornerShape(3.dp),
        medium = RoundedCornerShape(4.dp),
        large = RoundedCornerShape(4.dp),
        extraLarge = RoundedCornerShape(6.dp),
    )
    glass -> Shapes(
        extraSmall = RoundedCornerShape(10.dp),
        small = RoundedCornerShape(14.dp),
        medium = RoundedCornerShape(20.dp),
        large = RoundedCornerShape(24.dp),
        extraLarge = RoundedCornerShape(30.dp),
    )
    else -> Shapes(
        extraSmall = RoundedCornerShape(8.dp),
        small = RoundedCornerShape(12.dp),
        medium = RoundedCornerShape(16.dp),
        large = RoundedCornerShape(20.dp),
        extraLarge = RoundedCornerShape(28.dp),
    )
}

/** Frosted Switch colors when Glass mode is active, or standard Material 3 Switch colors otherwise. */
@Composable
fun nectarSwitchColors(): androidx.compose.material3.SwitchColors {
    val isGlass = LocalGlassTheme.current
    val scheme = MaterialTheme.colorScheme
    return if (isGlass) {
        androidx.compose.material3.SwitchDefaults.colors(
            checkedThumbColor = Color.White,
            checkedTrackColor = scheme.primary.copy(alpha = 0.62f),
            checkedBorderColor = scheme.outlineVariant,
            uncheckedThumbColor = scheme.onSurface.copy(alpha = 0.82f),
            uncheckedTrackColor = scheme.surfaceContainerHighest.copy(alpha = 0.42f),
            uncheckedBorderColor = scheme.outlineVariant,
        )
    } else {
        androidx.compose.material3.SwitchDefaults.colors()
    }
}

/**
 * The app's theme. `backdrop`: this is the app's main window, which draws the
 * theme's background itself (and Hive's comb on it); other screens keep a
 * plain opaque background.
 */
@Composable
fun NectarlinkTheme(
    appearance: Appearance = Appearance(),
    mediaSeedColor: UInt? = null,
    backdrop: Boolean = false,
    content: @Composable () -> Unit,
) {
    val dark = when (appearance.mode) {
        "light" -> false
        "dark" -> true
        else -> isSystemInDarkTheme()
    }
    val graphite = appearance.theme == "graphite"
    val glass = appearance.theme == "glass"
    // Hive keeps its own honey: no wallpaper, artwork or phone colors.
    val hive = appearance.theme == "hive"
    val isBlackSwatch = !appearance.dynamicColor && appearance.seed == "black"
    val amoledActive = dark && appearance.amoled && (graphite || isBlackSwatch)
    val error = when {
        hive -> if (dark) Tokens.hiveCombError else Tokens.hiveWaxError
        else -> if (dark) Tokens.errorDark else Tokens.errorLight
    }
    val context = LocalContext.current
    // The settings alone pick the color source. Artwork applies only when the user turned
    // "Media artwork color" on; with nothing playing, it falls back to the wallpaper or preset.
    val artworkSeed = if (!graphite && !hive && appearance.seed == "artwork") mediaSeedColor else null
    val wallpaper = !graphite && !hive && appearance.dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
    val wallpaperSeed = rememberWallpaperSeed(enabled = wallpaper && artworkSeed == null)
    val baseScheme = when {
        hive -> remember(dark, error) {
            (if (dark) Tokens.hiveComb else Tokens.hiveWax).toScheme(dark, error)
        }
        graphite -> remember(dark, error) {
            (if (dark) Tokens.graphiteSlate else Tokens.graphitePaper).toScheme(dark, error)
        }
        artworkSeed != null -> remember(artworkSeed, dark, error) {
            schemeFromArtworkSeed(artworkSeed, dark, error)
        }
        wallpaper && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> remember(context, dark, error, wallpaperSeed) {
            wallpaperScheme(context, dark, error, wallpaperSeed)
        }
        else -> remember(appearance.seed, dark, error) {
            val seed = Tokens.bloomSeeds[appearance.seed] ?: Tokens.bloomSeeds.getValue(Tokens.DEFAULT_SEED)
            (if (dark) seed.dark else seed.light).toScheme(dark, error)
        }
    }
    val combBehind = hive && backdrop
    val targetScheme = remember(baseScheme, glass, dark, amoledActive, combBehind) {
        val withGlass = if (glass) baseScheme.toGlassScheme(dark) else baseScheme
        val withAmoled = if (amoledActive) withGlass.toAmoledScheme(glass) else withGlass
        // Screens let the comb show through; the window paints the surface under it.
        if (combBehind) withAmoled.copy(background = Color.Transparent) else withAmoled
    }
    val reducedMotion = rememberReducedMotion()
    val powerSaver = rememberPowerSaver()

    // System bar icons follow the app's theme, which can differ from the system's.
    val view = LocalView.current
    if (!view.isInEditMode) {
        androidx.compose.runtime.LaunchedEffect(view, dark) {
            val window = (view.context as? Activity)?.window ?: return@LaunchedEffect
            WindowCompat.getInsetsController(window, view).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }
    }
    val assets = context.assets
    val fonts = remember(assets) { AppFonts(assets) }
    val currentShapes = remember(graphite, glass, hive) { shapes(graphite, glass, hive) }
    val currentTypography = remember(fonts, graphite) { typography(fonts, graphite) }
    CompositionLocalProvider(
        LocalAppFonts provides fonts,
        LocalReducedMotion provides reducedMotion,
        LocalGlassTheme provides glass,
        LocalAmoledTheme provides amoledActive,
        LocalBloomHomeCardPalette provides appearance.bloomHomeCardPalette(dark),
        LocalHiveTheme provides hive,
    ) {
        MaterialTheme(
            colorScheme = targetScheme,
            shapes = currentShapes,
            typography = currentTypography,
        ) {
            // The content stays in one place for every theme: calling it from
            // separate branches would rebuild the whole app on a switch to or
            // from Hive, losing state such as the selected tab.
            androidx.compose.foundation.layout.Box(
                androidx.compose.ui.Modifier.fillMaxSize().then(
                    if (combBehind) androidx.compose.ui.Modifier.background(baseScheme.surface) else androidx.compose.ui.Modifier,
                ),
            ) {
                if (combBehind) {
                    HiveComb(
                        dark = dark,
                        animate = appearance.combMotion && !reducedMotion && !powerSaver,
                    )
                }
                content()
            }
        }
    }
}

/**
 * Single-pass cached card background with a subtle uniform hairline border and no top-edge glare.
 */
fun Modifier.liquidGlassCard(
    cornerRadius: Dp = 24.dp,
    isGlass: Boolean,
    baseColor: Color,
    @Suppress("UNUSED_PARAMETER") tintColor: Color = Color.Transparent,
    outlineColor: Color = Color.Transparent,
): Modifier = this.drawWithCache {
    val radiusPx = cornerRadius.toPx()
    val cr = CornerRadius(radiusPx, radiusPx)
    val strokePx = 1.dp.toPx()
    val halfStroke = strokePx * 0.5f
    val topLeftOffset = Offset(halfStroke, halfStroke)
    val insetSize = Size(
        (size.width - strokePx).coerceAtLeast(0f),
        (size.height - strokePx).coerceAtLeast(0f),
    )
    val isDarkSurface = (baseColor.red * 0.299f + baseColor.green * 0.587f + baseColor.blue * 0.114f) < 0.5f
    val uniformRimColor = if (isGlass) {
        if (outlineColor.alpha > 0f) {
            outlineColor.copy(alpha = outlineColor.alpha.coerceAtMost(if (isDarkSurface) 0.14f else 0.22f))
        } else {
            Color.White.copy(alpha = if (isDarkSurface) 0.10f else 0.22f)
        }
    } else {
        outlineColor
    }
    val strokeStyle = Stroke(width = strokePx)
    onDrawBehind {
        drawRoundRect(color = baseColor, cornerRadius = cr)
        if (uniformRimColor.alpha > 0f) {
            drawRoundRect(
                color = uniformRimColor,
                topLeft = topLeftOffset,
                size = insetSize,
                cornerRadius = cr,
                style = strokeStyle,
            )
        }
    }
}

/**
 * No-op modifier kept for source compatibility; modal sheets use [glassModalScrimColor]
 * and [glassModalContainerColor] instead of blurring the entire root viewport into an FBO.
 */
fun Modifier.glassModalBlur(@Suppress("UNUSED_PARAMETER") modalOpen: Boolean): Modifier = this

/** High-legibility adaptive frosted container color for modal sheets and dialogs in Glass mode. */
@Composable
fun glassModalContainerColor(): Color {
    val isGlass = LocalGlassTheme.current
    val scheme = MaterialTheme.colorScheme
    if (!isGlass) return scheme.surfaceContainerHigh
    val luma = scheme.surface.red * 0.299f + scheme.surface.green * 0.587f + scheme.surface.blue * 0.114f
    val base = if (luma < 0.5f) Color(0xFF141720) else Color(0xFFF7F8FC)
    val tinted = Color(
        ColorUtils.blendARGB(
            android.graphics.Color.argb(255, (base.red * 255).toInt(), (base.green * 255).toInt(), (base.blue * 255).toInt()),
            android.graphics.Color.argb(255, (scheme.primary.red * 255).toInt(), (scheme.primary.green * 255).toInt(), (scheme.primary.blue * 255).toInt()),
            if (luma < 0.5f) 0.12f else 0.06f,
        ),
    )
    return tinted.copy(alpha = if (luma < 0.5f) 0.95f else 0.97f)
}

/** Deepened backdrop scrim color when sheets or dialogs open in Glass mode so background text never bleeds through. */
@Composable
fun glassModalScrimColor(): Color =
    if (LocalGlassTheme.current) Color.Black.copy(alpha = 0.62f) else Color.Black.copy(alpha = 0.38f)

/**
 * Lightweight zero-RenderNode interior well for bento strips and inner rows.
 */
fun Modifier.bentoWell(
    cornerRadius: Dp = 16.dp,
    fillColor: Color,
    borderColor: Color = Color.Transparent,
): Modifier = this.drawWithCache {
    val r = cornerRadius.toPx()
    val cr = CornerRadius(r, r)
    val strokePx = 1.dp.toPx()
    val halfStroke = strokePx * 0.5f
    val topLeftOffset = Offset(halfStroke, halfStroke)
    val insetSize = Size(
        (size.width - strokePx).coerceAtLeast(0f),
        (size.height - strokePx).coerceAtLeast(0f),
    )
    val strokeStyle = Stroke(width = strokePx)
    onDrawBehind {
        drawRoundRect(color = fillColor, cornerRadius = cr)
        if (borderColor.alpha > 0f) {
            drawRoundRect(
                color = borderColor,
                topLeft = topLeftOffset,
                size = insetSize,
                cornerRadius = cr,
                style = strokeStyle,
            )
        }
    }
}

