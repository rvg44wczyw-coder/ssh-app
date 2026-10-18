# Feature 29: syntax highlighting of files (modular, switchable)

Status: implemented for the file preview (2026-10-18); diff views are the next consumer.

## Goal
Colour the code in the file preview for as many file types as possible, as a module that can be
switched off (or replaced) in Settings → Appearance → Syntax highlighting.

## Design
- `SyntaxModule` protocol (`ios/Sources/SSHApp/Syntax`): an engine that says which language ids it
  supports and returns coloured lines for text + language + theme. The app never sees an engine's
  own types. `SyntaxService.modules` is the registry; the first enabled module that supports the
  language renders the file. A second engine (for example Rust `syntect` in the core, which would
  also serve Android later) is one more entry.
- First module: highlight.js through the Highlightr package, pinned to 2.3.0 (about 190 languages,
  runs in JavaScriptCore, off the main thread, one request at a time because the engine is not
  thread-safe). Only colours are taken from its output; font and size stay the app's.
- `SyntaxLanguages` maps file name and extension (and a shebang line for scripts) to a language id;
  unknown types are shown plain, nothing is auto-detected (it is slow and often wrong).
- Themes are families with a dark and a light variant (Atom One, GitHub, Solarized, Monokai, Visual
  Studio, Xcode, Gruvbox, Tokyo Night, Stack Overflow; Automatic = Atom One), chosen by the app's
  current appearance.
- Settings: master switch, theme, size limit (default 200 KB, 10..2000), per-module switch with the
  language count, and a live sample.

## Safety and cost
- Highlighting never changes the text, only its colours; the file content is passed to the engine
  as a string argument and never evaluated.
- Files over the size limit, binary files and unknown types skip highlighting; the preview itself
  already cuts at 500 lines.
- Nothing runs unless a file preview is open; there is no background work.
- The dependency is pinned to an exact version; Highlightr is MIT, highlight.js BSD-3-Clause.

## Not done
- Diff views and the AI transcript code blocks do not use it yet.
- No on-device test of the highlighter (Swift tests only compile here); verified by building and
  by the live sample in the settings page.
