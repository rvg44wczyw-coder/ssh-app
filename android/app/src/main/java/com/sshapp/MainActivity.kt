package com.sshapp

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.sshapp.ui.MainTerminalScreen
import com.sshapp.ui.theme.SSHAppTheme
import com.sshapp.viewmodel.TerminalViewModel

class MainActivity : ComponentActivity() {

    private val viewModel: TerminalViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Snapshot Privacy: Block screenshots and hide terminal content in Android Recents switcher
        window.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE
        )

        // Zero Battery Drain Lifecycle Observer
        lifecycle.addObserver(LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_STOP -> {
                    // Instantly teardown TCP socket when app is in background (0% CPU / Radio)
                    viewModel.disconnectForBackground()
                }
                Lifecycle.Event.ON_START -> {
                    // Fast resume: Reconnect to active tmux session on foreground
                    viewModel.reconnectOnForeground()
                }
                else -> Unit
            }
        })

        setContent {
            SSHAppTheme {
                MainTerminalScreen(viewModel = viewModel)
            }
        }
    }
}
