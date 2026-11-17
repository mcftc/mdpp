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

/// Fenced-code colouring (PLAN D5): what loading the syntax set costs (once, in the background)
/// and how long a code block takes per line — the basis of the `code_highlight_max_lines` default.
#[test]
#[ignore = "timing; run explicitly with --release"]
fn code_highlighting_cost() {
    use mdpp_markdown::code;
    let started = Instant::now();
    let syntaxes = code::load();
    let load = started.elapsed();
    let rust: Vec<String> = (0..1000)
        .map(|n| {
            format!(
                "    let value_{n}: Vec<u32> = items.iter().map(|x| x * {n}).collect(); // note {n}"
            )
        })
        .collect();
    let per_language = ["rust", "python", "js", "json", "bash", "yaml"].map(|language| {
        let time = p50(|| {
            code::highlight_lines(syntaxes, language, rust.iter().map(String::as_str));
        });
        format!(
            "{language} {:.1} µs/line",
            time.as_secs_f64() * 1e6 / rust.len() as f64
        )
    });
    println!("syntax set load {load:?}; {}", per_language.join(", "));
}
