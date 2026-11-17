//! HTML export (PLAN §4.2): one self-contained file — the rendered Markdown plus the stylesheet in
//! `assets/export.css`, which follows the reader's light/dark preference.

use pulldown_cmark::{Parser, html};

/// The stylesheet embedded in every export (data, not code).
pub const EXPORT_CSS: &str = include_str!("../../../assets/export.css");

/// A complete HTML document for `source`, titled `title` (escaped here).
pub fn to_html(source: &str, title: &str) -> String {
    let mut body = String::with_capacity(source.len() * 3 / 2);
    html::push_html(&mut body, Parser::new_ext(source, crate::parser_options()));
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"generator\" content=\"MD++\">\n<title>{}</title>\n<style>\n{EXPORT_CSS}</style>\n\
         </head>\n<body>\n{body}</body>\n</html>\n",
        escape(title)
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_a_complete_document() {
        let html = to_html(
            "# Hi & bye\n\n| a |\n|---|\n| 1 |\n\n- [x] done\n",
            "Notes & <draft>",
        );
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>Notes &amp; &lt;draft&gt;</title>"));
        assert!(html.contains("<h1>Hi &amp; bye</h1>"));
        assert!(html.contains("<table>"));
        assert!(html.contains("checked"));
        assert!(html.contains("prefers-color-scheme: dark"));
    }
}
