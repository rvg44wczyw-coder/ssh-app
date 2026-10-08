package com.sshapp.viewmodel

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.sshapp.model.AgentApprovalRequest
import com.sshapp.model.ServerProfile
import com.sshapp.model.SessionTab
import com.sshapp.service.AndroidKeyManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.ssh_core.*
import java.util.UUID

sealed class ConnectionUiState {
    object Disconnected : ConnectionUiState()
    object Connecting : ConnectionUiState()
    object Connected : ConnectionUiState()
    object Reconnecting : ConnectionUiState()
    data class Failed(val reason: String) : ConnectionUiState()
}

class TerminalViewModel(application: Application) : AndroidViewModel(application) {

    private val keyManager = AndroidKeyManager.getInstance(application)

    // Current Server Profile
    private val _serverProfile = MutableStateFlow(
        ServerProfile(
            name = "MacBook Pro",
            host = "macbook.tailnet",
            port = 22u,
            username = "ivs"
        )
    )
    val serverProfile: StateFlow<ServerProfile> = _serverProfile.asStateFlow()

    // Tabs
    private val _tabs = MutableStateFlow(
        listOf(
            SessionTab(title = "claude-code", tmuxSessionName = "claude-code"),
            SessionTab(title = "antigravity", tmuxSessionName = "antigravity"),
            SessionTab(title = "local-llm", tmuxSessionName = "local-llm")
        )
    )
    val tabs: StateFlow<List<SessionTab>> = _tabs.asStateFlow()

    private val _selectedTabId = MutableStateFlow(_tabs.value.first().id)
    val selectedTabId: StateFlow<String> = _selectedTabId.asStateFlow()

    // Connection UI state
    private val _connectionState = MutableStateFlow<ConnectionUiState>(ConnectionUiState.Disconnected)
    val connectionState: StateFlow<ConnectionUiState> = _connectionState.asStateFlow()

    // Terminal Screen Buffer
    private val _terminalOutput = MutableStateFlow("")
    val terminalOutput: StateFlow<String> = _terminalOutput.asStateFlow()

    // Pending Approval Request
    private val _pendingApproval = MutableStateFlow<AgentApprovalRequest?>(null)
    val pendingApproval: StateFlow<AgentApprovalRequest?> = _pendingApproval.asStateFlow()

    // Git Diff Viewer State
    private val _isDiffSheetOpen = MutableStateFlow(false)
    val isDiffSheetOpen: StateFlow<Boolean> = _isDiffSheetOpen.asStateFlow()

    private val _diffText = MutableStateFlow("")
    val diffText: StateFlow<String> = _diffText.asStateFlow()

    private val _isDiffLoading = MutableStateFlow(false)
    val isDiffLoading: StateFlow<Boolean> = _isDiffLoading.asStateFlow()

    // In-App Web Preview & Port Forward State
    private val _activePortForward = MutableStateFlow<PortForwardHandle?>(null)
    val activePortForward: StateFlow<PortForwardHandle?> = _activePortForward.asStateFlow()

    private val _isWebPreviewOpen = MutableStateFlow(false)
    val isWebPreviewOpen: StateFlow<Boolean> = _isWebPreviewOpen.asStateFlow()

    // AI Shell Assistant State
    private val _isAiAssistantOpen = MutableStateFlow(false)
    val isAiAssistantOpen: StateFlow<Boolean> = _isAiAssistantOpen.asStateFlow()

    private val _activeAiSuggestion = MutableStateFlow<AiCommandSuggestion?>(null)
    val activeAiSuggestion: StateFlow<AiCommandSuggestion?> = _activeAiSuggestion.asStateFlow()

    private val _isAiLoading = MutableStateFlow(false)
    val isAiLoading: StateFlow<Boolean> = _isAiLoading.asStateFlow()

    private val _aiError = MutableStateFlow<String?>(null)
    val aiError: StateFlow<String?> = _aiError.asStateFlow()

    // Active Rust Core session handle
    private var sessionHandle: SshSessionHandle? = null

    init {
        // Ensure Ed25519 keypair exists
        keyManager.getOrGenerateKeyPair()
        startActiveSession()
    }

    val activeTab: SessionTab?
        get() = _tabs.value.find { it.id == _selectedTabId.value }

    fun switchTab(tabId: String) {
        if (_selectedTabId.value == tabId) return
        _selectedTabId.value = tabId
        startActiveSession()
    }

    fun addTab(title: String) {
        val safeName = title.trim().lowercase().replace(Regex("[^a-z0-9_-]"), "-")
        val newTab = SessionTab(title = title, tmuxSessionName = safeName)
        _tabs.value = _tabs.value + newTab
        _selectedTabId.value = newTab.id
        startActiveSession()
    }

    fun startActiveSession() {
        val tab = activeTab ?: return
        val server = _serverProfile.value
        val privKey = keyManager.getPrivateKey() ?: return

        disconnectCurrent()

        _connectionState.value = ConnectionUiState.Connecting
        _terminalOutput.value = "Connecting to ${server.name} (${server.host})...\n"

        viewModelScope.launch(Dispatchers.IO) {
            try {
                val sessionConfig = SessionConfig(
                    host = server.host,
                    port = server.port,
                    username = server.username,
                    privateKeyPem = privKey,
                    sessionName = tab.tmuxSessionName,
                    initialCols = 80u,
                    initialRows = 24u,
                    command = null
                )

                val handle = SshSessionHandle(sessionConfig)
                sessionHandle = handle

                val callback = object : SshSessionCallback {
                    override fun onStateChanged(state: SessionState) {
                        viewModelScope.launch {
                            _connectionState.value = when (state) {
                                SessionState.Connected -> ConnectionUiState.Connected
                                SessionState.Connecting -> ConnectionUiState.Connecting
                                SessionState.Reconnecting -> ConnectionUiState.Reconnecting
                                SessionState.Disconnected -> ConnectionUiState.Disconnected
                                is SessionState.Failed -> ConnectionUiState.Failed(state.reason)
                            }
                        }
                    }

                    override fun onDataReceived(data: ByteArray) {
                        val text = String(data)
                        viewModelScope.launch {
                            appendTerminalOutput(text)
                            checkForApprovalRequest(text)
                        }
                    }

                    override fun onError(message: String) {
                        viewModelScope.launch {
                            _connectionState.value = ConnectionUiState.Failed(message)
                            appendTerminalOutput("\n[Error] $message\n")
                        }
                    }
                }

                handle.connect(callback)
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    _connectionState.value = ConnectionUiState.Failed(e.localizedMessage ?: "Connection error")
                    appendTerminalOutput("\n[Error] ${e.localizedMessage}\n")
                }
            }
        }
    }

    fun sendInput(bytes: ByteArray) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                sessionHandle?.sendInput(bytes)
            } catch (e: Exception) {
                e.printStackTrace()
            }
        }
    }

    fun sendText(text: String) {
        sendInput(text.toByteArray())
    }

    fun sendQuickKey(action: String) {
        when (action) {
            "y" -> sendInput(byteArrayOf(0x79, 0x0D)) // 'y' + Enter
            "n" -> sendInput(byteArrayOf(0x6E, 0x0D)) // 'n' + Enter
            "ctrl_c" -> sendInput(byteArrayOf(0x03))   // Ctrl+C
            "esc" -> sendInput(byteArrayOf(0x1B))      // Escape
            "tab" -> sendInput(byteArrayOf(0x09))      // Tab
            "up" -> sendInput(byteArrayOf(0x1B, 0x5B, 0x41))    // Arrow Up
            "down" -> sendInput(byteArrayOf(0x1B, 0x5B, 0x42))  // Arrow Down
            "pgup" -> sendInput(byteArrayOf(0x1B, 0x5B, 0x35, 0x7E)) // Page Up
            "pgdn" -> sendInput(byteArrayOf(0x1B, 0x5B, 0x36, 0x7E)) // Page Down
            "tmux_scroll" -> sendInput(byteArrayOf(0x02, 0x5B)) // tmux copy mode (Ctrl+B [)
            "q" -> sendInput(byteArrayOf(0x71)) // 'q' (exit copy mode)
            "pipe" -> sendText("|")
            "slash" -> sendText("/")
        }
    }

    fun resize(cols: Int, rows: Int) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                sessionHandle?.resize(cols = cols.toUShort(), rows = rows.toUShort())
            } catch (_: Exception) {}
        }
    }

    // Zero Battery Drain Handling
    fun disconnectForBackground() {
        disconnectCurrent()
        _connectionState.value = ConnectionUiState.Disconnected
    }

    fun reconnectOnForeground() {
        startActiveSession()
    }

    fun openWebPreview() {
        _isWebPreviewOpen.value = true
    }

    fun closeWebPreview() {
        _isWebPreviewOpen.value = false
        stopPortForward()
    }

    fun startPortForward(
        remotePort: UShort,
        localPort: UShort? = null,
        onResult: ((Result<PortForwardHandle>) -> Unit)? = null
    ) {
        val handle = sessionHandle ?: run {
            onResult?.invoke(Result.failure(IllegalStateException("Not connected")))
            return
        }

        viewModelScope.launch(Dispatchers.IO) {
            try {
                val fwd = handle.startPortForward(remotePort, localPort)
                _activePortForward.value = fwd
                withContext(Dispatchers.Main) {
                    onResult?.invoke(Result.success(fwd))
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    onResult?.invoke(Result.failure(e))
                }
            }
        }
    }

    fun stopPortForward() {
        _activePortForward.value?.stop()
        _activePortForward.value = null
    }

    // AI Shell Assistant Methods
    fun openAiAssistant() {
        _isAiAssistantOpen.value = true
        _aiError.value = null
    }

    fun closeAiAssistant() {
        _isAiAssistantOpen.value = false
    }

    fun queryAiAssistant(
        userPrompt: String,
        modelName: String = "qwen2.5-coder:7b",
        targetOs: String = "macOS"
    ) {
        val handle = sessionHandle ?: run {
            _aiError.value = "Session not connected"
            return
        }

        _isAiLoading.value = true
        _aiError.value = null

        viewModelScope.launch(Dispatchers.IO) {
            try {
                val lines = _terminalOutput.value.lines().filter { it.isNotBlank() }.takeLast(20)
                val systemPrompt = assembleAiSystemPrompt(targetOs, "zsh", null)
                val request = AiCommandRequest(
                    userPrompt = userPrompt,
                    targetOs = targetOs,
                    shellName = "zsh",
                    cwd = null,
                    terminalContext = lines
                )
                val assembledUserPrompt = assembleAiUserPrompt(request)

                val suggestion = handle.queryHostOllama(
                    model = modelName,
                    systemPrompt = systemPrompt,
                    userPrompt = assembledUserPrompt
                )
                withContext(Dispatchers.Main) {
                    _activeAiSuggestion.value = suggestion
                    _isAiLoading.value = false
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    _aiError.value = e.localizedMessage ?: "Failed to query AI assistant"
                    _isAiLoading.value = false
                }
            }
        }
    }

    fun insertAiCommand(command: String) {
        sendText(command)
        closeAiAssistant()
    }

    fun executeAiCommand(command: String) {
        sendText(command + "\r")
        closeAiAssistant()
    }

    private fun disconnectCurrent() {
        stopPortForward()
        try {
            sessionHandle?.disconnect()
        } catch (_: Exception) {}
        sessionHandle = null
    }

    // Approvals Handling
    fun approveRequest(req: AgentApprovalRequest) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val signature = keyManager.signApproval(req.getCanonicalBytes())
                // Send approval decision (y + enter) to unblock the agent
                sendInput(byteArrayOf(0x79, 0x0D))
                withContext(Dispatchers.Main) {
                    _pendingApproval.value = null
                    appendTerminalOutput("\n[Approved & Signed with Ed25519: ${signature.take(16)}...]\n")
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    appendTerminalOutput("\n[Signing Failed: ${e.localizedMessage}]\n")
                }
            }
        }
    }

    fun denyRequest(req: AgentApprovalRequest) {
        sendInput(byteArrayOf(0x03)) // Ctrl+C
        _pendingApproval.value = null
        appendTerminalOutput("\n[Approval Denied]\n")
    }

    // Simulated/Real agent trigger detection from stream
    private fun checkForApprovalRequest(chunk: String) {
        if (chunk.contains("[Approval Required]") || chunk.contains("PreToolUse") || chunk.contains("requests approval")) {
            if (_pendingApproval.value == null) {
                _pendingApproval.value = AgentApprovalRequest(
                    id = UUID.randomUUID().toString(),
                    agentName = "Claude Code",
                    command = "git push origin main",
                    cwd = "~/projects/ssh-app"
                )
            }
        }
    }

    // Git Diff Viewer
    fun openDiffViewer() {
        _isDiffSheetOpen.value = true
        fetchGitDiff()
    }

    fun closeDiffViewer() {
        _isDiffSheetOpen.value = false
    }

    fun fetchGitDiff() {
        val server = _serverProfile.value
        val privKey = keyManager.getPrivateKey() ?: return

        _isDiffLoading.value = true
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val config = RemoteServerConfig(
                    host = server.host,
                    port = server.port,
                    username = server.username,
                    privateKeyPem = privKey
                )
                val diffOutput = executeRemoteCommand(config, "git diff HEAD 2>&1")
                withContext(Dispatchers.Main) {
                    _diffText.value = diffOutput
                    _isDiffLoading.value = false
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    _diffText.value = "Failed to fetch git diff: ${e.localizedMessage}"
                    _isDiffLoading.value = false
                }
            }
        }
    }

    private fun appendTerminalOutput(newText: String) {
        val current = _terminalOutput.value
        // Keep buffer bounded to last 10,000 characters
        val combined = current + newText
        _terminalOutput.value = if (combined.length > 10000) {
            combined.takeLast(10000)
        } else {
            combined
        }
    }

    override fun onCleared() {
        super.onCleared()
        disconnectCurrent()
    }
}
