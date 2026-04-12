# Nushell completions for sqz — context intelligence layer
# Install: sqz init (auto-installs)
# Or manually: cp completions/sqz.nu ~/.config/nushell/completions/sqz.nu
# Then add to config.nu: source ~/.config/nushell/completions/sqz.nu

def "nu-complete sqz subcommands" [] {
    [
        [value description];
        ["init"       "Install shell hooks and create default presets"]
        ["compress"   "Compress text from stdin or argument"]
        ["export"     "Export a session to CTX format"]
        ["import"     "Import a CTX file into the session store"]
        ["status"     "Show current token budget and usage"]
        ["cost"       "Show USD cost breakdown for a session"]
        ["analyze"    "Analyze per-block Shannon entropy"]
        ["tee"        "List and retrieve saved uncompressed outputs"]
        ["dashboard"  "Launch local web dashboard"]
        ["proxy"      "[Coming soon] Transparent HTTP proxy"]
        ["uninstall"  "Remove sqz shell hooks from RC file"]
        ["help"       "Print help"]
    ]
}

def "nu-complete sqz tee subcommands" [] {
    [
        [value description];
        ["list" "List all saved tee entries"]
        ["get"  "Retrieve a saved output by ID"]
    ]
}

export extern "sqz" [
    subcommand?: string@"nu-complete sqz subcommands"
    --help(-h)    # Print help
    --version(-V) # Print version
]

export extern "sqz init" [
    --help(-h) # Print help
]

export extern "sqz compress" [
    text?: string  # Text to compress (reads stdin if omitted)
    --help(-h)     # Print help
]

export extern "sqz export" [
    session_id: string # Session ID to export
    --help(-h)         # Print help
]

export extern "sqz import" [
    file: path     # Path to .ctx file
    --help(-h)     # Print help
]

export extern "sqz status" [
    --help(-h) # Print help
]

export extern "sqz cost" [
    session_id: string # Session ID
    --help(-h)         # Print help
]

export extern "sqz analyze" [
    file?: path          # File to analyze (reads stdin if omitted)
    --high: float = 60.0 # High-percentile threshold
    --low: float = 25.0  # Low-percentile threshold
    --help(-h)           # Print help
]

export extern "sqz tee" [
    subcommand?: string@"nu-complete sqz tee subcommands"
    --help(-h) # Print help
]

export extern "sqz dashboard" [
    --port: int = 3001 # Port to serve on
    --help(-h)         # Print help
]

export extern "sqz proxy" [
    --port: int = 8080 # Port to listen on
    --help(-h)         # Print help
]

export extern "sqz uninstall" [
    --help(-h) # Print help
]
