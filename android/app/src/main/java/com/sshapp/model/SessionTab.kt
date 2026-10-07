package com.sshapp.model

import java.util.UUID

data class SessionTab(
    val id: String = UUID.randomUUID().toString(),
    val title: String,
    val tmuxSessionName: String,
    val serverId: String? = null
)
