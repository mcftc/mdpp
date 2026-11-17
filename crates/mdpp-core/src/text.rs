//! Turning file bytes into editor text and back without changing what the user did not touch
//! (PLAN principle 1, §3.3).
//!
//! The editor buffer is always UTF-8 with LF line breaks. On load we record the file's encoding,
//! byte-order mark and line-ending style; on save we restore them, so a file with uniform line
//! endings round-trips byte for byte. Encoding detection: a BOM wins; otherwise valid UTF-8 is
//! UTF-8; otherwise `chardetng` (Firefox's detector) guesses a legacy encoding.

use std::fmt;

pub use encoding_rs::{Encoding, UTF_8};
use encoding_rs::{UTF_16BE, UTF_16LE};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const UTF16LE_BOM: &[u8] = b"\xFF\xFE";
const UTF16BE_BOM: &[u8] = b"\xFE\xFF";

/// Encodings offered in "Reopen with" / "Save with", one WHATWG label per line.
pub const COMMON_ENCODINGS: &str = include_str!("../../../assets/encodings.txt");

/// Line-ending style of a document.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Eol {
    #[default]
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    /// Every line-ending style, as offered for conversion.
    pub const ALL: [Eol; 3] = [Eol::Lf, Eol::CrLf, Eol::Cr];

    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
            Eol::Cr => "\r",
        }
    }

    /// The platform's conventional line ending, used for new documents.
    pub fn native() -> Self {
        if cfg!(windows) { Eol::CrLf } else { Eol::Lf }
    }
}

impl fmt::Display for Eol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Eol::Lf => "LF",
            Eol::CrLf => "CRLF",
            Eol::Cr => "CR",
        })
    }
}

/// How many line breaks of each style a text contains.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EolCounts {
    pub lf: usize,
    pub crlf: usize,
    pub cr: usize,
}

impl EolCounts {
    pub fn of(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut counts = EolCounts::default();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                    counts.crlf += 1;
                    i += 1;
                }
                b'\r' => counts.cr += 1,
                b'\n' => counts.lf += 1,
                _ => {}
            }
            i += 1;
        }
        counts
    }

    /// More than one style present.
    pub fn is_mixed(&self) -> bool {
        [self.lf, self.crlf, self.cr]
            .iter()
            .filter(|&&n| n > 0)
            .count()
            > 1
    }

    /// The most frequent style; `None` when the text has no line breaks.
    pub fn dominant(&self) -> Option<Eol> {
        let max = self.lf.max(self.crlf).max(self.cr);
        if max == 0 {
            None
        } else if self.crlf == max {
            Some(Eol::CrLf)
        } else if self.lf == max {
            Some(Eol::Lf)
        } else {
            Some(Eol::Cr)
        }
    }
}

/// Text decoded from a file, ready for the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedText {
    /// Content with LF line endings and without BOM.
    pub text: String,
    pub encoding: &'static Encoding,
    /// Line ending to write back on save.
    pub eol: Eol,
    /// The file had more than one line-ending style; saving will use `eol` (PLAN §3.3 — the one
    /// documented, visible exception to principle 1; the UI tells the user).
    pub mixed_eol: bool,
    pub bom: bool,
    /// Some bytes were not valid in `encoding` and became U+FFFD (try "Reopen with").
    pub had_errors: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// NUL bytes in the first block: almost certainly not a text file.
    Binary,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Binary => f.write_str("this looks like a binary file, not text"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Bytes inspected for NULs when deciding whether a file is binary.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Decode file bytes for the editor, detecting the encoding. `default_eol` is used when the text
/// has no line breaks.
pub fn decode(bytes: &[u8], default_eol: Eol) -> Result<LoadedText, DecodeError> {
    let encoding = if let Some((encoding, _)) = Encoding::for_bom(bytes) {
        encoding
    } else {
        if bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
            return Err(DecodeError::Binary);
        }
        if std::str::from_utf8(bytes).is_ok() {
            UTF_8
        } else {
            // Not valid UTF-8 (checked above), so UTF-8 is not a candidate; a text editor has no
            // script-injection concern, so ISO-2022-JP may be guessed.
            let mut detector =
                chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
            detector.feed(bytes, true);
            detector.guess(None, chardetng::Utf8Detection::Deny)
        }
    };
    Ok(decode_as(bytes, encoding, default_eol))
}

/// Decode with a given encoding ("Reopen with"). A BOM for that encoding is honoured and recorded.
pub fn decode_as(bytes: &[u8], encoding: &'static Encoding, default_eol: Eol) -> LoadedText {
    let bom_len = match Encoding::for_bom(bytes) {
        Some((found, len)) if found == encoding => len,
        _ => 0,
    };
    let (raw, had_errors) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
    let counts = EolCounts::of(&raw);
    let text = if counts.crlf == 0 && counts.cr == 0 {
        raw.into_owned()
    } else {
        raw.replace("\r\n", "\n").replace('\r', "\n")
    };
    LoadedText {
        text,
        encoding,
        eol: counts.dominant().unwrap_or(default_eol),
        mixed_eol: counts.is_mixed(),
        bom: bom_len > 0,
        had_errors,
    }
}

/// A character the target encoding cannot represent; saving would lose it.
#[derive(Debug, PartialEq, Eq)]
pub struct Unrepresentable {
    pub character: char,
    pub encoding: &'static str,
}

impl fmt::Display for Unrepresentable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` cannot be written in {}; save with UTF-8 to keep it",
            self.character, self.encoding
        )
    }
}

impl std::error::Error for Unrepresentable {}

/// Encode editor text (LF line endings) for writing to disk. Fails instead of silently replacing
/// characters the encoding cannot represent.
pub fn encode(
    text: &str,
    eol: Eol,
    bom: bool,
    encoding: &'static Encoding,
) -> Result<Vec<u8>, Unrepresentable> {
    let with_eol;
    let text = if eol == Eol::Lf {
        text
    } else {
        with_eol = text.replace('\n', eol.as_str());
        with_eol.as_str()
    };
    let mut out = Vec::with_capacity(text.len() + 3);
    if encoding == UTF_16LE || encoding == UTF_16BE {
        // encoding_rs only decodes UTF-16 (the Web never writes it), so encode here.
        let little = encoding == UTF_16LE;
        if bom {
            out.extend_from_slice(if little { UTF16LE_BOM } else { UTF16BE_BOM });
        }
        for unit in text.encode_utf16() {
            out.extend_from_slice(&if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        return Ok(out);
    }
    if encoding == UTF_8 {
        if bom {
            out.extend_from_slice(UTF8_BOM);
        }
        out.extend_from_slice(text.as_bytes());
        return Ok(out);
    }
    let (bytes, _, had_errors) = encoding.encode(text);
    if had_errors {
        // Name the first character that does not survive, so the user knows what to change.
        let character = text
            .chars()
            .find(|c| encoding.encode(c.encode_utf8(&mut [0; 4])).2)
            .unwrap_or('\u{FFFD}');
        return Err(Unrepresentable {
            character,
            encoding: encoding.name(),
        });
    }
    out.extend_from_slice(&bytes);
    Ok(out)
}

/// The labels of [`COMMON_ENCODINGS`].
pub fn common_encodings() -> impl Iterator<Item = &'static str> {
    COMMON_ENCODINGS
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
}

/// Whether files in `encoding` customarily start with a byte-order mark (UTF-16 does).
pub fn wants_bom(encoding: &'static Encoding) -> bool {
    encoding == UTF_16LE || encoding == UTF_16BE
}

/// Look up an encoding by label (`UTF-8`, `windows-1254`, `latin5`, …).
pub fn encoding_for_label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.trim().as_bytes())
}

/// Cheap identity of a text, to tell whether an edit history has returned to the saved state
/// (PLAN §3.3: undoing back to the saved text clears the dirty marker). The length is compared
/// first, so the hash only runs when lengths match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    len: usize,
    hash: u64,
}

impl Fingerprint {
    pub fn of_chunks<'a>(chunks: impl IntoIterator<Item = &'a str>) -> Self {
        use std::hash::Hasher;
        let mut hasher = std::hash::DefaultHasher::new();
        let mut len = 0;
        for chunk in chunks {
            len += chunk.len();
            hasher.write(chunk.as_bytes());
        }
        Fingerprint {
            len,
            hash: hasher.finish(),
        }
    }

    /// Whether `chunks` (of total length `len`) still match this fingerprint.
    pub fn matches<'a>(&self, len: usize, chunks: impl IntoIterator<Item = &'a str>) -> bool {
        len == self.len && Fingerprint::of_chunks(chunks) == *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8]) -> Vec<u8> {
        let loaded = decode(bytes, Eol::Lf).unwrap();
        encode(&loaded.text, loaded.eol, loaded.bom, loaded.encoding).unwrap()
    }

    #[test]
    fn uniform_files_round_trip_exactly() {
        for case in [
            &b""[..],
            b"no newline",
            b"a\nb\n",
            b"a\r\nb\r\n",
            b"a\rb\r",
            b"\xEF\xBB\xBF# t\r\n\r\nbody",
            "T\u{fc}rk\u{e7}e \u{0130}\n".as_bytes(),
        ] {
            assert_eq!(
                round_trip(case),
                case,
                "{:?}",
                String::from_utf8_lossy(case)
            );
        }
    }

    #[test]
    fn buffer_is_always_lf() {
        let loaded = decode(b"a\r\nb\rc\n", Eol::Lf).unwrap();
        assert_eq!(loaded.text, "a\nb\nc\n");
        assert!(loaded.mixed_eol);
    }

    #[test]
    fn mixed_files_save_with_the_dominant_style() {
        let loaded = decode(b"a\r\nb\r\nc\nd", Eol::Lf).unwrap();
        assert_eq!(loaded.eol, Eol::CrLf);
        assert_eq!(
            encode(&loaded.text, loaded.eol, false, UTF_8).unwrap(),
            b"a\r\nb\r\nc\r\nd"
        );
    }

    #[test]
    fn no_line_breaks_uses_the_default() {
        assert_eq!(decode(b"x", Eol::CrLf).unwrap().eol, Eol::CrLf);
        assert!(!decode(b"x", Eol::CrLf).unwrap().mixed_eol);
    }

    #[test]
    fn legacy_turkish_file_is_detected_and_round_trips() {
        let windows_1254 = encoding_for_label("windows-1254").unwrap();
        let text =
            "Şu çalışma dosyası İstanbul'da yazıldı; ğüşıöç ĞÜŞİÖÇ harfleri doğru görünmeli.\r\n";
        let (bytes, _, _) = windows_1254.encode(text);
        let loaded = decode(&bytes, Eol::Lf).unwrap();
        assert!(
            loaded.encoding == windows_1254 || loaded.encoding.name() == "ISO-8859-9",
            "detected {}",
            loaded.encoding.name()
        );
        assert_eq!(loaded.text, text.replace("\r\n", "\n"));
        assert_eq!(round_trip(&bytes), bytes.to_vec());
    }

    #[test]
    fn utf16_with_bom_round_trips() {
        for (encoding, bom) in [(UTF_16LE, UTF16LE_BOM), (UTF_16BE, UTF16BE_BOM)] {
            let mut bytes = bom.to_vec();
            for unit in "ğ\r\nx".encode_utf16() {
                let pair = if encoding == UTF_16LE {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                };
                bytes.extend_from_slice(&pair);
            }
            let loaded = decode(&bytes, Eol::Lf).unwrap();
            assert_eq!(
                (loaded.encoding, loaded.text.as_str(), loaded.bom),
                (encoding, "ğ\nx", true)
            );
            assert_eq!(round_trip(&bytes), bytes);
        }
    }

    #[test]
    fn reopen_with_another_encoding() {
        let bytes = "çay".as_bytes(); // UTF-8 bytes
        let latin1 = encoding_for_label("ISO-8859-1").unwrap();
        assert_eq!(decode_as(bytes, latin1, Eol::Lf).text, "Ã§ay");
    }

    #[test]
    fn unrepresentable_characters_are_refused() {
        let latin1 = encoding_for_label("ISO-8859-1").unwrap();
        let error = encode("café ğ", Eol::Lf, false, latin1).unwrap_err();
        assert_eq!(error.character, 'ğ');
        assert_eq!(encode("café", Eol::Lf, false, latin1).unwrap(), b"caf\xe9");
    }

    #[test]
    fn binary_files_are_refused() {
        assert_eq!(decode(b"PNG\0\0data", Eol::Lf), Err(DecodeError::Binary));
    }

    #[test]
    fn common_encodings_are_known_labels() {
        for label in common_encodings() {
            assert!(encoding_for_label(label).is_some(), "unknown label {label}");
        }
    }

    #[test]
    fn fingerprints_ignore_chunking() {
        let whole = Fingerprint::of_chunks(["hello world"]);
        assert_eq!(whole, Fingerprint::of_chunks(["hello", " ", "world"]));
        assert!(whole.matches(11, ["hello ", "world"]));
        assert!(!whole.matches(11, ["hello_world"]));
        assert!(!whole.matches(10, ["hello worl"]));
    }
}
