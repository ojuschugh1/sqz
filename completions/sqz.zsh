#compdef sqz
# Zsh completions for sqz — context intelligence layer
# Install: cp completions/sqz.zsh ~/.zsh/completions/_sqz
# Or add completions dir to fpath: fpath=(~/.zsh/completions $fpath)

_sqz() {
    local -a subcommands
    subcommands=(
        'init:Install shell hooks and create default presets'
        'compress:Compress text from stdin or argument'
        'export:Export a session to CTX format'
        'import:Import a CTX file into the session store'
        'status:Show current token budget and usage'
        'cost:Show USD cost breakdown for a session'
        'analyze:Analyze per-block Shannon entropy'
        'tee:List and retrieve saved uncompressed outputs'
        'dashboard:Launch local web dashboard'
        'proxy:[Coming soon] Transparent HTTP proxy'
        'uninstall:Remove sqz shell hooks from RC file'
        'help:Print help'
    )

    _arguments -C \
        '(-h --help)'{-h,--help}'[Print help]' \
        '(-V --version)'{-V,--version}'[Print version]' \
        '1: :->subcommand' \
        '*: :->args'

    case $state in
        subcommand)
            _describe 'sqz subcommand' subcommands
            ;;
        args)
            case $words[2] in
                import)
                    _files
                    ;;
                analyze)
                    _arguments \
                        '--high[High-percentile threshold]:threshold:(60)' \
                        '--low[Low-percentile threshold]:threshold:(25)' \
                        ':file:_files'
                    ;;
                dashboard|proxy)
                    _arguments '--port[Port number]:port:(3001 8080)'
                    ;;
                tee)
                    local -a tee_cmds
                    tee_cmds=('list:List all saved tee entries' 'get:Retrieve a saved output by ID')
                    _describe 'tee subcommand' tee_cmds
                    ;;
            esac
            ;;
    esac
}

_sqz "$@"
