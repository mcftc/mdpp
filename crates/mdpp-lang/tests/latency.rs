//! Completion latency budget (PLAN §5, gate G2): context detection + index build + candidates on a
//! 1 MB document must stay well under the 50 ms popup budget. Timing tests are noisy on shared CI
//! runners, so this one is opt-in: `cargo test --release -p mdpp-lang -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use mdpp_lang::{FileSystem, candidates, context_at};
use mdpp_markdown::{BlockState, DocIndex};
use ropey::Rope;

/// PLAN §5: completion p95 ≤ 30 ms, with the index the background analysis keeps current.
const BUDGET: Duration = Duration::from_millis(30);
/// Gate G2: the popup within 50 ms even when the index must be built first (cold start).
const POPUP_BUDGET: Duration = Duration::from_millis(50);
const DOC_BYTES: usize = 1_000_000;

fn document() -> String {
    let mut doc = String::with_capacity(DOC_BYTES + 1024);
    let mut n = 0;
    while doc.len() < DOC_BYTES {
        doc.push_str(&format!(
            "# Section {n}\n\nText with **bold**, a [link](./x.md#a) and `code`.\n\n- item\n- [ ] task\n\n"
        ));
        n += 1;
    }
    doc.push_str("See [the start](#sec");
    doc
}

#[test]
#[ignore = "timing test; run in release with --ignored"]
fn anchor_completion_on_one_megabyte() {
    let text = Rope::from_str(&document());
    let offset = text.len();
    let cached = DocIndex::build(&text.to_string());
    let (mut cold, mut warm) = (Vec::new(), Vec::new());
    for _ in 0..20 {
        let started = Instant::now();
        let context = context_at(&text, offset, BlockState::Normal).expect("anchor context");
        let index = DocIndex::build(&text.to_string());
        assert!(!candidates(&context, Some(&index), &FileSystem, None).is_empty());
        cold.push(started.elapsed());

        let started = Instant::now();
        let context = context_at(&text, offset, BlockState::Normal).expect("anchor context");
        assert!(!candidates(&context, Some(&cached), &FileSystem, None).is_empty());
        warm.push(started.elapsed());
    }
    let p95 = |times: &mut Vec<Duration>| {
        times.sort();
        times[times.len() * 95 / 100]
    };
    let (cold, warm) = (p95(&mut cold), p95(&mut warm));
    println!("anchor completion on 1 MB: p95 warm={warm:?} cold={cold:?}");
    assert!(warm <= BUDGET, "warm p95 {warm:?} over {BUDGET:?}");
    assert!(
        cold <= POPUP_BUDGET,
        "cold p95 {cold:?} over {POPUP_BUDGET:?}"
    );
}

#[test]
#[ignore = "timing test; run in release with --ignored"]
fn emoji_completion() {
    let text = Rope::from_str("Shipping it :roc");
    let started = Instant::now();
    let context = context_at(&text, text.len(), BlockState::Normal).expect("emoji context");
    let items = candidates(&context, None, &FileSystem, None);
    let took = started.elapsed();
    println!("emoji completion: {took:?} ({} items)", items.len());
    assert!(took <= BUDGET);
}
