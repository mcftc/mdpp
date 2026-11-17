# Bake-off spikes

Throw-away code behind the decisions in [`docs/RESEARCH.md`](../../docs/RESEARCH.md) §2–§3. Not shipped.

| Dir | What it is |
|---|---|
| `egui/`, `iced/`, `gpui/` | The same minimal app in three toolkits: open a file into the toolkit's multi-line editor, print time to first frame |
| `tsbench/` | pulldown-cmark vs tree-sitter-md full and incremental parse on the same file |
| `measure.sh` | Runs one app, reports binary size, RSS after 6 s / 16 s, idle CPU over 10 s and the first-frame time |
| `gen-sample.py` | Generates the 1 MB test document |

Reproduce (Linux, from this directory):

```sh
python3 gen-sample.py > sample-1mb.md
for t in egui iced gpui; do (cd $t && cargo build --release); done
for t in egui iced gpui; do ./measure.sh $t "$PWD/$t/target/release/bake-$t" "$PWD/sample-1mb.md"; done
(cd tsbench && cargo build --release && ./target/release/tsbench ../sample-1mb.md)
```

`measure.sh` writes into `results/` and needs a graphical session. The ship-profile sizes come from
`cargo build --profile ship` (profile defined in each `Cargo.toml`).
