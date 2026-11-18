//! Opt-in timings for an opened folder: `cargo test --release -p mdpp-core --test folder_timing
//! -- --ignored --nocapture` with MDPP_FOLDER set to a large folder.

use std::time::Instant;

use mdpp_core::{folder, fuzzy};

#[test]
#[ignore = "timing; needs MDPP_FOLDER"]
fn listing_and_matching_a_large_folder() {
    let root = std::path::PathBuf::from(std::env::var("MDPP_FOLDER").expect("MDPP_FOLDER"));
    let started = Instant::now();
    let listing = folder::list(&root, 5000);
    let listed = started.elapsed();
    let names: Vec<String> = listing
        .files
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let started = Instant::now();
    let tree = folder::Tree::new(&listing);
    let built = started.elapsed();
    for query in ["n", "note", "d7n42", "zzz"] {
        let started = Instant::now();
        let matched = fuzzy::rank(query, names.iter().map(String::as_str)).len();
        println!(
            "rank {query:?}: {matched} matches in {:?}",
            started.elapsed()
        );
    }
    println!(
        "{} files listed in {listed:?}, tree of {} rows in {built:?}",
        listing.files.len(),
        tree.nodes.len()
    );
}
