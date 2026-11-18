//! Property tests (PLAN §8, Phase 1 exit): saving and reopening never changes the text, whatever
//! the text, line endings, BOM and Unicode encoding; atomic save writes exactly the bytes.

use mdpp_core::text::{Eol, UTF_8, decode_as, encode};
use proptest::prelude::*;

/// Any text as the editor holds it: LF line endings only, no BOM character at the start.
fn editor_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            8 => "[^\r\u{feff}]",
            2 => Just("\n".to_string()),
        ],
        0..200,
    )
    .prop_map(|parts| parts.concat())
}

fn eol() -> impl Strategy<Value = Eol> {
    prop_oneof![Just(Eol::Lf), Just(Eol::CrLf)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn unicode_files_round_trip(text in editor_text(), eol in eol(), bom: bool, which in 0..3usize) {
        let encoding = [UTF_8, encoding_rs::UTF_16LE, encoding_rs::UTF_16BE][which];
        let bytes = encode(&text, eol, bom, encoding).expect("Unicode holds every character");
        let loaded = decode_as(&bytes, encoding, Eol::Lf);
        prop_assert_eq!(&loaded.text, &text);
        prop_assert!(!loaded.had_errors);
        if text.contains('\n') {
            prop_assert_eq!(loaded.eol, eol);
        }
        // Encoding again gives the same bytes: nothing drifts across save cycles.
        let again = encode(&loaded.text, loaded.eol, bom, encoding).unwrap();
        prop_assert_eq!(again, bytes);
    }

    #[test]
    fn atomic_save_writes_exactly_the_bytes(first: Vec<u8>, second: Vec<u8>) {
        let dir = std::env::temp_dir().join(format!("mdpp-prop-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.md");
        mdpp_core::file::save(&path, &first).unwrap();
        prop_assert_eq!(std::fs::read(&path).unwrap(), first);
        mdpp_core::file::save(&path, &second).unwrap();
        prop_assert_eq!(std::fs::read(&path).unwrap(), second);
        // No temporary file is left behind.
        prop_assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
