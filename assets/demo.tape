# Regenerate with: vhs assets/demo.tape   (from the repo root, after `cargo build --release`)
# Everything shown is real output from the release binary against a throwaway database.
Output assets/demo.gif

Set Shell bash
Set FontSize 15
Set Width 1000
Set Height 620
Set Theme "Catppuccin Mocha"
Set Padding 16
Set TypingSpeed 35ms
Set WaitTimeout 15s
Set WaitPattern /^\$ ?$/

Hide
Type `export PATH="$PWD/target/release:$PATH" PS1='$ ' SQZ_DB_PATH="$(mktemp -d)/demo.db" NO_COLOR=1`
Enter
Wait
Type `sqz --version >/dev/null; clear`
Enter
Wait
Show

Sleep 800ms
Type `# the agent reads a 200-line file`
Enter
Wait
Type `cat demo/auth.py | sqz compress --cmd "cat auth.py" | head -4`
Enter
Wait
Sleep 2s

Type `# then re-reads lines 41-80 of it`
Enter
Wait
Type `sed -n '41,80p' demo/auth.py | sqz compress --cmd "sed -n '41,80p' auth.py"`
Enter
Wait
Sleep 3s

Type `# and runs the tests`
Enter
Wait
Type `cat demo/cargo-test.txt | sqz compress --cmd "cargo test"`
Enter
Wait
Sleep 3.5s

Type `sqz stats --breakdown | tail -n 6`
Enter
Wait
Sleep 5s
