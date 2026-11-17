use std::time::Instant;
use tree_sitter::{InputEdit, Parser, Point};

fn point_at(doc: &str, byte: usize) -> Point {
    let before = &doc[..byte];
    let row = before.matches('\n').count();
    let col = byte - before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    Point { row, column: col }
}
fn median(mut v: Vec<f64>) -> f64 { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); v[v.len() / 2] }

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let mut doc = std::fs::read_to_string(&path).unwrap();
    println!("doc {} bytes, {} lines", doc.len(), doc.lines().count());

    // pulldown-cmark full parse (with offsets), 10 runs
    let mut t = vec![];
    for _ in 0..10 {
        let t0 = Instant::now();
        let n = pulldown_cmark::Parser::new_ext(&doc, pulldown_cmark::Options::all()).into_offset_iter().count();
        t.push(t0.elapsed().as_secs_f64() * 1e3); std::hint::black_box(n);
    }
    println!("pulldown-cmark full parse (offset_iter): median {:.2} ms", median(t));

    // tree-sitter block grammar only
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_md::LANGUAGE.into()).unwrap();
    let t0 = Instant::now();
    let mut tree = parser.parse(&doc, None).unwrap();
    println!("tree-sitter-md BLOCK full parse: {:.2} ms", t0.elapsed().as_secs_f64() * 1e3);

    // 20 single-char inserts at different places inside paragraphs, correct InputEdit points
    let mut inc = vec![];
    for i in 0..20 {
        let mut b = doc.len() * (i + 1) / 22;
        // move to a char boundary right after "Lorem " on the next paragraph line
        b = b + doc[b..].find("Lorem ").unwrap() + 6;
        let p = point_at(&doc, b);
        doc.insert(b, 'x');
        tree.edit(&InputEdit { start_byte: b, old_end_byte: b, new_end_byte: b + 1,
            start_position: p, old_end_position: p, new_end_position: Point { row: p.row, column: p.column + 1 } });
        let t0 = Instant::now();
        tree = parser.parse(&doc, Some(&tree)).unwrap();
        inc.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    println!("tree-sitter-md BLOCK incremental (20 edits): median {:.3} ms, max {:.3} ms", median(inc.clone()), inc.iter().cloned().fold(0./0., f64::max));

    // Full block+inline via MarkdownParser
    let mut mp = tree_sitter_md::MarkdownParser::default();
    let t0 = Instant::now();
    let mtree = mp.parse(doc.as_bytes(), None).unwrap();
    println!("tree-sitter-md BLOCK+INLINE full parse: {:.2} ms", t0.elapsed().as_secs_f64() * 1e3);
    std::hint::black_box(mtree);
}
