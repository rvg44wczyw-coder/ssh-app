package com.sshapp.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sshapp.ui.theme.AccentBlue
import com.sshapp.ui.theme.DarkSurface
import com.sshapp.ui.theme.TextPrimary
import kotlinx.coroutines.launch

@Composable
fun TerminalView(
    output: String,
    modifier: Modifier = Modifier
) {
    val coroutineScope = rememberCoroutineScope()
    val verticalScrollState = rememberScrollState()
    val horizontalScrollState = rememberScrollState()

    // Determine if the user is currently at the bottom of the scroll buffer
    val isNearBottom by remember {
        derivedStateOf {
            verticalScrollState.maxValue == 0 ||
            verticalScrollState.value >= (verticalScrollState.maxValue - 100)
        }
    }

    // Auto-scroll to bottom only if user hasn't deliberately scrolled up to read history
    LaunchedEffect(output) {
        if (isNearBottom) {
            verticalScrollState.animateScrollTo(verticalScrollState.maxValue)
        }
    }

    Box(
        modifier = modifier
            .fillMaxSize()
            .background(Color.Black)
    ) {
        // Dual-axis scrollable terminal buffer (Vertical + Horizontal)
        SelectionContainer(modifier = Modifier.fillMaxSize()) {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(verticalScrollState)
                    .horizontalScroll(horizontalScrollState)
                    .padding(8.dp)
            ) {
                Text(
                    text = output.ifEmpty { "Session initialized. Waiting for remote tmux shell...\n" },
                    color = TextPrimary,
                    fontFamily = FontFamily.Monospace,
                    fontSize = 12.sp,
                    lineHeight = 16.sp,
                    softWrap = false // Keep line formatting and allow horizontal scroll
                )
            }
        }

        // Floating "Jump to Bottom" button when user scrolls up into history
        AnimatedVisibility(
            visible = !isNearBottom,
            enter = fadeIn(),
            exit = fadeOut(),
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(16.dp)
        ) {
            FloatingActionButton(
                onClick = {
                    coroutineScope.launch {
                        verticalScrollState.animateScrollTo(verticalScrollState.maxValue)
                    }
                },
                containerColor = DarkSurface.copy(alpha = 0.85f),
                contentColor = AccentBlue,
                shape = CircleShape,
                modifier = Modifier.size(38.dp)
            ) {
                Icon(
                    imageVector = Icons.Default.KeyboardArrowDown,
                    contentDescription = "Jump to Bottom",
                    modifier = Modifier.size(20.dp)
                )
            }
        }
    }
}
