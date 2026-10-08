package com.sshapp.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sshapp.ui.theme.DarkBorder
import com.sshapp.ui.theme.DarkSurface
import com.sshapp.ui.theme.TerminalGreen
import com.sshapp.ui.theme.DangerRed

@Composable
fun KeyboardAccessoryRow(
    onKeyClick: (String) -> Unit,
    onAiClick: (() -> Unit)? = null,
    modifier: Modifier = Modifier
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        color = DarkSurface,
        shadowElevation = 4.dp
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 6.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp)
        ) {
            // AI Shell Assistant Button
            if (onAiClick != null) {
                Button(
                    onClick = onAiClick,
                    colors = ButtonDefaults.buttonColors(
                        containerColor = Color(0xFF9333EA).copy(alpha = 0.25f),
                        contentColor = Color(0xFFC084FC)
                    ),
                    shape = RoundedCornerShape(6.dp),
                    contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                    modifier = Modifier.height(34.dp)
                ) {
                    Text(
                        text = "🪄 AI",
                        fontSize = 12.sp,
                        fontWeight = FontWeight.Bold,
                        fontFamily = FontFamily.Monospace
                    )
                }
            }

            // Quick Approval Action: ✓ y⏎
            Button(
                onClick = { onKeyClick("y") },
                colors = ButtonDefaults.buttonColors(
                    containerColor = TerminalGreen.copy(alpha = 0.2f),
                    contentColor = TerminalGreen
                ),
                shape = RoundedCornerShape(6.dp),
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                modifier = Modifier.height(34.dp)
            ) {
                Text(
                    text = "✓ y⏎",
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold,
                    fontFamily = FontFamily.Monospace
                )
            }

            // Quick Deny Action: ✗ n⏎
            Button(
                onClick = { onKeyClick("n") },
                colors = ButtonDefaults.buttonColors(
                    containerColor = DangerRed.copy(alpha = 0.2f),
                    contentColor = DangerRed
                ),
                shape = RoundedCornerShape(6.dp),
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                modifier = Modifier.height(34.dp)
            ) {
                Text(
                    text = "✗ n⏎",
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold,
                    fontFamily = FontFamily.Monospace
                )
            }

            // Standard Terminal Keys
            AccessoryKeyButton("Esc") { onKeyClick("esc") }
            AccessoryKeyButton("Tab") { onKeyClick("tab") }
            AccessoryKeyButton("Ctrl+C") { onKeyClick("ctrl_c") }

            // Terminal & Tmux Scroll Navigation
            AccessoryKeyButton("📜 Scroll") { onKeyClick("tmux_scroll") }
            AccessoryKeyButton("PgUp") { onKeyClick("pgup") }
            AccessoryKeyButton("PgDn") { onKeyClick("pgdn") }
            AccessoryKeyButton("q") { onKeyClick("q") }

            AccessoryKeyButton("|") { onKeyClick("pipe") }
            AccessoryKeyButton("/") { onKeyClick("slash") }
            AccessoryKeyButton("▲") { onKeyClick("up") }
            AccessoryKeyButton("▼") { onKeyClick("down") }
        }
    }
}

@Composable
private fun AccessoryKeyButton(
    label: String,
    onClick: () -> Unit
) {
    OutlinedButton(
        onClick = onClick,
        colors = ButtonDefaults.outlinedButtonColors(
            contentColor = Color.White
        ),
        shape = RoundedCornerShape(6.dp),
        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
        modifier = Modifier.height(34.dp)
    ) {
        Text(
            text = label,
            fontSize = 12.sp,
            fontWeight = FontWeight.Medium,
            fontFamily = FontFamily.Monospace
        )
    }
}
