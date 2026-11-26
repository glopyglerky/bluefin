package org.bluefin.android

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val MoonlitLight = lightColorScheme(
    primary = Color(0xFF333B5A), onPrimary = Color(0xFFCCD0D3),
    primaryContainer = Color(0xFFCCD0D3), onPrimaryContainer = Color(0xFF061727),
    secondary = Color(0xFF061727), onSecondary = Color(0xFFCCD0D3),
    secondaryContainer = Color(0xFFBDC5C9), onSecondaryContainer = Color(0xFF061727),
    tertiary = Color(0xFF333B5A), onTertiary = Color(0xFFCCD0D3),
    tertiaryContainer = Color(0xFFCCD0D3), onTertiaryContainer = Color(0xFF061727),
    background = Color(0xFFA3ACB1), onBackground = Color(0xFF061727),
    surface = Color(0xFFCCD0D3), onSurface = Color(0xFF061727),
    surfaceVariant = Color(0xFFBDC5C9), onSurfaceVariant = Color(0xFF2B4050),
    outline = Color(0xFF465966), outlineVariant = Color(0xFF8A989F),
    inverseSurface = Color(0xFF102435), inverseOnSurface = Color(0xFFCCD0D3),
    inversePrimary = Color(0xFFADB5D1), surfaceTint = Color.Transparent,
    surfaceBright = Color(0xFFCCD0D3), surfaceDim = Color(0xFFA3ACB1),
    surfaceContainerLowest = Color(0xFFA3ACB1), surfaceContainerLow = Color(0xFFCCD0D3),
    surfaceContainer = Color(0xFFCCD0D3), surfaceContainerHigh = Color(0xFFBDC5C9),
    surfaceContainerHighest = Color(0xFFBDC5C9),
)
private val MoonlitDark = darkColorScheme(
    primary = Color(0xFFADB5D1), onPrimary = Color(0xFF061727),
    primaryContainer = Color(0xFF102435), onPrimaryContainer = Color(0xFFCCD0D3),
    secondary = Color(0xFF8289A5), onSecondary = Color(0xFF061727),
    secondaryContainer = Color(0xFF182D3D), onSecondaryContainer = Color(0xFFCCD0D3),
    tertiary = Color(0xFFADB5D1), onTertiary = Color(0xFF061727),
    tertiaryContainer = Color(0xFF102435), onTertiaryContainer = Color(0xFFCCD0D3),
    background = Color(0xFF061727), onBackground = Color(0xFFCCD0D3),
    surface = Color(0xFF102435), onSurface = Color(0xFFCCD0D3),
    surfaceVariant = Color(0xFF182D3D), onSurfaceVariant = Color(0xFFA3ACB1),
    outline = Color(0xFF637889), outlineVariant = Color(0xFF2D4354),
    inverseSurface = Color(0xFFCCD0D3), inverseOnSurface = Color(0xFF061727),
    inversePrimary = Color(0xFF333B5A), surfaceTint = Color.Transparent,
    surfaceBright = Color(0xFF182D3D), surfaceDim = Color(0xFF061727),
    surfaceContainerLowest = Color(0xFF061727), surfaceContainerLow = Color(0xFF102435),
    surfaceContainer = Color(0xFF102435), surfaceContainerHigh = Color(0xFF182D3D),
    surfaceContainerHighest = Color(0xFF182D3D),
)

@Composable
internal fun bluefinColorScheme(): ColorScheme =
    if (isSystemInDarkTheme()) MoonlitDark else MoonlitLight
