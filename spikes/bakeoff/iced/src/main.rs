use iced::widget::text_editor;
use iced::{Element, Length};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
static T0: OnceLock<Instant> = OnceLock::new();
static FIRST: AtomicBool = AtomicBool::new(true);
struct App { content: text_editor::Content }
#[derive(Debug, Clone)]
enum Msg { Edit(text_editor::Action) }
fn update(app: &mut App, msg: Msg) { match msg { Msg::Edit(a) => app.content.perform(a) } }
fn view(app: &App) -> Element<'_, Msg> {
    if FIRST.swap(false, Ordering::Relaxed) { eprintln!("FIRST_VIEW_MS {}", T0.get().unwrap().elapsed().as_millis()); }
    text_editor(&app.content).on_action(Msg::Edit).height(Length::Fill).into()
}
fn main() -> iced::Result {
    T0.set(Instant::now()).unwrap();
    let path = std::env::args().nth(1).expect("usage: bake-iced <file>");
    let text = std::fs::read_to_string(&path).expect("read");
    eprintln!("LOADED_MS {}", T0.get().unwrap().elapsed().as_millis());
    iced::application(move || App { content: text_editor::Content::with_text(&text) }, update, view)
        .title("bake-iced")
        .run()
}
