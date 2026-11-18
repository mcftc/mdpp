//! Display math in the preview (PLAN §4.2): `$$…$$` laid out by RaTeX (KaTeX-compatible, pure
//! Rust) and drawn as an SVG image in the theme's text colour, with the KaTeX fonts embedded.
//! Inline `$…$` stays styled text: the preview's lines are text runs, an image would break them.
//!
//! Each formula is laid out once per colour and size; the preview redraws from the cache.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::{Hsla, Image, ImageFormat};
use ratex_layout::layout_options::LayoutOptions;
use ratex_layout::to_display::to_display_list;
use ratex_layout::{engine, validate_layout};
use ratex_svg::{SvgOptions, render_to_svg};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;

/// Space around a formula, in em.
const PADDING_EM: f64 = 0.25;

/// Formula + colour + size → its image (or why it could not be laid out).
type Key = (String, [u8; 4], u32);

#[derive(Default)]
pub struct MathCache {
    images: RefCell<HashMap<Key, Result<Arc<Image>, String>>>,
}

impl MathCache {
    /// The image of `tex` in `color`, `em` pixels per em; `Err` carries the parse error.
    pub fn image(&self, tex: &str, color: Hsla, em: f32) -> Result<Arc<Image>, String> {
        let rgba = color.to_rgb();
        let channels = [rgba.r, rgba.g, rgba.b, rgba.a].map(|c| (c * 255.0).round() as u8);
        let key = (tex.to_string(), channels, em.to_bits());
        self.images
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let [r, g, b, a] = channels.map(|c| f32::from(c) / 255.0);
                let svg = render_svg(tex, Color { r, g, b, a }, f64::from(em))?;
                Ok(Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    svg.into_bytes(),
                )))
            })
            .clone()
    }
}

/// The SVG of display formula `tex`.
pub fn render_svg(tex: &str, color: Color, em: f64) -> Result<String, String> {
    let ast = ratex_parser::parser::parse(tex).map_err(|error| error.to_string())?;
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(color);
    let laid_out = engine::layout(&ast, &options);
    validate_layout(&laid_out).map_err(|error| error.to_string())?;
    let options = SvgOptions {
        font_size: em,
        padding: em * PADDING_EM,
        embed_glyphs: true,
        ..SvgOptions::default()
    };
    Ok(render_to_svg(&to_display_list(&laid_out), &options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_become_self_contained_svg() {
        let svg = render_svg(r"\frac{a}{b} + \sqrt{x^2}", Color::BLACK, 20.0).unwrap();
        assert!(svg.starts_with("<svg") && svg.contains("<path"));
        assert!(
            !svg.contains("<text"),
            "glyphs are outlines, no fonts needed"
        );
        assert!(render_svg(r"\frac{a}{", Color::BLACK, 20.0).is_err());
    }

    #[test]
    #[ignore = "timing"]
    fn formula_cost() {
        let tex = r"x = \frac{-b \pm \sqrt{b^2-4ac}}{2a} + \sum_{k=1}^{n} \int_0^1 f(t)\,dt";
        render_svg(tex, Color::BLACK, 20.0).unwrap(); // fonts parsed once
        let started = std::time::Instant::now();
        for _ in 0..20 {
            render_svg(tex, Color::BLACK, 20.0).unwrap();
        }
        println!("formula: {:?} each", started.elapsed() / 20);
    }
}
