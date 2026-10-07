package com.sshapp.model

import java.util.UUID

data class ServerProfile(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val host: String,
    val port: UShort = 22u,
    val username: String,
    val privateKeyPem: String = ""
)
