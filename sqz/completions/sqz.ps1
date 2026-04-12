# PowerShell completions for sqz — context intelligence layer
# Install: sqz init (auto-installs)
# Or manually: Add-Content $PROFILE (Get-Content completions/sqz.ps1)

Register-ArgumentCompleter -Native -CommandName sqz -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $subcommands = @{
        'init'      = 'Install shell hooks and create default presets'
        'compress'  = 'Compress text from stdin or argument'
        'export'    = 'Export a session to CTX format'
        'import'    = 'Import a CTX file into the session store'
        'status'    = 'Show current token budget and usage'
        'cost'      = 'Show USD cost breakdown for a session'
        'analyze'   = 'Analyze per-block Shannon entropy'
        'tee'       = 'List and retrieve saved uncompressed outputs'
        'dashboard' = 'Launch local web dashboard'
        'proxy'     = '[Coming soon] Transparent HTTP proxy'
        'uninstall' = 'Remove sqz shell hooks from RC file'
        'help'      = 'Print help'
    }

    $tokens = $commandAst.CommandElements
    $subcommand = if ($tokens.Count -ge 2) { $tokens[1].Value } else { $null }

    if ($null -eq $subcommand -or $tokens.Count -eq 1) {
        # Complete subcommands
        $subcommands.GetEnumerator() |
            Where-Object { $_.Key -like "$wordToComplete*" } |
            ForEach-Object {
                [System.Management.Automation.CompletionResult]::new(
                    $_.Key, $_.Key,
                    [System.Management.Automation.CompletionResultType]::ParameterValue,
                    $_.Value
                )
            }
        return
    }

    # Subcommand-specific completions
    switch ($subcommand) {
        'tee' {
            @('list', 'get') |
                Where-Object { $_ -like "$wordToComplete*" } |
                ForEach-Object {
                    [System.Management.Automation.CompletionResult]::new(
                        $_, $_, 'ParameterValue', $_
                    )
                }
        }
        'import' {
            # File path completion
            Get-ChildItem -Path "$wordToComplete*" -Filter '*.ctx' |
                ForEach-Object {
                    [System.Management.Automation.CompletionResult]::new(
                        $_.FullName, $_.Name, 'ParameterValue', $_.FullName
                    )
                }
        }
        'analyze' {
            @('--high', '--low') |
                Where-Object { $_ -like "$wordToComplete*" } |
                ForEach-Object {
                    [System.Management.Automation.CompletionResult]::new(
                        $_, $_, 'ParameterName', $_
                    )
                }
        }
        { $_ -in 'dashboard', 'proxy' } {
            '--port' |
                Where-Object { $_ -like "$wordToComplete*" } |
                ForEach-Object {
                    [System.Management.Automation.CompletionResult]::new(
                        $_, $_, 'ParameterName', 'Port number'
                    )
                }
        }
    }
}
