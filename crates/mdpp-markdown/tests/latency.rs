//! Background parse budgets (PLAN §5): the structure index and the preview blocks are rebuilt after
//! each typing pause, so on a 1 MB document both must stay well inside one debounce interval.
//! Opt-in timing test: `cargo test --release -p mdpp-markdown -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use mdpp_markdown::{DocIndex, blocks};

const DOC_BYTES: usize = 1_000_000;
/// PLAN §5 (re-baselined in Phase 0 on this dense document): index ≤ 40 ms and preview blocks
/// ≤ 60 ms per MB, in the background after the typing pause — never on the UI thread.
const INDEX_BUDGET: Duration = Duration::from_millis(40);
const PREVIEW_BUDGET: Duration = Duration::from_millis(60);

fn document() -> String {
    let mut doc = String::with_capacity(DOC_BYTES + 1024);
    let mut n = 0;
    while doc.len() < DOC_BYTES {
        doc.push_str(&format!(
            "# Section {n}\n\nText **bold** [link](./x.md) `code`.\n\n- [ ] task\n- item\n\n\
             | a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet x = {n};\n```\n\n"
        ));
        n += 1;
    }
    doc
}

fn p50(mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..9)
        .map(|_| {
            let started = Instant::now();
            f();
            started.elapsed()
        })
        .collect();
    times.sort();
    times[times.len() / 2]
}

#[test]
#[ignore = "timing test; run in release with --ignored"]
fn one_megabyte_parses_within_budget() {
    let doc = document();
    let index = p50(|| drop(DocIndex::build(&doc)));
    let preview = p50(|| drop(blocks::parse(&doc)));
    println!("1 MB: index p50={index:?}  preview blocks p50={preview:?}");
    assert!(
        index <= INDEX_BUDGET,
        "index {index:?} over {INDEX_BUDGET:?}"
    );
    assert!(
        preview <= PREVIEW_BUDGET,
        "preview blocks {preview:?} over {PREVIEW_BUDGET:?}"
    );
}
