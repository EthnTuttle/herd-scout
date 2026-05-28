package com.herdscout.admin

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val AdminOrange = Color(0xFFFF7A1A)
private val AdminOrangeContainer = Color(0xFFFFE0CC)
private val AdminCharcoal = Color(0xFF1A1A1A)
private val AdminCharcoalContainer = Color(0xFF2C2C2C)
private val AdminOnCharcoal = Color(0xFFF5F5F5)

private val AdminLightColors = lightColorScheme(
    primary = AdminOrange,
    onPrimary = Color.White,
    primaryContainer = AdminOrangeContainer,
    onPrimaryContainer = Color(0xFF3A1A00),
    secondary = AdminCharcoal,
    onSecondary = AdminOnCharcoal,
    background = Color(0xFFFAF8F6),
    onBackground = AdminCharcoal,
    surface = Color.White,
    onSurface = AdminCharcoal,
)

private val AdminDarkColors = darkColorScheme(
    primary = AdminOrange,
    onPrimary = AdminCharcoal,
    primaryContainer = AdminCharcoalContainer,
    onPrimaryContainer = AdminOrange,
    secondary = AdminOrange,
    onSecondary = AdminCharcoal,
    background = AdminCharcoal,
    onBackground = AdminOnCharcoal,
    surface = AdminCharcoalContainer,
    onSurface = AdminOnCharcoal,
)

@Composable
fun AdminTheme(content: @Composable () -> Unit) {
    val colors = if (isSystemInDarkTheme()) AdminDarkColors else AdminLightColors
    MaterialTheme(colorScheme = colors, content = content)
}
