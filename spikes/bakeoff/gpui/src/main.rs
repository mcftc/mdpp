use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::*;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
static T0: OnceLock<Instant> = OnceLock::new();
static FIRST: AtomicBool = AtomicBool::new(true);
struct Example { editor: Entity<TextareaState> }
impl Render for Example {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if FIRST.swap(false, Ordering::Relaxed) { eprintln!("FIRST_RENDER_MS {}", T0.get().unwrap().elapsed().as_millis()); }
        div().size_full().child(Textarea::new(&self.editor).bordered(false).h(relative(1.)))
    }
}
fn main() {
    T0.set(Instant::now()).unwrap();
    let path = std::env::args().nth(1).expect("usage: bake-gpui <file>");
    let text = std::fs::read_to_string(&path).expect("read");
    eprintln!("LOADED_MS {}", T0.get().unwrap().elapsed().as_millis());
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Example {
                editor: cx.new(|cx| TextareaState::new(window, cx).soft_wrap(false).default_value(text.clone())),
            })
        })
        .expect("window");
    });
}
