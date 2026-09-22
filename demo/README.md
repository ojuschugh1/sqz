# demo fixtures

Inputs for the README recording (`assets/demo.tape`) and the repeat-read table in the README.

- `auth.py`: a 200-line generated Python module. Read in full, then lines 41-80 re-read.
- `cargo-test.txt`: captured `cargo test` output with one failure.

Regenerate the recording from the repo root after `cargo build --release`:

```sh
vhs assets/demo.tape
```
