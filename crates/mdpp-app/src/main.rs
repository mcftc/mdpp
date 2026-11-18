//! MD++ — a lightweight Markdown editor. See docs/PLAN.md.
//!
//! Environment: `MDPP_DATA_DIR` overrides where the session and backups live (tests, portable
//! use); `MDPP_BENCH` runs a built-in measurement (see `bench.rs`) — then persistence is off
//! unless `MDPP_DATA_DIR` is set, so measurements never touch a real session. `MDPP_INSTANCE`
//! names the single-instance socket (separate test instances from a real one).

mod bench;
mod code_actions;
mod completion;
mod document;
mod find;
mod highlight;
mod menus;
mod navigation;
mod palette;
mod preview;
mod prompt;
mod search_panel;
mod settings_window;
mod snippets;
mod watch;
mod workspace;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::Theme;
use gpui_kit::component::input::CursorBlink;
use gpui_kit::{AppContext as _, WindowBounds, WindowOptions, px, size};
use mdpp_core::cli::parse_args;
use mdpp_core::instance::{self, Claim};
use mdpp_core::paths::Paths;
use mdpp_core::session::Store;
use mdpp_core::settings::Settings;

use crate::workspace::{Startup, Workspace};

fn main() {
    let started = Instant::now();
    let bench_mode = bench::Mode::from_env();
    let cwd = std::env::current_dir().unwrap_or_default();
    let files = parse_args(std::env::args_os().skip(1), &cwd);

    let instance_name = std::env::var("MDPP_INSTANCE").unwrap_or_else(|_| instance::default_name());
    // Single instance: hand the files to a running MD++ and stop. Measurements always run their
    // own instance, so they can never reach a user's window.
    let server = match bench_mode {
        Some(_) => None,
        None => match instance::claim(&instance_name, &files) {
            Claim::Forwarded => return,
            Claim::Primary(server) => Some(server),
            Claim::Unavailable(error) => {
                eprintln!("mdpp: running without single instance: {error}");
                None
            }
        },
    };

    // Settings and keymap: shipped defaults overlaid with the user's files, if any.
    let paths = Paths::find();
    // Session, backups and crash reports live in the data folder (overridable for tests).
    let data_dir = match std::env::var_os("MDPP_DATA_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None if bench_mode.is_some() => None,
        None => paths.as_ref().map(|p| p.data.clone()),
    };
    // Crash reports: the next start mentions a new one once (PLAN §4.4).
    let mut warnings = Vec::new();
    if let Some(logs) = data_dir.as_ref().map(|dir| dir.join("logs")) {
        mdpp_core::crash::install(logs.clone(), env!("CARGO_PKG_VERSION"));
        if let Some(report) = mdpp_core::crash::take_unseen(&logs) {
            warnings.push(format!(
                "MD++ closed unexpectedly last time. A crash report was saved to {} — please attach it to a bug report.",
                report.display()
            ));
        }
    }
    let read = |path: Option<PathBuf>| path.and_then(|p| std::fs::read_to_string(p).ok());
    let loaded =
        Settings::load(&read(paths.as_ref().map(Paths::settings_file)).unwrap_or_default());
    let (bindings, keymap_warnings) =
        mdpp_core::keymap::load(read(paths.as_ref().map(Paths::keymap_file)).as_deref());
    let settings = Rc::new(loaded.settings);
    // The session also keeps view state and recent files; `hot_exit` decides whether tabs and
    // their unsaved text are part of it.
    let store = data_dir.map(Store::new);
    warnings.extend(loaded.warnings);
    warnings.extend(keymap_warnings);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            warnings.extend(workspace::bind_keymap(&bindings, cx));
            highlight::configure_language(&settings.auto_close_pairs, cx);

            Theme::update(cx, |theme| {
                theme.cursor_blink = CursorBlink::blinking(settings.cursor_blink_interval)
                    .with_idle_timeout(settings.cursor_blink_idle_timeout);
            });
            // Measurements must not take the keyboard from whoever is using the machine: their
            // window opens without focus (simulated keystrokes still reach it).
            let interactive = bench_mode.is_none();
            if interactive {
                cx.activate(true);
            }

            let window_size = size(px(settings.window_width), px(settings.window_height));
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(window_size, cx)),
                focus: interactive,
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
                {
                    let startup = Startup {
                        files,
                        settings,
                        store,
                        paths,
                        server,
                        warnings,
                        bench_mode,
                        started,
                    };
                    cx.new(|cx| Workspace::new(startup, window, cx))
                }
            });
            if let Err(error) = opened {
                eprintln!("mdpp: could not open a window: {error:#}");
                cx.quit();
            }
        });
}
