# Feature 40: Voice Input for Agent Prompts

- **Slug**: `agent-voice-dictation`
- **Target Release**: Milestone 29 (Mobile Ergonomics & Natural Language Steering)
- **Status**: Closed 2026-10-22 without code. The iOS keyboard's own dictation is used in the composer.

## Decision

Voice input in the Agent Feed composer is the system keyboard's dictation (the microphone key). The app has no recorder, no `SFSpeechRecognizer` code, no microphone permission string and no text post-processing.

- **Why**: a system component needs no code, no permission of ours and no maintenance, and the project prefers ready-made system components.
- **Privacy**: whether recognition runs on the device or sends audio to Apple is the system's choice per language and iOS version; the app neither controls nor shows it. The earlier promise "audio never leaves the device" is not made.
- **Technical words** (`auth.rs`, `--force`, `camelCase`): not handled. Whether dictation damages them was never measured on a device. If a device test shows real damage, a word-based normalizer (plain comparisons, no regular expressions; strip control characters; English and Russian spoken forms) can be added, and only then.
