package com.sshapp.service

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.SharedPreferences
import android.widget.Toast
import uniffi.ssh_core.generateSshKeypair
import uniffi.ssh_core.derivePublicKey
import uniffi.ssh_core.signApprovalPayload

class AndroidKeyManager(private val context: Context) {

    private val prefs: SharedPreferences =
        context.getSharedPreferences("ssh_app_key_store", Context.MODE_PRIVATE)

    companion object {
        private const val KEY_PRIVATE = "ed25519_private_key"
        private const val KEY_PUBLIC = "ed25519_public_key"

        @Volatile
        private var instance: AndroidKeyManager? = null

        fun getInstance(context: Context): AndroidKeyManager {
            return instance ?: synchronized(this) {
                instance ?: AndroidKeyManager(context.applicationContext).also { instance = it }
            }
        }
    }

    fun hasPrivateKey(): Boolean {
        return prefs.getString(KEY_PRIVATE, null)?.isNotBlank() == true
    }

    fun getPrivateKey(): String? {
        return prefs.getString(KEY_PRIVATE, null)
    }

    fun getPublicKey(): String? {
        val pub = prefs.getString(KEY_PUBLIC, null)
        if (!pub.isNullOrBlank()) return pub

        val priv = getPrivateKey()
        if (!priv.isNullOrBlank()) {
            try {
                val derived = derivePublicKey(priv)
                savePublicKey(derived)
                return derived
            } catch (_: Exception) {}
        }
        return null
    }

    fun savePrivateKey(privKeyPem: String) {
        prefs.edit().putString(KEY_PRIVATE, privKeyPem.trim()).apply()
        try {
            val derived = derivePublicKey(privKeyPem.trim())
            savePublicKey(derived)
        } catch (_: Exception) {}
    }

    fun savePublicKey(pubKeyOpenssh: String) {
        prefs.edit().putString(KEY_PUBLIC, pubKeyOpenssh.trim()).apply()
    }

    fun getOrGenerateKeyPair(): Pair<String, String> {
        val priv = getPrivateKey()
        val pub = getPublicKey()
        if (!priv.isNullOrBlank() && !pub.isNullOrBlank()) {
            return Pair(pub, priv)
        }

        val keypair = generateSshKeypair()
        savePrivateKey(keypair.privateKeyOpenssh)
        savePublicKey(keypair.publicKeyOpenssh)
        return Pair(keypair.publicKeyOpenssh, keypair.privateKeyOpenssh)
    }

    fun signApproval(canonicalBytes: ByteArray): String {
        val privKey = getPrivateKey() ?: throw IllegalStateException("No private key found")
        return signApprovalPayload(privKey, canonicalBytes)
    }

    fun copyPublicKeyToClipboard(context: Context) {
        val pub = getPublicKey() ?: return
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val clip = ClipData.newPlainText("SSH Public Key", pub)
        clipboard.setPrimaryClip(clip)
        Toast.makeText(context, "Public key copied to clipboard", Toast.LENGTH_SHORT).show()
    }
}
