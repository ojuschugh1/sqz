# Fish shell completions for sqz — context intelligence layer
# Install: cp completions/sqz.fish ~/.config/fish/completions/sqz.fish
# Or: sqz init (auto-installs completions)

# Disable file completions for all sqz subcommands
complete -c sqz -f

# Top-level subcommands
complete -c sqz -n '__fish_use_subcommand' -a 'init'       -d 'Install shell hooks and create default presets'
complete -c sqz -n '__fish_use_subcommand' -a 'compress'   -d 'Compress text from stdin or argument'
complete -c sqz -n '__fish_use_subcommand' -a 'export'     -d 'Export a session to CTX format'
complete -c sqz -n '__fish_use_subcommand' -a 'import'     -d 'Import a CTX file into the session store'
complete -c sqz -n '__fish_use_subcommand' -a 'status'     -d 'Show current token budget and usage'
complete -c sqz -n '__fish_use_subcommand' -a 'cost'       -d 'Show USD cost breakdown for a session'
complete -c sqz -n '__fish_use_subcommand' -a 'analyze'    -d 'Analyze per-block Shannon entropy'
complete -c sqz -n '__fish_use_subcommand' -a 'tee'        -d 'List and retrieve saved uncompressed outputs'
complete -c sqz -n '__fish_use_subcommand' -a 'dashboard'  -d 'Launch local web dashboard'
complete -c sqz -n '__fish_use_subcommand' -a 'proxy'      -d '[Coming soon] Transparent HTTP proxy'
complete -c sqz -n '__fish_use_subcommand' -a 'uninstall'  -d 'Remove sqz shell hooks from RC file'
complete -c sqz -n '__fish_use_subcommand' -a 'help'       -d 'Print help'

# compress: accepts text argument or reads from stdin
complete -c sqz -n '__fish_seen_subcommand_from compress' -a '(echo)' -d 'Text to compress'

# export: session ID argument
complete -c sqz -n '__fish_seen_subcommand_from export' -a '(sqz status 2>/dev/null | grep session | awk "{print \$2}")' -d 'Session ID'

# import: file path
complete -c sqz -n '__fish_seen_subcommand_from import' -F

# analyze: file path + options
complete -c sqz -n '__fish_seen_subcommand_from analyze' -F
complete -c sqz -n '__fish_seen_subcommand_from analyze' -l high -d 'High-percentile threshold (default 60)' -r
complete -c sqz -n '__fish_seen_subcommand_from analyze' -l low  -d 'Low-percentile threshold (default 25)' -r

# tee subcommands
complete -c sqz -n '__fish_seen_subcommand_from tee' -a 'list' -d 'List all saved tee entries'
complete -c sqz -n '__fish_seen_subcommand_from tee' -a 'get'  -d 'Retrieve a saved output by ID'

# dashboard: port option
complete -c sqz -n '__fish_seen_subcommand_from dashboard' -l port -d 'Port to serve on (default 3001)' -r

# proxy: port option
complete -c sqz -n '__fish_seen_subcommand_from proxy' -l port -d 'Port to listen on (default 8080)' -r

# Global options
complete -c sqz -s h -l help    -d 'Print help'
complete -c sqz -s V -l version -d 'Print version'
