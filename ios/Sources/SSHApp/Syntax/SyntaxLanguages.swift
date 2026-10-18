import Foundation

/// Which highlight.js language a file is, from its name (and, for scripts, its first line).
/// Returns an id only; whether a provider can render it is the provider's business.
enum SyntaxLanguages {
    private static let byExtension: [String: String] = [
        // Systems and general purpose
        "rs": "rust", "c": "c", "h": "c", "cc": "cpp", "cpp": "cpp", "cxx": "cpp", "hpp": "cpp",
        "hh": "cpp", "hxx": "cpp", "m": "objectivec", "mm": "objectivec", "swift": "swift",
        "go": "go", "java": "java", "kt": "kotlin", "kts": "kotlin", "scala": "scala",
        "cs": "csharp", "fs": "fsharp", "vb": "vbnet", "dart": "dart", "zig": "zig", "nim": "nim",
        "d": "d", "v": "verilog", "sv": "verilog", "vhd": "vhdl", "vhdl": "vhdl", "ada": "ada",
        "adb": "ada", "pas": "delphi", "f90": "fortran", "f": "fortran", "for": "fortran",
        "cob": "cobol", "asm": "x86asm", "s": "armasm", "wat": "wasm", "wasm": "wasm",
        // Scripting
        "py": "python", "pyw": "python", "pyi": "python", "rb": "ruby", "rake": "ruby",
        "gemspec": "ruby", "php": "php", "pl": "perl", "pm": "perl", "lua": "lua", "r": "r",
        "jl": "julia", "tcl": "tcl", "groovy": "groovy", "gradle": "groovy", "ex": "elixir",
        "exs": "elixir", "erl": "erlang", "hrl": "erlang", "clj": "clojure", "cljs": "clojure",
        "lisp": "lisp", "el": "lisp", "scm": "scheme", "rkt": "scheme", "hs": "haskell",
        "ml": "ocaml", "mli": "ocaml", "elm": "elm", "purs": "purescript", "cr": "crystal",
        "awk": "awk", "applescript": "applescript", "scpt": "applescript",
        // Web
        "js": "javascript", "mjs": "javascript", "cjs": "javascript", "jsx": "javascript",
        "ts": "typescript", "tsx": "typescript", "mts": "typescript", "cts": "typescript",
        "html": "xml", "htm": "xml", "xhtml": "xml", "xml": "xml", "xsl": "xml", "xsd": "xml",
        "svg": "xml", "plist": "xml", "csproj": "xml", "storyboard": "xml", "xib": "xml",
        "css": "css", "scss": "scss", "sass": "scss", "less": "less", "styl": "stylus",
        "vue": "xml", "svelte": "xml", "astro": "xml", "hbs": "handlebars", "mustache": "handlebars",
        "twig": "twig", "erb": "erb", "ejs": "xml", "pug": "pug", "jade": "pug",
        "coffee": "coffeescript", "graphql": "graphql", "gql": "graphql", "wasi": "wasm",
        // Shell and ops
        "sh": "bash", "bash": "bash", "zsh": "bash", "ksh": "bash", "fish": "shell",
        "ps1": "powershell", "psm1": "powershell", "bat": "dos", "cmd": "dos",
        "tf": "hcl", "hcl": "hcl", "nix": "nix", "dockerfile": "dockerfile",
        "mk": "makefile", "cmake": "cmake", "ninja": "ninja", "bzl": "python", "nginx": "nginx",
        "conf": "ini", "ini": "ini", "cfg": "ini", "properties": "properties", "toml": "ini",
        "service": "ini", "desktop": "ini", "env": "bash", "gitignore": "bash",
        // Data and docs
        "json": "json", "jsonc": "json", "json5": "json", "geojson": "json", "ndjson": "json",
        "yaml": "yaml", "yml": "yaml", "csv": "plaintext", "sql": "sql", "psql": "pgsql",
        "md": "markdown", "markdown": "markdown", "mdx": "markdown", "rst": "plaintext",
        "tex": "latex", "latex": "latex", "bib": "latex", "diff": "diff", "patch": "diff",
        "proto": "protobuf", "thrift": "thrift", "capnp": "capnproto", "fbs": "protobuf",
        "wgsl": "wgsl", "glsl": "glsl", "vert": "glsl", "frag": "glsl", "metal": "cpp",
        "hlsl": "hlsl", "cu": "cpp", "cl": "cpp", "sol": "solidity", "move": "rust",
        "gd": "gdscript", "tscn": "ini", "qml": "qml", "pro": "ini", "http": "http",
        "log": "accesslog", "pdf": "plaintext",
    ]

    private static let byName: [String: String] = [
        "makefile": "makefile", "gnumakefile": "makefile", "dockerfile": "dockerfile",
        "containerfile": "dockerfile", "cmakelists.txt": "cmake", "rakefile": "ruby",
        "gemfile": "ruby", "podfile": "ruby", "fastfile": "ruby", "vagrantfile": "ruby",
        "brewfile": "ruby", "jenkinsfile": "groovy", "package.swift": "swift", "cargo.lock": "ini",
        "cargo.toml": "ini", "go.mod": "go", "go.sum": "plaintext", ".bashrc": "bash",
        ".zshrc": "bash", ".zprofile": "bash", ".profile": "bash", ".bash_profile": "bash",
        ".gitconfig": "ini", ".editorconfig": "ini", ".env": "bash", ".gitignore": "bash",
        "nginx.conf": "nginx", "justfile": "makefile", "procfile": "yaml",
    ]

    private static let shebang: [(String, String)] = [
        ("python", "python"), ("node", "javascript"), ("deno", "typescript"), ("bun", "javascript"),
        ("ruby", "ruby"), ("perl", "perl"), ("php", "php"), ("lua", "lua"),
        ("pwsh", "powershell"), ("fish", "shell"), ("bash", "bash"), ("zsh", "bash"),
        ("/sh", "bash"), (" sh", "bash"), ("awk", "awk"), ("Rscript", "r"), ("swift", "swift"),
    ]

    static func id(forFilename filename: String, firstLine: String?) -> String? {
        let lower = filename.lowercased()
        if let id = byName[lower] { return id }
        if lower.hasPrefix("dockerfile.") { return "dockerfile" }
        if let dot = lower.lastIndex(of: "."), dot != lower.startIndex {
            let ext = String(lower[lower.index(after: dot)...])
            if let id = byExtension[ext] { return id }
        }
        if let line = firstLine, line.hasPrefix("#!") {
            for (needle, id) in shebang where line.contains(needle) { return id }
        }
        return nil
    }
}
