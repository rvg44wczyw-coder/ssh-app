package com.sshapp.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sshapp.ui.theme.*

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DiffViewerDialog(
    diffText: String,
    isLoading: Boolean,
    onRefresh: () -> Unit,
    onDismiss: () -> Unit
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = DarkBackground,
        dragHandle = { BottomSheetDefaults.DragHandle(color = DarkBorder) }
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .fillMaxHeight(0.85f)
        ) {
            // Header Bar
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    text = "Git Diff",
                    style = MaterialTheme.typography.titleMedium,
                    color = TextPrimary,
                    modifier = Modifier.weight(1f)
                )

                IconButton(onClick = onRefresh, enabled = !isLoading) {
                    Icon(Icons.Default.Refresh, contentDescription = "Refresh", tint = AccentBlue)
                }
                IconButton(onClick = onDismiss) {
                    Icon(Icons.Default.Close, contentDescription = "Close", tint = TextSecondary)
                }
            }

            HorizontalDivider(color = DarkBorder)

            // Content
            if (isLoading) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator(color = AccentBlue)
                }
            } else if (diffText.trim().isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            text = "Working tree clean",
                            color = TerminalGreen,
                            style = MaterialTheme.typography.titleSmall
                        )
                        Spacer(modifier = Modifier.height(4.dp))
                        Text(
                            text = "No uncommitted changes on remote host",
                            color = TextSecondary,
                            fontSize = 12.sp
                        )
                    }
                }
            } else {
                val lines = diffText.split("\n")
                LazyColumn(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(Color.Black)
                        .horizontalScroll(rememberScrollState())
                ) {
                    items(lines) { line ->
                        DiffLine(line)
                    }
                }
            }
        }
    }
}

@Composable
private fun DiffLine(line: String) {
    val (bgColor, fgColor) = when {
        line.startsWith("+++") || line.startsWith("---") -> Pair(Color.Gray.copy(alpha = 0.2f), TextPrimary)
        line.startsWith("+") -> Pair(TerminalGreen.copy(alpha = 0.18f), TerminalGreen)
        line.startsWith("-") -> Pair(DangerRed.copy(alpha = 0.18f), DangerRed)
        line.startsWith("@@") -> Pair(AccentBlue.copy(alpha = 0.15f), AccentBlue)
        line.startsWith("diff --git") -> Pair(WarningYellow.copy(alpha = 0.2f), WarningYellow)
        else -> Pair(Color.Transparent, TextPrimary)
    }

    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(bgColor)
            .padding(horizontal = 8.dp, vertical = 1.dp)
    ) {
        Text(
            text = if (line.isEmpty()) " " else line,
            fontSize = 11.sp,
            fontFamily = FontFamily.Monospace,
            color = fgColor
        )
    }
}
