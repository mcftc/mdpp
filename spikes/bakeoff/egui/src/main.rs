use std::time::Instant;
fn main() -> eframe::Result {
    let t0 = Instant::now();
    let path = std::env::args().nth(1).expect("usage: bake-egui <file>");
    let text = std::fs::read_to_string(&path).expect("read");
    eprintln!("LOADED_MS {}", t0.elapsed().as_millis());
    eframe::run_native(
        "bake-egui",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| Ok(Box::new(App { text, t0, first: true }))),
    )
}
struct App { text: String, t0: Instant, first: bool }
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_sized(ui.available_size(), egui::TextEdit::multiline(&mut self.text).code_editor());
            });
        });
        if self.first { self.first = false; eprintln!("FIRST_FRAME_MS {}", self.t0.elapsed().as_millis()); }
    }
}
