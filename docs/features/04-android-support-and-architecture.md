# Feature Specification: Android Client Architecture & Cross-Platform Support

- **Slug**: `04-android-support-and-architecture`
- **Milestone**: M7 (Android Client: Jetpack Compose + UniFFI Kotlin + AndroidKeyStore)
- **Participating Roles**: `doc-dev`, `rust-core-dev`, `android-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (Mandatory: Hardware-backed key storage in AndroidKeyStore/TEE, BiometricPrompt authorization, Zero-Knowledge FCM/UnifiedPush wake-up, and `FLAG_SECURE` privacy)

---

## 1. Problem Statement & Scope

### 1.1. Context & Motivation
Following the implementation of the iOS client and Rust Core (`crates/core`), users on Android devices (Google Pixel, Samsung Galaxy, OnePlus, and privacy-focused systems like GrapheneOS / CalyxOS) need the identical set of capabilities:
1. **Zero Battery Drain**: Sockets closed when the app is placed in background (`Lifecycle.Event.ON_STOP`); remote long-running jobs remain uninterrupted inside `tmux` on the Mac.
2. **Hardware-Signed Approvals**: Mobile approval decisions cryptographically signed via Ed25519 backed by hardware security (StrongBox / TEE / AndroidKeyStore), verified on the MacBook against `~/.ssh/authorized_keys`.
3. **Zero Network Attack Surface on Host**: No open HTTP/REST servers or listening ports on the Mac.
4. **Zero-Knowledge Wake-up Notifications**: Wake-up triggers that contain **zero** command text, project paths, or code snippets, functioning with both Firebase Cloud Messaging (FCM) and self-hosted open standards (UnifiedPush / ntfy over Tailscale).
5. **Interactive Terminal & Agent Review**: Rich Jetpack Compose UI, colorized `git diff` review, quick-action accessory bar (`✓ y⏎`, `✗ n⏎`, `Ctrl`, `Esc`), and Lock Screen Notification Actions.

### 1.2. Scope
- **In Scope**:
  1. **UniFFI Kotlin Bindings & JNI Packaging**: Cross-compiling `crates/core` for Android ABIs (`arm64-v8a`, `armeabi-v7a`, `x86_64`) via `cargo-ndk` and generating clean Kotlin wrappers.
  2. **Hardware Key Management**: `AndroidKeyStore` integration with StrongBox Keymaster / TEE for Ed25519 keypair generation and signing, with optional `BiometricPrompt` authorization.
  3. **Dual Push Notification Architecture**:
     - *Standard*: High-priority data-only wake-up pings via Firebase Cloud Messaging (FCM) through the Cloudflare Worker ($0 cost, 0 sensitive bytes).
     - *Private / De-Googled*: Self-hosted UnifiedPush / ntfy ping directly over the Tailscale network without any Google Play Services requirement.
  4. **Direct Notification Actions**: Android `NotificationCompat` action buttons (`[Approve]`, `[Deny]`, `[Diff]`) allowing approval execution without bringing the app to the foreground.
  5. **Jetpack Compose UI**: Modern declarative Android UI mirroring iOS features (multi-tab session switcher, terminal container with SwiftTerm/Termux emulator engine, diff viewer sheet).
  6. **Privacy & Snapshot Protection**: `WindowManager.LayoutParams.FLAG_SECURE` prevention of sensitive terminal data leaks in Android's Recents app switcher.
- **Out of Scope**:
  - Proprietary third-party cloud hosting or syncing of terminal history.
  - SMS or email verification fallback.

---

## 2. Architecture & Layer Comparison (iOS vs Android)

```
┌────────────────────────────────────────────────────────────────────────┐
│                          SHARED RUST CORE                              │
│  crates/core (russh, ssh-key, ed25519-dalek, tmux, OutputThrottler)   │
└──────────────────┬─────────────────────────────────┬───────────────────┘
                   │ UniFFI Swift Bindings           │ UniFFI Kotlin Bindings
                   ▼                                 ▼
┌──────────────────────────────────────┐  ┌──────────────────────────────────────┐
│             iOS CLIENT               │  │           ANDROID CLIENT             │
├──────────────────────────────────────┤  ├──────────────────────────────────────┤
│ UI: SwiftUI                          │  │ UI: Jetpack Compose                  │
│ Terminal: SwiftTerm                  │  │ Terminal: Termux View / Compose PTY  │
│ Crypto: Keychain + Secure Enclave    │  │ Crypto: AndroidKeyStore (StrongBox)  │
│ Push: APNs via Cloudflare Worker     │  │ Push: FCM via Cloudflare Worker OR   │
│                                      │  │       UnifiedPush/ntfy via Tailscale │
│ Background: sceneDidEnterBackground  │  │ Background: Lifecycle ON_STOP        │
│ Snapshot Privacy: SwiftUI Overlay    │  │ Snapshot Privacy: Window FLAG_SECURE │
│ Biometrics: LocalAuthentication      │  │ Biometrics: BiometricPrompt          │
└──────────────────────────────────────┘  └──────────────────────────────────────┘
```

---

## 3. UniFFI Kotlin Bindings & Native Library Compilation

### 3.1. Android ABIs
`crates/core` compiles into native static/shared libraries for all standard Android architectures:
- `aarch64-linux-android` $\rightarrow$ `jniLibs/arm64-v8a/libssh_core.so` (Physical devices)
- `armv7-linux-androideabi` $\rightarrow$ `jniLibs/armeabi-v7a/libssh_core.so` (Older 32-bit devices)
- `x86_64-linux-android` $\rightarrow$ `jniLibs/x86_64/libssh_core.so` (Android Studio Emulator)

### 3.2. Automated Android Build Script (`scripts/build-android.sh`)
```bash
#!/usr/bin/env bash
set -euo pipefail

# 1. Ensure Android NDK targets are installed in Rust
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

# 2. Compile shared libraries with cargo-ndk
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o android/app/src/main/jniLibs build --release -p ssh-core

# 3. Generate Kotlin bindings from UniFFI
cargo run -p ssh-core --bin uniffi-bindgen generate \
  target/aarch64-linux-android/release/libssh_core.so \
  --language kotlin \
  --out-dir android/app/src/main/java/com/sshapp/core/generated
```

### 3.3. Kotlin Native API Surface
UniFFI generates identical strongly-typed Kotlin classes and suspendable functions:
```kotlin
package com.sshapp.core.generated

// Identical to Swift / Rust Core API
val canonicalBytes: ByteArray = createCanonicalSigningBytes(
    id = request.id,
    commandHashSha256 = request.commandHashSha256,
    nonce = request.nonce,
    timestampSec = request.timestampSec,
    approved = true
)

val isFresh: Boolean = verifyApprovalFreshness(
    timestampSec = request.timestampSec,
    currentTimeSec = System.currentTimeMillis() / 1000,
    maxDriftSec = 120UL
)
```

---

## 4. Hardware Cryptography: AndroidKeyStore & Biometrics

### 4.1. Hardware Key Storage Strategy
On Android, cryptographic key storage leverages the `AndroidKeyStore` provider:
- **Android 13+ (API 33+)**: Direct hardware-backed Ed25519 key generation via:
  ```kotlin
  val kpg = KeyPairGenerator.getInstance(
      KeyProperties.KEY_ALGORITHM_EC,
      "AndroidKeyStore"
  )
  val spec = KeyGenParameterSpec.Builder(
      "ssh_app_user_key",
      KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY
  )
      .setAlgorithmParameterSpec(NamedParameterSpec("ed25519"))
      .setDigests(KeyProperties.DIGEST_NONE)
      .setIsStrongBoxBacked(true) // StrongBox HSM if available, fallback to TEE
      .setUserAuthenticationRequired(true) // Requires biometrics/device unlock
      .setUserAuthenticationParameters(
          30, // 30-second auth window
          KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL
      )
      .build()
  kpg.initialize(spec)
  val keyPair = kpg.generateKeyPair()
  ```
- **Android < 13 Support**:
  If the device runs Android 10–12 where `AndroidKeyStore` Ed25519 was not standardized, the app generates the Ed25519 keypair in Rust core (`generate_keypair()`) and stores the private key encrypted with a 256-bit AES-GCM MasterKey inside `EncryptedSharedPreferences` / Jetpack Security (`androidx.security.crypto.EncryptedSharedPreferences`), with the AES key strictly anchored in `AndroidKeyStore`.

### 4.2. Biometric Authentication Gate
Before signing critical commands (`rm -rf`, `git push --force`, database drop), the app prompts the user via `BiometricPrompt`:
```kotlin
val biometricPrompt = BiometricPrompt(
    activity,
    ContextCompat.getMainExecutor(context),
    object : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
            // Sign canonical payload and dispatch to SSH channel
            val signatureHex = signApprovalPayload(privateKey, canonicalBytes)
            sendApprovalOverSsh(request.id, signatureHex)
        }
        override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
            // Reject or cancel approval
        }
    }
)
```

---

## 5. Push Notifications: FCM vs. Self-Hosted UnifiedPush

To respect the user's preference for either standard cloud conveniences or total independence from Big Tech, Android supports two notification delivery channels:

```
[Mac Agent Hook]
       │
       ├─► (Option A: Cloudflare Worker ──► Firebase Cloud Messaging ──► Android Phone)
       │    - Cost: $0/month (Free Tier)
       │    - Payload: Empty wake-up ping (Zero-Knowledge)
       │
       └─► (Option B: Tailscale Mesh ──────► UnifiedPush / ntfy ────────► Android Phone)
            - Cost: $0/month (Self-Hosted on Mac or Home Server)
            - 100% De-Googled (GrapheneOS / CalyxOS / LineageOS)
            - No traffic leaves the WireGuard Tailscale tunnel!
```

### 5.1. Channel A: Zero-Knowledge FCM (Firebase Cloud Messaging)
- **Cloudflare Worker Extension**: The worker at `scripts/apns-worker/worker.ts` adds an endpoint for FCM HTTP v1 API (`/fcm/wake`).
- **Payload Guarantee**: Contains **ONLY** an opaque FCM device registration token and a silent data ping:
  ```json
  {
    "message": {
      "token": "<64_CHAR_DEVICE_TOKEN>",
      "data": { "action": "WAKE_UP_APPROVAL" },
      "android": { "priority": "high" }
    }
  }
  ```
- **Execution**: The device receives the high-priority data message even in Doze mode, triggers a transient `CoroutineWorker`, opens an SSH channel to the Mac over Tailscale, and retrieves the approval payload in under 500ms.

### 5.2. Channel B: Direct Tailscale UnifiedPush / ntfy (Zero-Cloud Mode)
- **Zero Third-Party Relays**: Ideal for users running GrapheneOS without Google Play Services.
- **Mechanism**: The Mac approval daemon sends a local HTTP POST directly to an `ntfy` server running inside the user's own Tailscale network:
  ```bash
  curl -s -d "WAKE" "http://macbook.tailnet:8080/ssh-app-approvals"
  ```
- The Android phone connects directly to `macbook.tailnet:8080` via persistent Tailscale WireGuard socket with zero cloud intermediaries.

---

## 6. Android Notification Actions & Direct Approvals

Android provides a distinct advantage over iOS: **Actionable Notifications without opening the app**.

```
┌──────────────────────────────────────────────────────────┐
│ 🛡️ SSH-App • Agent Approval Required                     │
│ Claude Code requests permission                          │
│ Command: git push origin main --force                    │
│ Nonce: a1b2c3d4... • Hash: 16f88028...                   │
├──────────────────────────────────────────────────────────┤
│   [ ✓ APPROVE (SIGN) ]   [ ✗ DENY ]   [ 👁️ VIEW DIFF ]    │
└──────────────────────────────────────────────────────────┘
```

### 6.1. Notification Implementation (`ApprovalNotificationManager.kt`)
```kotlin
val approveIntent = Intent(context, ApprovalActionReceiver::class.java).apply {
    action = "ACTION_APPROVE"
    putExtra("EXTRA_REQUEST_ID", request.id)
    putExtra("EXTRA_COMMAND_HASH", request.commandHashSha256)
}
val approvePendingIntent = PendingIntent.getBroadcast(
    context, reqCodeApprove, approveIntent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
)

val notification = NotificationCompat.Builder(context, CHANNEL_ID)
    .setSmallIcon(R.drawable.ic_shield_check)
    .setContentTitle("${request.agentName} requests approval")
    .setContentText(request.command)
    .setStyle(NotificationCompat.BigTextStyle().bigText("Directory: ${request.cwd}\nCommand: ${request.command}"))
    .setPriority(NotificationCompat.PRIORITY_MAX)
    .setCategory(NotificationCompat.CATEGORY_ALARM)
    .addAction(R.drawable.ic_check, "Approve (Sign)", approvePendingIntent)
    .addAction(R.drawable.ic_close, "Deny", denyPendingIntent)
    .addAction(R.drawable.ic_diff, "View Diff", diffPendingIntent)
    .setAutoCancel(true)
    .build()
```

When the user taps `[Approve (Sign)]` directly from the lock screen:
1. `ApprovalActionReceiver` (a `BroadcastReceiver`) receives the intent.
2. If biometric authentication is required, it prompts with `BiometricPrompt`; otherwise it signs using `AndroidKeyStore`.
3. An ephemeral `russh` SSH command transmits the signature back to the MacBook's Unix Domain Socket.
4. The notification updates to a green checkmark: `"Approved & Signed"`.

---

## 7. State Machine & Android Lifecycle (Zero Battery Drain)

### 7.1. Lifecycle Mapping
```
        ┌─────────────────────────────────────────────────┐
        │                 App Launch                      │
        └────────────────────────┬────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                    ACTIVE / FOREGROUND                          │
│  - Activity in ON_START / ON_RESUME                             │
│  - TCP socket open to MacBook via Tailscale                     │
│  - SwiftTerm/Termux PTY rendering terminal stream               │
│  - WindowManager.LayoutParams.FLAG_SECURE active                │
└────────────────────────┬────────────────────────────────┬───────┘
                         │                                ▲
         User minimizes  │                                │ User opens app /
         (ON_STOP)       │                                │ taps notification
                         ▼                                │
┌─────────────────────────────────────────────────────────┴───────┐
│                 SUSPENDED / ZERO BATTERY DRAIN                  │
│  - TCP socket closed immediately; 0% CPU & 0% Radio             │
│  - Remote agents run persistently inside tmux                   │
│  - Incoming wake-up: transient WorkManager / FCM wake           │
└─────────────────────────────────────────────────────────────────┘
```

### 7.2. Snapshot Privacy Protection (`FLAG_SECURE`)
In `MainActivity.kt`:
```kotlin
override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    // Prevents Android OS from caching snapshots of terminal output in the Recents screen
    // Blocks screen capture and screen recording of sensitive SSH keys and API tokens
    window.setFlags(
        WindowManager.LayoutParams.FLAG_SECURE,
        WindowManager.LayoutParams.FLAG_SECURE
    )
}
```

---

## 8. User Interface Specification (Jetpack Compose)

1. **`MainTerminalScreen.kt`**:
   - Header with active server pill (`• MacBook Pro (Tailscale)`), tmux tab bar (`claude-code`, `antigravity`, `ollama`), and quick actions (`Git Diff`, `Keys`, `Settings`).
   - Terminal Container embedding high-performance PTY view (`TerminalView`) supporting TrueColor `xterm-256color` and hardware font rendering (JetBrains Mono).
   - In-app `ApprovalBanner`: dismissable floating banner with monospace command preview, color-coded danger badge, and Haptic feedback.
2. **`KeyboardAccessoryRow.kt`**:
   - Fixed Compose row above the Android virtual keyboard: `Esc`, `Tab`, `Ctrl`, `▲`, `▼`, `|`, `/`, `~`, `✓ y⏎`, `✗ n⏎`.
3. **`DiffViewerDialog.kt`**:
   - Modal bottom sheet displaying colorized `git diff HEAD` fetched via transient SSH channel, with line numbers and diff header syntax.

---

## 9. Testing & Verification Criteria

1. **Rust NDK Cross-Compilation Tests**:
   - Validate clean compilation with `cargo ndk` for `aarch64-linux-android` and `x86_64-linux-android`.
   - Ensure JNI shared libraries export all UniFFI symbols without missing `libc++_shared.so` linkages.
2. **Android Unit & Crypto Tests**:
   - Verify `createCanonicalSigningBytes` produces exact identical binary outputs between Kotlin and Rust.
   - Test Ed25519 signing with AndroidKeyStore / StrongBox and verify round-trip signature with Rust's `verify_approval_signature`.
   - Verify replay protection (stale timestamps > 120s rejected).
3. **Lifecycle & Battery Tests**:
   - Validate that placing the app in background triggers immediate TCP disconnect within 100ms.
   - Validate 0% background battery drain in Android Battery Historian.
4. **Push & Lock Screen Action Tests**:
   - Verify lock-screen notification displays `Approve` and `Deny` action buttons.
   - Verify tapping `Approve` delivers cryptographic signature to Mac agent hook within 2 seconds.
