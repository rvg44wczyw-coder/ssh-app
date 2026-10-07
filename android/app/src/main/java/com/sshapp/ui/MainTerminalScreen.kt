package com.sshapp.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.DataObject
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Terminal
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
import com.sshapp.service.AndroidKeyManager
import com.sshapp.ui.theme.*
import com.sshapp.viewmodel.ConnectionUiState
import com.sshapp.viewmodel.TerminalViewModel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainTerminalScreen(
    viewModel: TerminalViewModel
) {
    val context = LocalContext.current
    val keyManager = remember { AndroidKeyManager.getInstance(context) }

    val tabs by viewModel.tabs.collectAsState()
    val selectedTabId by viewModel.selectedTabId.collectAsState()
    val serverProfile by viewModel.serverProfile.collectAsState()
    val connectionState by viewModel.connectionState.collectAsState()
    val terminalOutput by viewModel.terminalOutput.collectAsState()
    val pendingApproval by viewModel.pendingApproval.collectAsState()
    val isDiffSheetOpen by viewModel.isDiffSheetOpen.collectAsState()
    val diffText by viewModel.diffText.collectAsState()
    val isDiffLoading by viewModel.isDiffLoading.collectAsState()

    var showKeyBanner by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(
                            imageVector = Icons.Default.Terminal,
                            contentDescription = "Terminal",
                            tint = AccentBlue,
                            modifier = Modifier.size(18.dp)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = viewModel.activeTab?.title ?: "Terminal",
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.Bold,
                            color = TextPrimary
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = "• ${serverProfile.name}",
                            fontSize = 11.sp,
                            color = TextSecondary
                        )
                    }
                },
                actions = {
                    // Git Diff Viewer Button
                    IconButton(onClick = { viewModel.openDiffViewer() }) {
                        Icon(
                            imageVector = Icons.Default.DataObject,
                            contentDescription = "Git Diff",
                            tint = AccentBlue
                        )
                    }
                    // Public Key Toggle Button
                    IconButton(onClick = { showKeyBanner = !showKeyBanner }) {
                        Icon(
                            imageVector = Icons.Default.Key,
                            contentDescription = "Public Key",
                            tint = WarningYellow
                        )
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = DarkSurface
                )
            )
        },
        bottomBar = {
            KeyboardAccessoryRow(
                onKeyClick = { key ->
                    viewModel.sendQuickKey(key)
                }
            )
        },
        containerColor = DarkBackground
    ) { paddingValues ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
        ) {
            // Tab Selector Bar
            ScrollableTabRow(
                selectedTabIndex = tabs.indexOfFirst { it.id == selectedTabId }.coerceAtLeast(0),
                containerColor = DarkSurface,
                contentColor = TextPrimary,
                edgePadding = 8.dp,
                divider = { HorizontalDivider(color = DarkBorder) },
                modifier = Modifier.fillMaxWidth()
            ) {
                tabs.forEach { tab ->
                    Tab(
                        selected = tab.id == selectedTabId,
                        onClick = { viewModel.switchTab(tab.id) },
                        text = {
                            Text(
                                text = tab.title,
                                fontSize = 12.sp,
                                fontFamily = FontFamily.Monospace,
                                fontWeight = if (tab.id == selectedTabId) FontWeight.Bold else FontWeight.Normal
                            )
                        }
                    )
                }
            }

            // Public Key Banner
            AnimatedVisibility(visible = showKeyBanner) {
                val pubKey = keyManager.getPublicKey() ?: "Generating key..."
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(DarkSurface)
                        .padding(horizontal = 8.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Icon(
                        Icons.Default.Key,
                        contentDescription = "Key",
                        tint = WarningYellow,
                        modifier = Modifier.size(14.dp)
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = pubKey,
                        fontSize = 10.sp,
                        fontFamily = FontFamily.Monospace,
                        color = TextPrimary,
                        maxLines = 1,
                        modifier = Modifier.weight(1f)
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Button(
                        onClick = { keyManager.copyPublicKeyToClipboard(context) },
                        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 2.dp),
                        modifier = Modifier.height(28.dp),
                        shape = RoundedCornerShape(4.dp)
                    ) {
                        Text(text = "Copy Key", fontSize = 10.sp, fontWeight = FontWeight.Bold)
                    }
                }
            }

            // Connection Status Sub-banner (if Connecting/Reconnecting)
            when (val state = connectionState) {
                is ConnectionUiState.Connecting, is ConnectionUiState.Reconnecting -> {
                    LinearProgressIndicator(
                        modifier = Modifier.fillMaxWidth(),
                        color = AccentBlue,
                        trackColor = DarkSurface
                    )
                }
                is ConnectionUiState.Failed -> {
                    Surface(
                        color = DangerRed.copy(alpha = 0.2f),
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text(
                            text = "Connection Failed: ${state.reason}",
                            color = DangerRed,
                            fontSize = 11.sp,
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp)
                        )
                    }
                }
                else -> Unit
            }

            // Pending Approval Request Banner
            pendingApproval?.let { request ->
                ApprovalBanner(
                    request = request,
                    onApprove = { req -> viewModel.approveRequest(req) },
                    onDeny = { req -> viewModel.denyRequest(req) }
                )
            }

            // Main Terminal Output Area
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f)
            ) {
                TerminalView(output = terminalOutput)
            }
        }

        // Git Diff BottomSheet
        if (isDiffSheetOpen) {
            DiffViewerDialog(
                diffText = diffText,
                isLoading = isDiffLoading,
                onRefresh = { viewModel.fetchGitDiff() },
                onDismiss = { viewModel.closeDiffViewer() }
            )
        }
    }
}
