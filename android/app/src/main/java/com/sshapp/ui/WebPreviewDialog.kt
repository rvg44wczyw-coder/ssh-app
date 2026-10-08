package com.sshapp.ui

import android.annotation.SuppressLint
import android.content.Intent
import android.net.Uri
import android.view.ViewGroup
import android.webkit.ConsoleMessage
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import com.sshapp.viewmodel.TerminalViewModel
import uniffi.ssh_core.PortForwardHandle

data class AndroidConsoleLog(
    val level: ConsoleMessage.MessageLevel,
    val message: String
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WebPreviewDialog(
    viewModel: TerminalViewModel,
    onDismiss: () -> Unit
) {
    val context = LocalContext.current
    val activeForward by viewModel.activePortForward.collectAsState()

    var selectedPort by remember { mutableStateOf<UShort>(3000u) }
    var customPortText by remember { mutableStateOf("") }
    var isLoading by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    val consoleLogs = remember { mutableStateListOf<AndroidConsoleLog>() }
    var isConsoleExpanded by remember { mutableStateOf(false) }
    var webViewInstance by remember { mutableStateOf<WebView?>(null) }

    val quickPorts = listOf<UShort>(3000u, 5173u, 8000u, 8080u)

    LaunchedEffect(Unit) {
        if (activeForward == null || activeForward?.isActive() != true) {
            isLoading = true
            viewModel.startPortForward(selectedPort) { result ->
                isLoading = false
                result.onFailure { errorMessage = it.localizedMessage }
            }
        } else {
            activeForward?.let { selectedPort = it.getRemotePort().toUShort() }
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Color(0xFF121316),
        dragHandle = { BottomSheetDefaults.DragHandle(color = Color(0xFF4A4D54)) },
        modifier = Modifier.fillMaxHeight(0.95f)
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            // Header Bar
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = "🌐 Web Preview",
                        fontSize = 16.sp,
                        fontWeight = FontWeight.Bold,
                        color = Color.White
                    )
                    activeForward?.let { fwd ->
                        if (fwd.isActive()) {
                            Spacer(modifier = Modifier.width(8.dp))
                            Surface(
                                color = Color(0xFF1B3828),
                                shape = RoundedCornerShape(4.dp)
                            ) {
                                Text(
                                    text = ":${fwd.getLocalPort()} ⇄ :${fwd.getRemotePort()}",
                                    fontSize = 11.sp,
                                    fontFamily = FontFamily.Monospace,
                                    color = Color(0xFF4ADE80),
                                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)
                                )
                            }
                        }
                    }
                }

                Row(verticalAlignment = Alignment.CenterVertically) {
                    IconButton(
                        onClick = { webViewInstance?.reload() },
                        enabled = activeForward?.isActive() == true
                    ) {
                        Icon(Icons.Default.Refresh, contentDescription = "Reload", tint = Color.LightGray)
                    }

                    IconButton(
                        onClick = {
                            activeForward?.let { fwd ->
                                val intent = Intent(Intent.ACTION_VIEW, Uri.parse(fwd.getLocalUrl()))
                                context.startActivity(intent)
                            }
                        },
                        enabled = activeForward?.isActive() == true
                    ) {
                        Icon(Icons.Default.Share, contentDescription = "Open in Chrome", tint = Color.LightGray)
                    }

                    IconButton(onClick = onDismiss) {
                        Icon(Icons.Default.Close, contentDescription = "Close", tint = Color.LightGray)
                    }
                }
            }

            // Port Selector Row
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(Color(0xFF18191D))
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 12.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    text = "Remote Port:",
                    fontSize = 12.sp,
                    color = Color.Gray,
                    modifier = Modifier.padding(end = 6.dp)
                )

                quickPorts.forEach { port ->
                    val isSelected = selectedPort == port
                    Surface(
                        onClick = {
                            selectedPort = port
                            isLoading = true
                            errorMessage = null
                            viewModel.startPortForward(port) { res ->
                                isLoading = false
                                res.onFailure { errorMessage = it.localizedMessage }
                            }
                        },
                        color = if (isSelected) MaterialTheme.colorScheme.primary else Color(0xFF26282E),
                        shape = RoundedCornerShape(6.dp),
                        modifier = Modifier.padding(horizontal = 4.dp)
                    ) {
                        Text(
                            text = ":$port",
                            fontSize = 12.sp,
                            fontWeight = FontWeight.SemiBold,
                            fontFamily = FontFamily.Monospace,
                            color = if (isSelected) Color.White else Color.LightGray,
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp)
                        )
                    }
                }

                Spacer(modifier = Modifier.width(6.dp))

                OutlinedTextField(
                    value = customPortText,
                    onValueChange = { customPortText = it.filter { ch -> ch.isDigit() } },
                    placeholder = { Text("Port", fontSize = 11.sp, color = Color.Gray) },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, imeAction = ImeAction.Go),
                    keyboardActions = KeyboardActions(onGo = {
                        val portNum = customPortText.toUShortOrNull()
                        if (portNum != null && portNum > 0u) {
                            selectedPort = portNum
                            isLoading = true
                            errorMessage = null
                            viewModel.startPortForward(portNum) { res ->
                                isLoading = false
                                res.onFailure { errorMessage = it.localizedMessage }
                            }
                        }
                    }),
                    modifier = Modifier
                        .width(72.dp)
                        .height(42.dp),
                    textStyle = LocalTextStyle.current.copy(fontSize = 12.sp, fontFamily = FontFamily.Monospace),
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedTextColor = Color.White,
                        unfocusedTextColor = Color.White,
                        focusedBorderColor = MaterialTheme.colorScheme.primary,
                        unfocusedBorderColor = Color(0xFF3E4048)
                    )
                )
            }

            HorizontalDivider(color = Color(0xFF2A2B30), thickness = 0.5.dp)

            // Main Web Viewport or Loading
            Box(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth()
            ) {
                if (isLoading) {
                    Column(
                        modifier = Modifier.fillMaxSize(),
                        verticalArrangement = Arrangement.Center,
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        CircularProgressIndicator(color = MaterialTheme.colorScheme.primary)
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(
                            text = "Opening SSH direct-tcpip tunnel to :$selectedPort...",
                            fontSize = 13.sp,
                            fontFamily = FontFamily.Monospace,
                            color = Color.Gray
                        )
                    }
                } else if (activeForward != null && activeForward?.isActive() == true) {
                    val localUrl = activeForward!!.getLocalUrl()
                    AndroidWebViewContainer(
                        url = localUrl,
                        onCreated = { webViewInstance = it },
                        onConsoleMessage = { log -> consoleLogs.add(log) }
                    )
                } else {
                    Column(
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(24.dp),
                        verticalArrangement = Arrangement.Center,
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        Text(
                            text = "No Port Forward Active",
                            fontSize = 15.sp,
                            fontWeight = FontWeight.Bold,
                            color = Color.White
                        )
                        errorMessage?.let { err ->
                            Spacer(modifier = Modifier.height(6.dp))
                            Text(
                                text = err,
                                fontSize = 12.sp,
                                color = Color.Red,
                                fontFamily = FontFamily.Monospace
                            )
                        }
                        Spacer(modifier = Modifier.height(12.dp))
                        Button(
                            onClick = {
                                isLoading = true
                                errorMessage = null
                                viewModel.startPortForward(selectedPort) { res ->
                                    isLoading = false
                                    res.onFailure { errorMessage = it.localizedMessage }
                                }
                            }
                        ) {
                            Icon(Icons.Default.PlayArrow, contentDescription = null)
                            Spacer(modifier = Modifier.width(4.dp))
                            Text("Forward :$selectedPort")
                        }
                    }
                }

                // Collapsible Console Drawer
                if (isConsoleExpanded) {
                    Surface(
                        modifier = Modifier
                            .align(Alignment.BottomCenter)
                            .fillMaxWidth()
                            .height(180.dp),
                        color = Color(0xFF0F1012),
                        tonalElevation = 8.dp,
                        shadowElevation = 8.dp
                    ) {
                        Column {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .background(Color(0xFF1E2024))
                                    .padding(horizontal = 12.dp, vertical = 6.dp),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Text(
                                    text = "Console (${consoleLogs.size})",
                                    fontSize = 11.sp,
                                    fontWeight = FontWeight.Bold,
                                    color = Color.White
                                )
                                Row {
                                    TextButton(onClick = { consoleLogs.clear() }) {
                                        Text("Clear", fontSize = 11.sp)
                                    }
                                    IconButton(
                                        onClick = { isConsoleExpanded = false },
                                        modifier = Modifier.size(24.dp)
                                    ) {
                                        Icon(Icons.Default.Close, contentDescription = null, tint = Color.Gray)
                                    }
                                }
                            }

                            LazyColumn(
                                modifier = Modifier
                                    .fillMaxSize()
                                    .padding(8.dp)
                            ) {
                                items(consoleLogs) { log ->
                                    val logColor = when (log.level) {
                                        ConsoleMessage.MessageLevel.ERROR -> Color(0xFFF87171)
                                        ConsoleMessage.MessageLevel.WARNING -> Color(0xFFFBBF24)
                                        else -> Color(0xFFE2E8F0)
                                    }
                                    Text(
                                        text = log.message,
                                        fontSize = 11.sp,
                                        fontFamily = FontFamily.Monospace,
                                        color = logColor,
                                        modifier = Modifier.padding(vertical = 2.dp)
                                    )
                                }
                            }
                        }
                    }
                }
            }

            // Bottom DevTools Bar
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(Color(0xFF16171B))
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    text = if (activeForward?.isActive() == true) "Tunnel Connected" else "Stopped",
                    fontSize = 11.sp,
                    color = Color.Gray
                )

                TextButton(
                    onClick = { isConsoleExpanded = !isConsoleExpanded }
                ) {
                    Text(
                        text = "DevTools Console (${consoleLogs.size})",
                        fontSize = 11.sp,
                        fontWeight = FontWeight.Bold,
                        color = if (consoleLogs.isEmpty()) Color.Gray else MaterialTheme.colorScheme.primary
                    )
                }
            }
        }
    }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable
private fun AndroidWebViewContainer(
    url: String,
    onCreated: (WebView) -> Unit,
    onConsoleMessage: (AndroidConsoleLog) -> Unit
) {
    AndroidView(
        factory = { ctx ->
            WebView(ctx).apply {
                layoutParams = ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT
                )
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                settings.allowFileAccess = false
                settings.allowContentAccess = false

                webViewClient = WebViewClient()
                webChromeClient = object : WebChromeClient() {
                    override fun onConsoleMessage(consoleMessage: ConsoleMessage?): Boolean {
                        consoleMessage?.let {
                            onConsoleMessage(AndroidConsoleLog(it.messageLevel(), it.message()))
                        }
                        return super.onConsoleMessage(consoleMessage)
                    }
                }
                loadUrl(url)
                onCreated(this)
            }
        },
        update = { webView ->
            if (webView.url != url) {
                webView.loadUrl(url)
            }
        },
        modifier = Modifier.fillMaxSize()
    )
}
