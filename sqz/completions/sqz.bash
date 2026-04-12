# Bash completions for sqz — context intelligence layer
# Install: source completions/sqz.bash
# Or add to ~/.bashrc: source /path/to/sqz.bash

_sqz_completions() {
    local cur prev words cword
    _init_completion || return

    local subcommands="init compress export import status cost analyze tee dashboard proxy uninstall help"

    case "$cword" in
        1)
            COMPREPLY=($(compgen -W "$subcommands" -- "$cur"))
            ;;
        2)
            case "$prev" in
                import|analyze)
                    COMPREPLY=($(compgen -f -- "$cur"))
                    ;;
                tee)
                    COMPREPLY=($(compgen -W "list get" -- "$cur"))
                    ;;
            esac
            ;;
        *)
            case "${words[1]}" in
                analyze)
                    COMPREPLY=($(compgen -W "--high --low" -- "$cur"))
                    ;;
                dashboard)
                    COMPREPLY=($(compgen -W "--port" -- "$cur"))
                    ;;
                proxy)
                    COMPREPLY=($(compgen -W "--port" -- "$cur"))
                    ;;
            esac
            ;;
    esac
}

complete -F _sqz_completions sqz
