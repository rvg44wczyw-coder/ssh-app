package com.sshapp.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sshapp.viewmodel.TerminalViewModel
import uniffi.ssh_core.AiCommandSuggestion
import uniffi.ssh_core.AiRiskLevel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AiAssistantDialog(
    viewModel: TerminalViewModel,
    onDismiss: () -> Unit
) {
    val context = LocalContext.current
    val isAiLoading by viewModel.isAiLoading.collectAsState()
    val aiError by viewModel.aiError.collectAsState()
    val activeSuggestion by viewModel.activeAiSuggestion.collectAsState()

    var userPrompt by remember { mutableStateOf("") }
    var selectedModel by remember { mutableStateOf("qwen2.5-coder:7b") }
    var showDestructiveConfirm by remember { mutableStateOf(false) }

    val availableModels = listOf(
        "qwen2.5-coder:7b",
        "llama3.2",
        "deepseek-coder",
        "mistral"
    )

    val quickPrompts = listOf(
        "Show open ports & listeners",
        "Find files modified today",
        "Disk usage by top directories",
        "Kill process on port 3000",
        "Revert last git commit keeping changes"
    )

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Color(0xFF131418),
        dragHandle = { BottomSheetDefaults.DragHandle(color = Color(0xFF4A4D54)) }
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            // Header Row
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Surface(
                    color = Color(0xFF9333EA).copy(alpha = 0.2f),
                    shape = RoundedCornerShape(10.dp),
                    modifier = Modifier.size(40.dp)
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            imageVector = Icons.Default.AutoAwesome,
                            contentDescription = "AI",
                            tint = Color(0xFFC084FC),
                            modifier = Modifier.size(22.dp)
                        )
                    }
                }
                Spacer(modifier = Modifier.width(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = "AI Shell Assistant",
                        fontSize = 17.sp,
                        fontWeight = FontWeight.Bold,
                        color = Color.White
                    )
                    Text(
                        text = "Generate shell commands via Host Ollama",
                        fontSize = 12.sp,
                        color = Color.Gray
                    )
                }
                IconButton(onClick = onDismiss) {
                    Icon(
                        imageVector = Icons.Default.Close,
                        contentDescription = "Close",
                        tint = Color.Gray
                    )
                }
            }

            // Model Selection Chips
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    text = "HOST MODEL",
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    color = Color.Gray
                )
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    availableModels.forEach { model ->
                        val isSelected = selectedModel == model
                        FilterChip(
                            selected = isSelected,
                            onClick = { selectedModel = model },
                            label = {
                                Text(
                                    text = model,
                                    fontSize = 12.sp,
                                    fontFamily = FontFamily.Monospace
                                )
                            },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = Color(0xFF9333EA).copy(alpha = 0.3f),
                                selectedLabelColor = Color(0xFFC084FC)
                            )
                        )
                    }
                }
            }

            // User Prompt Input
            OutlinedTextField(
                value = userPrompt,
                onValueChange = { userPrompt = it },
                placeholder = {
                    Text(
                        text = "Describe what you want to achieve...",
                        fontSize = 14.sp,
                        color = Color.Gray
                    )
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 90.dp),
                shape = RoundedCornerShape(10.dp),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedContainerColor = Color(0xFF1E2026),
                    unfocusedContainerColor = Color(0xFF1E2026),
                    focusedBorderColor = Color(0xFF9333EA),
                    unfocusedBorderColor = Color(0xFF32353E),
                    focusedTextColor = Color.White,
                    unfocusedTextColor = Color.White
                )
            )

            // Generate Button
            Button(
                onClick = {
                    viewModel.queryAiAssistant(
                        userPrompt = userPrompt,
                        modelName = selectedModel,
                        targetOs = "macOS"
                    )
                },
                enabled = userPrompt.isNotBlank() && !isAiLoading,
                colors = ButtonDefaults.buttonColors(
                    containerColor = Color(0xFF9333EA),
                    contentColor = Color.White,
                    disabledContainerColor = Color(0xFF9333EA).copy(alpha = 0.3f)
                ),
                shape = RoundedCornerShape(10.dp),
                modifier = Modifier.fillMaxWidth().height(44.dp)
            ) {
                if (isAiLoading) {
                    CircularProgressIndicator(
                        color = Color.White,
                        modifier = Modifier.size(20.dp),
                        strokeWidth = 2.dp
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(text = "Consulting Host Ollama...", fontWeight = FontWeight.SemiBold)
                } else {
                    Icon(
                        imageVector = Icons.Default.AutoAwesome,
                        contentDescription = "Generate",
                        modifier = Modifier.size(18.dp)
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(text = "Generate Command", fontWeight = FontWeight.SemiBold)
                }
            }

            // Quick Prompt Chips
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    text = "SUGGESTIONS",
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    color = Color.Gray
                )
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    quickPrompts.forEach { prompt ->
                        Surface(
                            onClick = { userPrompt = prompt },
                            color = Color(0xFF22242B),
                            shape = RoundedCornerShape(12.dp)
                        ) {
                            Text(
                                text = prompt,
                                fontSize = 11.sp,
                                color = Color.White.copy(alpha = 0.85f),
                                modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp)
                            )
                        }
                    }
                }
            }

            // Error Display
            aiError?.let { err ->
                Surface(
                    color = Color(0xFFDC2626).copy(alpha = 0.15f),
                    shape = RoundedCornerShape(8.dp),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Row(
                        modifier = Modifier.padding(12.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Icon(
                            imageVector = Icons.Default.Warning,
                            contentDescription = "Error",
                            tint = Color(0xFFF87171),
                            modifier = Modifier.size(18.dp)
                        )
                        Spacer(modifier = Modifier.width(8.dp))
                        Text(
                            text = err,
                            fontSize = 12.sp,
                            color = Color(0xFFF87171)
                        )
                    }
                }
            }

            // Suggestion Card
            activeSuggestion?.let { suggestion ->
                AiSuggestionCard(
                    suggestion = suggestion,
                    onCopy = {
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        clipboard.setPrimaryClip(ClipData.newPlainText("Shell Command", suggestion.command))
                        Toast.makeText(context, "Command copied", Toast.LENGTH_SHORT).show()
                    },
                    onInsert = {
                        viewModel.insertAiCommand(suggestion.command)
                        onDismiss()
                    },
                    onRun = {
                        if (suggestion.riskLevel == AiRiskLevel.DESTRUCTIVE) {
                            showDestructiveConfirm = true
                        } else {
                            viewModel.executeAiCommand(suggestion.command)
                            onDismiss()
                        }
                    }
                )
            }
        }
    }

    if (showDestructiveConfirm) {
        AlertDialog(
            onDismissRequest = { showDestructiveConfirm = false },
            title = { Text(text = "Destructive Command", color = Color.White) },
            text = {
                Text(
                    text = "This command contains potentially destructive operations (e.g. recursive file deletion or hard reset). Are you sure you want to run it immediately?",
                    color = Color.LightGray
                )
            },
            confirmButton = {
                Button(
                    onClick = {
                        activeSuggestion?.let { viewModel.executeAiCommand(it.command) }
                        showDestructiveConfirm = false
                        onDismiss()
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = Color(0xFFDC2626))
                ) {
                    Text("Run Anyway")
                }
            },
            dismissButton = {
                TextButton(onClick = { showDestructiveConfirm = false }) {
                    Text("Cancel")
                }
            },
            containerColor = Color(0xFF1E2026)
        )
    }
}

@Composable
private fun AiSuggestionCard(
    suggestion: AiCommandSuggestion,
    onCopy: () -> Unit,
    onInsert: () -> Unit,
    onRun: () -> Unit
) {
    Surface(
        color = Color(0xFF1A1C22),
        shape = RoundedCornerShape(12.dp),
        border = CardDefaults.outlinedCardBorder().copy(brush = androidx.compose.ui.graphics.SolidColor(Color(0xFF2E313C))),
        modifier = Modifier.fillMaxWidth()
    ) {
        Column(
            modifier = Modifier.padding(14.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            // Risk Level Badge & Copy Header
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                RiskBadge(level = suggestion.riskLevel)
                Spacer(modifier = Modifier.weight(1f))
                IconButton(onClick = onCopy, modifier = Modifier.size(32.dp)) {
                    Icon(
                        imageVector = Icons.Default.ContentCopy,
                        contentDescription = "Copy",
                        tint = Color.Gray,
                        modifier = Modifier.size(16.dp)
                    )
                }
            }

            // Warnings if any
            if (suggestion.warnings.isNotEmpty()) {
                Surface(
                    color = Color(0xFFD97706).copy(alpha = 0.15f),
                    shape = RoundedCornerShape(6.dp),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Column(
                        modifier = Modifier.padding(8.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp)
                    ) {
                        suggestion.warnings.forEach { warn ->
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Icon(
                                    imageVector = Icons.Default.Warning,
                                    contentDescription = "Warning",
                                    tint = Color(0xFFFBBF24),
                                    modifier = Modifier.size(14.dp)
                                )
                                Spacer(modifier = Modifier.width(6.dp))
                                Text(
                                    text = warn,
                                    fontSize = 11.sp,
                                    color = Color(0xFFFBBF24)
                                )
                            }
                        }
                    }
                }
            }

            // Command Code Box
            Surface(
                color = Color(0xFF0F1014),
                shape = RoundedCornerShape(8.dp),
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    text = suggestion.command,
                    fontFamily = FontFamily.Monospace,
                    fontSize = 13.sp,
                    color = Color(0xFF4ADE80),
                    modifier = Modifier.padding(12.dp)
                )
            }

            // Explanation
            Text(
                text = suggestion.explanation,
                fontSize = 12.sp,
                color = Color.LightGray,
                lineHeight = 16.sp
            )

            // Action Buttons Row
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                OutlinedButton(
                    onClick = onInsert,
                    shape = RoundedCornerShape(8.dp),
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = Color.White),
                    modifier = Modifier.weight(1f).height(38.dp)
                ) {
                    Text(text = "Insert", fontSize = 13.sp, fontWeight = FontWeight.Bold)
                }

                Button(
                    onClick = onRun,
                    shape = RoundedCornerShape(8.dp),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = if (suggestion.riskLevel == AiRiskLevel.DESTRUCTIVE) Color(0xFFDC2626) else Color(0xFF2563EB),
                        contentColor = Color.White
                    ),
                    modifier = Modifier.weight(1f).height(38.dp)
                ) {
                    Icon(
                        imageVector = Icons.Default.PlayArrow,
                        contentDescription = "Run",
                        modifier = Modifier.size(16.dp)
                    )
                    Spacer(modifier = Modifier.width(4.dp))
                    Text(text = "Run ⏎", fontSize = 13.sp, fontWeight = FontWeight.Bold)
                }
            }
        }
    }
}

@Composable
private fun RiskBadge(level: AiRiskLevel) {
    val (label, bgColor, textColor) = when (level) {
        AiRiskLevel.SAFE -> Triple("🟢 Safe", Color(0xFF15803D).copy(alpha = 0.2f), Color(0xFF4ADE80))
        AiRiskLevel.CAUTION -> Triple("🟡 Caution", Color(0xFFB45309).copy(alpha = 0.2f), Color(0xFFFBBF24))
        AiRiskLevel.ELEVATED -> Triple("🟠 Elevated", Color(0xFFC2410C).copy(alpha = 0.2f), Color(0xFFFB923C))
        AiRiskLevel.DESTRUCTIVE -> Triple("🔴 Destructive", Color(0xFFB91C1C).copy(alpha = 0.2f), Color(0xFFF87171))
    }

    Surface(
        color = bgColor,
        shape = RoundedCornerShape(6.dp)
    ) {
        Text(
            text = label,
            fontSize = 11.sp,
            fontWeight = FontWeight.Bold,
            color = textColor,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp)
        )
    }
}
