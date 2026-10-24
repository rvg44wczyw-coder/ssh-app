# Face ID for the Files extension (2026-10-24)

## Scope

`ios/Sources/Shared/SharedServers.swift` (`guarded`, `publishGuardedKeys`, guarded read), `ios/SSHAppFiles/ConnectionPool.swift`, `ios/Sources/SSHApp/Services/FilesIntegration.swift`, the switch on the Security page. Owner's decision 2026-10-24: one global switch, a separate guarded copy of the key (option A). Code analysis and unit tests only: the system prompt inside a File Provider extension has to be confirmed on a phone.

Applied: credential handling, authorization bypass. Ruled out: network protocol, parsing (no new input from outside).

## Design

- Switch on: the app writes a copy of every key used by a server in Files under service `com.sshapp.filekeys` with `SecAccessControl(.userPresence, WhenPasscodeSetThisDeviceOnly)`; published servers carry `guarded = true`.
- The extension then reads only the guarded copy through an `LAContext` (60 s reuse); failure or a missing copy is `notAuthenticated`. It never falls back to the plain key.
- Switch off: the guarded copies are deleted first, then the list is published without the flag.
- Order of publishing: copies first, list second, so a guarded server never exists without its copy.

## Findings

1. **Controversial, kept as a named tradeoff**: the plain key stays in the same shared Keychain group, so the gate is enforced by the extension's code, not by cryptography. Someone who can run their own code with the app's entitlement has the key anyway; the switch stops the real threat, a person with the unlocked phone opening Files.
2. **Controversial, kept**: files the Files app already downloaded stay on the device and open without the prompt. The gate protects the connection and any new listing or download, not copies already materialised.
3. **Low, by design**: a 60 s reuse window after a successful check; a second visit inside it does not ask again.
4. **Low, handled**: the passcode is removed: the system deletes the guarded copies, guarded servers fail closed in Files, the next app launch republishes (and, with no passcode, cannot create copies and the servers stay closed). Turning the switch on needs a passcode (`AppLock.canBeUsed`).
5. **Open until a device run**: whether the system shows the Face ID sheet from inside the extension, or answers `errSecInteractionNotAllowed`. The code fails closed in both cases; if no sheet appears, the fallback is the in-app unlock window (option B in the discussion).

## Verdict

No Critical, High or Medium finding by analysis; one item (5) needs the phone.
