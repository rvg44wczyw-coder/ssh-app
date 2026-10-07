package com.sshapp.model

import uniffi.ssh_core.createCanonicalSigningBytes
import uniffi.ssh_core.hashCommandSha256
import java.util.UUID

data class AgentApprovalRequest(
    val id: String,
    val agentName: String,
    val command: String,
    val cwd: String,
    val timestampSec: ULong = (System.currentTimeMillis() / 1000).toULong(),
    val nonce: String = UUID.randomUUID().toString().replace("-", "").lowercase(),
    val commandHashSha256: String = hashCommandSha256(command)
) {
    fun getCanonicalBytes(): ByteArray {
        return createCanonicalSigningBytes(
            id = id,
            commandHashSha256 = commandHashSha256,
            nonce = nonce,
            timestampSec = timestampSec,
            approved = true
        )
    }
}

data class SignedApprovalResult(
    val id: String,
    val approved: Boolean,
    val signatureHex: String,
    val publicKeyOpenssh: String,
    val timestampSec: ULong
)
