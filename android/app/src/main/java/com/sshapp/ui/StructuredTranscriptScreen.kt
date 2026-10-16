package com.sshapp.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sshapp.ui.theme.*
import com.sshapp.viewmodel.TerminalViewModel
import uniffi.ssh_core.*

@Composable
fun StructuredTranscriptScreen(
    viewModel: TerminalViewModel,
    modifier: Modifier = Modifier
) {
    val transcriptTurns by viewModel.transcriptTurns.collectAsState()
    val isTranscriptLoading by viewModel.isTranscriptLoading.collectAsState()
    val activePath by viewModel.activeTranscriptPath.collectAsState()
    val transcriptError by viewModel.transcriptError.collectAsState()

    var promptInput by remember { mutableStateOf("") }
    val listState = rememberLazyListState()

    LaunchedEffect(transcriptTurns.size) {
        if (transcriptTurns.isNotEmpty()) {
            listState.animateScrollToItem(transcriptTurns.size - 1)
        }
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(DarkBackground)
    ) {
        // Path banner & Refresh row
        Surface(
            color = DarkSurface,
            modifier = Modifier.fillMaxWidth()
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Icon(
                    imageVector = Icons.Default.Description,
                    contentDescription = null,
                    tint = AccentBlue,
                    modifier = Modifier.size(14.dp)
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = activePath ?: "Detecting active transcript...",
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace,
                    color = TextSecondary,
                    maxLines = 1,
                    modifier = Modifier.weight(1f)
                )

                if (isTranscriptLoading) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = AccentBlue
                    )
                } else {
                    IconButton(
                        onClick = { viewModel.loadTranscript() },
                        modifier = Modifier.size(24.dp)
                    ) {
                        Icon(
                            imageVector = Icons.Default.Refresh,
                            contentDescription = "Refresh",
                            tint = AccentBlue,
                            modifier = Modifier.size(16.dp)
                        )
                    }
                }
            }
        }

        HorizontalDivider(color = DarkBorder)

        // Main List or Empty State
        Box(
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
        ) {
            if (transcriptTurns.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    if (isTranscriptLoading) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            CircularProgressIndicator(color = AccentBlue)
                            Spacer(modifier = Modifier.height(12.dp))
                            Text(
                                text = "Reading agent transcript from host...",
                                color = TextSecondary,
                                fontSize = 13.sp
                            )
                        }
                    } else {
                        Column(
                            horizontalAlignment = Alignment.CenterHorizontally,
                            modifier = Modifier.padding(24.dp)
                        ) {
                            Icon(
                                imageVector = Icons.Default.Forum,
                                contentDescription = null,
                                tint = TextSecondary,
                                modifier = Modifier.size(48.dp)
                            )
                            Spacer(modifier = Modifier.height(12.dp))
                            Text(
                                text = "No Active Agent Transcript",
                                color = TextPrimary,
                                fontWeight = FontWeight.Bold,
                                fontSize = 16.sp
                            )
                            Spacer(modifier = Modifier.height(6.dp))
                            Text(
                                text = transcriptError ?: "Transcripts from Claude Code or Google Antigravity sessions will appear here.",
                                color = TextSecondary,
                                fontSize = 12.sp,
                                modifier = Modifier.padding(horizontal = 16.dp)
                            )
                            Spacer(modifier = Modifier.height(16.dp))
                            Button(
                                onClick = { viewModel.loadTranscript() },
                                colors = ButtonDefaults.buttonColors(containerColor = AccentBlue)
                            ) {
                                Text("Detect Active Transcript", fontSize = 13.sp)
                            }
                        }
                    }
                }
            } else {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(12.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    items(transcriptTurns, key = { it.id }) { turn ->
                        TranscriptTurnCard(turn = turn)
                    }
                }
            }
        }

        HorizontalDivider(color = DarkBorder)

        // Bottom Prompt Input Bar
        Surface(
            color = DarkSurface,
            modifier = Modifier.fillMaxWidth()
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                OutlinedTextField(
                    value = promptInput,
                    onValueChange = { promptInput = it },
                    placeholder = { Text("Send instruction to agent...", fontSize = 12.sp, color = TextSecondary) },
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor = AccentBlue,
                        unfocusedBorderColor = DarkBorder,
                        focusedTextColor = TextPrimary,
                        unfocusedTextColor = TextPrimary
                    )
                )
                Spacer(modifier = Modifier.width(8.dp))
                IconButton(
                    onClick = {
                        val text = promptInput.trim()
                        if (text.isNotEmpty()) {
                            viewModel.sendTerminalInput("$text\n")
                            promptInput = ""
                        }
                    },
                    enabled = promptInput.isNotBlank()
                ) {
                    Icon(
                        imageVector = Icons.Default.Send,
                        contentDescription = "Send",
                        tint = if (promptInput.isNotBlank()) AccentBlue else TextSecondary
                    )
                }
            }
        }
    }
}

@Composable
private fun TranscriptTurnCard(turn: TranscriptTurn) {
    var isThinkingExpanded by remember { mutableStateOf(false) }

    val isUser = turn.turnType == TranscriptTurnType.USER
    val cardBackground = if (isUser) Color(0xFF1E293B) else DarkSurface
    val cardBorder = if (isUser) AccentBlue.copy(alpha = 0.3f) else DarkBorder

    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(10.dp),
        colors = CardDefaults.cardColors(containerColor = cardBackground),
        border = androidx.compose.foundation.BorderStroke(1.dp, cardBorder)
    ) {
        Column(modifier = Modifier.padding(10.dp)) {
            // Header Row: Role & Status
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                when (turn.turnType) {
                    TranscriptTurnType.USER -> {
                        Icon(
                            imageVector = Icons.Default.Person,
                            contentDescription = null,
                            tint = AccentBlue,
                            modifier = Modifier.size(16.dp)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = "User",
                            fontWeight = FontWeight.Bold,
                            color = AccentBlue,
                            fontSize = 12.sp
                        )
                    }
                    TranscriptTurnType.ASSISTANT -> {
                        Icon(
                            imageVector = Icons.Default.AutoAwesome,
                            contentDescription = null,
                            tint = Color(0xFFC084FC),
                            modifier = Modifier.size(16.dp)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = "Agent",
                            fontWeight = FontWeight.Bold,
                            color = Color(0xFFC084FC),
                            fontSize = 12.sp
                        )
                    }
                    TranscriptTurnType.APPROVAL_REQUEST -> {
                        Icon(
                            imageVector = Icons.Default.Security,
                            contentDescription = null,
                            tint = WarningYellow,
                            modifier = Modifier.size(16.dp)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = "Approval Request",
                            fontWeight = FontWeight.Bold,
                            color = WarningYellow,
                            fontSize = 12.sp
                        )
                    }
                    TranscriptTurnType.SYSTEM -> {
                        Icon(
                            imageVector = Icons.Default.Settings,
                            contentDescription = null,
                            tint = TextSecondary,
                            modifier = Modifier.size(16.dp)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = "System",
                            fontWeight = FontWeight.Bold,
                            color = TextSecondary,
                            fontSize = 12.sp
                        )
                    }
                }

                Spacer(modifier = Modifier.weight(1f))

                turn.timestamp?.let { time ->
                    Text(
                        text = time.substringAfter("T").take(8),
                        fontSize = 10.sp,
                        color = TextSecondary
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                }

                when (turn.status) {
                    TranscriptStatus.RUNNING -> CircularProgressIndicator(
                        modifier = Modifier.size(12.dp),
                        strokeWidth = 2.dp,
                        color = AccentBlue
                    )
                    TranscriptStatus.DONE -> Icon(
                        imageVector = Icons.Default.CheckCircle,
                        contentDescription = "Done",
                        tint = TerminalGreen,
                        modifier = Modifier.size(14.dp)
                    )
                    TranscriptStatus.ERROR -> Icon(
                        imageVector = Icons.Default.Error,
                        contentDescription = "Error",
                        tint = DangerRed,
                        modifier = Modifier.size(14.dp)
                    )
                }
            }

            // Thinking Accordion
            turn.thinking?.let { thinking ->
                if (thinking.isNotBlank()) {
                    Spacer(modifier = Modifier.height(6.dp))
                    Surface(
                        color = Color.Black.copy(alpha = 0.25f),
                        shape = RoundedCornerShape(6.dp),
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { isThinkingExpanded = !isThinkingExpanded }
                    ) {
                        Column(modifier = Modifier.padding(8.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Icon(
                                    imageVector = Icons.Default.Psychology,
                                    contentDescription = null,
                                    tint = TextSecondary,
                                    modifier = Modifier.size(14.dp)
                                )
                                Spacer(modifier = Modifier.width(4.dp))
                                Text(
                                    text = "Thinking (${thinking.lines().size} steps)",
                                    fontSize = 11.sp,
                                    color = TextSecondary,
                                    modifier = Modifier.weight(1f)
                                )
                                Icon(
                                    imageVector = if (isThinkingExpanded) Icons.Default.ExpandLess else Icons.Default.ExpandMore,
                                    contentDescription = null,
                                    tint = TextSecondary,
                                    modifier = Modifier.size(16.dp)
                                )
                            }
                            if (isThinkingExpanded) {
                                Spacer(modifier = Modifier.height(6.dp))
                                Text(
                                    text = thinking,
                                    fontFamily = FontFamily.Monospace,
                                    fontSize = 11.sp,
                                    color = TextSecondary
                                )
                            }
                        }
                    }
                }
            }

            // Content text
            turn.content?.let { content ->
                if (content.isNotBlank()) {
                    Spacer(modifier = Modifier.height(6.dp))
                    Text(
                        text = content,
                        fontSize = 13.sp,
                        color = TextPrimary
                    )
                }
            }

            // Tool Calls
            if (turn.toolCalls.isNotEmpty()) {
                Spacer(modifier = Modifier.height(6.dp))
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    turn.toolCalls.forEach { toolCall ->
                        ToolCallCard(toolCall = toolCall)
                    }
                }
            }
        }
    }
}

@Composable
private fun ToolCallCard(toolCall: TranscriptToolCall) {
    var isExpanded by remember { mutableStateOf(false) }

    Surface(
        color = Color.Black.copy(alpha = 0.35f),
        shape = RoundedCornerShape(6.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clickable { isExpanded = !isExpanded }
    ) {
        Column(modifier = Modifier.padding(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    imageVector = when (toolCall.toolName.lowercase()) {
                        "run_command", "bash" -> Icons.Default.Terminal
                        "view_file", "read_file" -> Icons.Default.Description
                        "write_file", "write_to_file" -> Icons.Default.Edit
                        "search_web" -> Icons.Default.Language
                        else -> Icons.Default.Build
                    },
                    contentDescription = null,
                    tint = when (toolCall.toolName.lowercase()) {
                        "run_command", "bash" -> Color(0xFFF97316)
                        "write_file", "write_to_file" -> Color(0xFFC084FC)
                        "view_file", "read_file" -> AccentBlue
                        else -> TextSecondary
                    },
                    modifier = Modifier.size(14.dp)
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = toolCall.toolName,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    fontSize = 11.sp,
                    color = TextPrimary
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = toolCall.argumentsSummary,
                    fontFamily = FontFamily.Monospace,
                    fontSize = 11.sp,
                    color = TextSecondary,
                    maxLines = 1,
                    modifier = Modifier.weight(1f)
                )

                Spacer(modifier = Modifier.width(6.dp))
                when (toolCall.status) {
                    TranscriptStatus.RUNNING -> CircularProgressIndicator(
                        modifier = Modifier.size(10.dp),
                        strokeWidth = 1.5.dp,
                        color = AccentBlue
                    )
                    TranscriptStatus.DONE -> Icon(
                        imageVector = Icons.Default.Check,
                        contentDescription = "Success",
                        tint = TerminalGreen,
                        modifier = Modifier.size(12.dp)
                    )
                    TranscriptStatus.ERROR -> Icon(
                        imageVector = Icons.Default.Close,
                        contentDescription = "Failed",
                        tint = DangerRed,
                        modifier = Modifier.size(12.dp)
                    )
                }

                Icon(
                    imageVector = if (isExpanded) Icons.Default.ExpandLess else Icons.Default.ExpandMore,
                    contentDescription = null,
                    tint = TextSecondary,
                    modifier = Modifier.size(16.dp)
                )
            }

            AnimatedVisibility(visible = isExpanded) {
                Column(modifier = Modifier.padding(top = 8.dp)) {
                    toolCall.fullArgumentsJson?.let { args ->
                        Text(
                            text = "Arguments:",
                            fontSize = 10.sp,
                            fontWeight = FontWeight.Bold,
                            color = TextSecondary
                        )
                        Spacer(modifier = Modifier.height(2.dp))
                        Surface(
                            color = DarkBackground,
                            shape = RoundedCornerShape(4.dp),
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            Text(
                                text = args,
                                fontFamily = FontFamily.Monospace,
                                fontSize = 10.sp,
                                color = TextPrimary,
                                modifier = Modifier.padding(6.dp)
                            )
                        }
                    }

                    toolCall.result?.let { result ->
                        Spacer(modifier = Modifier.height(6.dp))
                        Text(
                            text = "Output:",
                            fontSize = 10.sp,
                            fontWeight = FontWeight.Bold,
                            color = if (toolCall.isError) DangerRed else TextSecondary
                        )
                        Spacer(modifier = Modifier.height(2.dp))
                        Surface(
                            color = DarkBackground,
                            shape = RoundedCornerShape(4.dp),
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            Text(
                                text = result,
                                fontFamily = FontFamily.Monospace,
                                fontSize = 10.sp,
                                color = if (toolCall.isError) DangerRed else TextPrimary,
                                modifier = Modifier.padding(6.dp)
                            )
                        }
                    }
                }
            }
        }
    }
}
