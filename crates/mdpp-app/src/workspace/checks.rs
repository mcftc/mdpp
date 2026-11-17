//! Built-in measurements and end-to-end checks (`MDPP_BENCH`, see `bench.rs`): they drive the
//! real window through actions and keystrokes and print `BENCH …` lines.

use super::*;
use mdpp_core::text::Eol;

impl Workspace {
    pub(super) fn start_bench(
        &mut self,
        mode: Option<bench::Mode>,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let Some(mode) = mode else {
            return Task::ready(());
        };
        let probe = self.probe.clone();
        cx.spawn_in(window, async move |this, cx| {
            let editor = this
                .update(cx, |this, cx| {
                    this.active_document().read(cx).editor().clone()
                })
                .ok();
            let Some(editor) = editor else {
                return;
            };
            match mode {
                bench::Mode::Startup => {
                    while probe.borrow().first_paint.is_none() {
                        cx.background_executor()
                            .timer(bench::KEYSTROKE_INTERVAL)
                            .await;
                    }
                    let painted = probe.borrow().first_paint.unwrap_or(started);
                    let (docs, dirty) = this
                        .update(cx, |this, cx| {
                            let dirty = this
                                .documents
                                .iter()
                                .filter(|d| d.read(cx).is_dirty())
                                .count();
                            (this.documents.len(), dirty)
                        })
                        .unwrap_or_default();
                    println!(
                        "BENCH startup first_paint={:.1}ms docs={docs} dirty={dirty}",
                        (painted - started).as_secs_f64() * 1e3
                    );
                }
                bench::Mode::Idle(seconds) => {
                    let blink_off = this
                        .update(cx, |this, _| this.settings.cursor_blink_idle_timeout)
                        .unwrap_or_default();
                    cx.background_executor()
                        .timer(bench::SETTLE + blink_off)
                        .await;
                    let (paints, cpu) = (probe.borrow().paints, bench::process_cpu_time());
                    let window = std::time::Duration::from_secs(seconds);
                    cx.background_executor().timer(window).await;
                    let paints = probe.borrow().paints - paints;
                    let cpu = cpu.zip(bench::process_cpu_time()).map(|(a, b)| {
                        format!(
                            "{:.2}%",
                            (b - a).as_secs_f64() * 100.0 / window.as_secs_f64()
                        )
                    });
                    println!(
                        "BENCH idle {seconds}s frames_painted={paints} cpu={}",
                        cpu.as_deref().unwrap_or("n/a")
                    );
                }
                bench::Mode::Check => {
                    Self::check_assists(&this, cx).await;
                    Self::run_check(&this, &editor, cx).await
                }
                bench::Mode::Crash => {
                    cx.background_executor().timer(bench::SETTLE).await;
                    panic!("deliberate crash for the crash-report check");
                }
                bench::Mode::Typing(count, with_preview) => {
                    if with_preview {
                        this.update_in(cx, |_, window, cx| {
                            window.dispatch_action(Box::new(TogglePreview), cx)
                        })
                        .ok();
                    }
                    cx.background_executor().timer(bench::SETTLE).await;
                    editor
                        .update_in(cx, |state, window, cx| {
                            let middle = state.text().len_lines(ropey::LineType::LF) / 2;
                            state.set_cursor_position(Position::new(middle as u32, 0), window, cx);
                        })
                        .ok();
                    for _ in 0..count {
                        editor
                            .update_in(cx, |state, window, cx| {
                                probe.borrow_mut().pending = Some(Instant::now());
                                let frame_probe = probe.clone();
                                // Runs right before the frame is drawn: separates our work from
                                // the wait for the display's next frame.
                                window.on_next_frame(move |_, _| {
                                    frame_probe.borrow_mut().frame_start = Some(Instant::now());
                                });
                                state.insert("x", window, cx);
                            })
                            .ok();
                        cx.background_executor()
                            .timer(bench::KEYSTROKE_INTERVAL)
                            .await;
                    }
                    cx.background_executor().timer(bench::SETTLE).await;
                    println!("BENCH typing {}", probe.borrow().report());
                }
            }
            cx.update(|_, cx| cx.quit()).ok();
        })
    }

    /// `MDPP_BENCH=check`: type, Save through the action, compare bytes; open the preview, move
    /// to the middle, check scroll sync. Destructive — run it on a scratch copy.
    /// Keystrokes through the real keymap: Enter continues a list, Ctrl+B bolds a selection,
    /// Shift+Enter stays a plain line break. Runs in a fresh untitled tab.
    async fn check_assists(this: &WeakEntity<Self>, cx: &mut AsyncWindowContext) {
        this.update_in(cx, |this, window, cx| {
            this.add_document(OpenedDoc::untitled(None), window, cx)
        })
        .ok();
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let Ok(editor) = this.update(cx, |this, cx| {
            this.active_document().read(cx).editor().clone()
        }) else {
            return;
        };
        // Dispatch without holding the workspace: the handlers update it themselves.
        let press = |keys: &'static str, cx: &mut AsyncWindowContext| {
            cx.update(|window, cx| {
                if let Ok(keystroke) = Keystroke::parse(keys) {
                    window.dispatch_keystroke(keystroke, cx);
                }
            })
            .ok();
        };
        editor
            .update_in(cx, |state, window, cx| state.insert("- one", window, cx))
            .ok();
        press("enter", cx);
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        editor
            .update_in(cx, |state, window, cx| state.insert("two", window, cx))
            .ok();
        press("enter", cx);
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        press("enter", cx); // empty item: ends the list
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        editor
            .update_in(cx, |state, window, cx| state.insert("bold", window, cx))
            .ok();
        editor
            .update_in(cx, |state, _, cx| {
                let end = state.text().len();
                state.set_selected_range(end - 4..end, cx);
            })
            .ok();
        press("ctrl-b", cx);
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let text = editor.read_with(cx, |state, _| state.text().to_string());
        println!(
            "BENCH check assists list_and_bold={} text={text:?}",
            text == "- one\n- two\n\n**bold**"
        );
        // Auto-closing pair, typed through the editor's text input.
        editor
            .update_in(cx, |state, window, cx| {
                let end = state.text().len();
                state.set_selected_range(end..end, cx);
                state.insert(" ", window, cx);
            })
            .ok();
        press("(", cx);
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let tail = editor.read_with(cx, |state, _| {
            let text = state.text().to_string();
            text[text.len().saturating_sub(3)..].to_string()
        });
        println!(
            "BENCH check assists auto_close={} tail={tail:?}",
            tail == " ()"
        );
        // F12 on a link to a heading moves the caret to the heading.
        editor
            .update_in(cx, |state, window, cx| {
                state.replace_all("# Top\n\nsee [top](#top)\n".to_string(), window, cx);
                state.set_selected_range(14..14, cx);
            })
            .ok();
        press("f12", cx);
        cx.background_executor().timer(bench::SETTLE).await;
        let caret = editor.read_with(cx, |state, _| state.cursor());
        println!(
            "BENCH check assists go_to_definition={} caret={caret}",
            caret == 0
        );

        // Each step: set the text and caret, press keys, compare the text.
        let steps: [(&str, usize, &[&str], &str); 5] = [
            ("- a\n- b", 7, &["tab"], "- a\n  - b"),
            ("- a\n  - b", 9, &["shift-tab"], "- a\n- b"),
            ("Title", 5, &["ctrl-shift-]", "ctrl-shift-]"], "## Title"),
            ("a\nb\nc", 5, &["alt-up", "ctrl-d"], "a\nc\nc\nb"),
            ("a\nb\nc", 2, &["ctrl-shift-k"], "a\nc"),
        ];
        let mut failed = Vec::new();
        for (text, caret, keys, expected) in steps {
            editor
                .update_in(cx, |state, window, cx| {
                    state.replace_all(text.to_string(), window, cx);
                    state.set_selected_range(caret..caret, cx);
                })
                .ok();
            for key in keys {
                press(key, cx);
                cx.background_executor()
                    .timer(bench::KEYSTROKE_INTERVAL)
                    .await;
            }
            let result = editor.read_with(cx, |state, _| state.text().to_string());
            if result != expected {
                failed.push(format!("{keys:?} on {text:?} gave {result:?}"));
            }
        }
        println!(
            "BENCH check commands keystrokes_ok={} failed={failed:?}",
            failed.is_empty()
        );

        // Go to line through the bar: Ctrl+G, type 3, Enter.
        editor
            .update_in(cx, |state, window, cx| {
                state.replace_all("one\ntwo\nthree\nfour".to_string(), window, cx);
                state.set_selected_range(0..0, cx);
            })
            .ok();
        for key in ["ctrl-g", "3", "enter"] {
            press(key, cx);
            cx.background_executor()
                .timer(bench::KEYSTROKE_INTERVAL)
                .await;
        }
        let line = editor.read_with(cx, |state, _| state.cursor_position().line);
        let bar_closed = this.update(cx, |this, _| !this.show_goto).unwrap_or(false);
        println!(
            "BENCH check commands go_to_line={} line={line} bar_closed={bar_closed}",
            line == 2
        );

        // Zoom and word wrap.
        let base = this
            .update(cx, |this, _| this.font_size())
            .unwrap_or_default();
        for key in ["ctrl-=", "ctrl-=", "ctrl--"] {
            press(key, cx);
        }
        let zoomed = this
            .update(cx, |this, _| this.font_size())
            .unwrap_or_default();
        press("ctrl-0", cx);
        let reset = this
            .update(cx, |this, _| this.font_size())
            .unwrap_or_default();
        let wrap_before = this
            .update(cx, |this, cx| this.active_document().read(cx).soft_wrap())
            .unwrap_or_default();
        press("alt-z", cx);
        let wrap_after = this
            .update(cx, |this, cx| this.active_document().read(cx).soft_wrap())
            .unwrap_or_default();
        println!(
            "BENCH check commands zoom={} reset={} wrap_toggled={}",
            zoomed == base + 1.0,
            reset == base,
            wrap_before != wrap_after
        );
        press("alt-z", cx); // back as it was

        this.update_in(cx, |this, window, cx| {
            let doc = this.active_document().clone();
            doc.update(cx, |doc, _| doc.discard_backup());
            this.remove_document(&doc, window, cx);
        })
        .ok();
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
    }

    async fn run_check(
        this: &WeakEntity<Self>,
        editor: &Entity<gpui_kit::component::input::EditorState>,
        cx: &mut AsyncWindowContext,
    ) {
        cx.background_executor().timer(bench::SETTLE).await;
        let document = this
            .update(cx, |this, _| this.active_document().clone())
            .ok();
        let Some(document) = document else {
            return;
        };
        // Fenced code is coloured in the background after opening (the file has ```rust fn main() {}).
        let colored = document.read_with(cx, |doc, cx| {
            use gpui_kit::component::input::HighlightStyleResolver as _;
            let colors = doc.code_colors();
            let key = mdpp_markdown::code::block_key("rust", ["fn main() {}"]);
            let line = colors
                .block(key)
                .and_then(|lines| lines.first().cloned())
                .unwrap_or_default();
            let keyword = line
                .iter()
                .any(|(r, name)| *name == "keyword" && *r == (0..2));
            let themed = cx.theme().highlight_theme.style("keyword").is_some();
            format!(
                "block_colored={} fn_is_keyword={keyword} theme_styles_keyword={themed}",
                !line.is_empty()
            )
        });
        println!("BENCH check code_colors {colored}");
        // Every menu item reaches a handler from the editor (a menu entry that does nothing is a bug).
        let menus = editor
            .update_in(cx, |state, window, cx| {
                state.focus(window, cx);
                let dead: Vec<&str> = crate::menus::actions()
                    .filter(|name| {
                        cx.build_action(name, None)
                            .map_or(true, |action| !window.is_action_available(&*action, cx))
                    })
                    .collect();
                format!(
                    "problems={} unhandled={dead:?}",
                    crate::menus::problems(cx).len()
                )
            })
            .unwrap_or_default();
        // The menu bar row stays shown for the rest of the check, so every later frame draws it.
        let shown = this
            .update(cx, |this, cx| {
                this.show_menu_bar = true;
                cx.notify();
                this.show_menu_bar
            })
            .unwrap_or(false);
        println!("BENCH check menus {menus} menu_bar_shown={shown}");
        let path = document.read_with(cx, |doc, _| doc.path().map(PathBuf::from));
        let before = path.as_ref().and_then(|p| std::fs::read(p).ok());
        editor
            .update_in(cx, |state, window, cx| {
                state.set_cursor_position(Position::new(0, 0), window, cx);
                state.insert("x", window, cx);
            })
            .ok();
        // Change events are delivered after the update that caused them.
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let dirty_after_edit = document.read_with(cx, |doc, _| doc.is_dirty());
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(Save), cx)
        })
        .ok();
        cx.background_executor().timer(bench::SETTLE).await;
        match (path, before) {
            (Some(path), Some(before)) => {
                // "x" typed at the start, written back in the file's own encoding, BOM and line endings.
                let original = mdpp_core::text::decode(&before, mdpp_core::text::Eol::Lf)
                    .expect("the check file is text");
                let expected = mdpp_core::text::encode(
                    &format!("x{}", original.text),
                    original.eol,
                    original.bom,
                    original.encoding,
                )
                .unwrap_or_default();
                let on_disk = std::fs::read(&path).unwrap_or_default();
                let clean = document.read_with(cx, |doc, _| !doc.is_dirty());
                println!(
                    "BENCH check save bytes_match={} dirty_after_edit={dirty_after_edit} clean_after_save={clean}",
                    on_disk == expected
                );
            }
            _ => println!("BENCH check save skipped (needs an existing file)"),
        }
        // HTML export (without the dialog): the file must exist and contain the rendered heading.
        let export_path =
            std::env::temp_dir().join(format!("mdpp-check-{}.html", std::process::id()));
        let exporting = document.update_in(cx, |doc, window, cx| {
            doc.export_html_to(export_path.clone(), window, cx)
        });
        if let Ok(exporting) = exporting {
            let ok = exporting.await;
            let html = std::fs::read_to_string(&export_path).unwrap_or_default();
            println!(
                "BENCH check export ok={ok} html_has_heading={}",
                html.contains("<h2>")
            );
            let _ = std::fs::remove_file(&export_path);
        }
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(TogglePreview), cx)
        })
        .ok();
        cx.background_executor().timer(bench::SETTLE).await;
        editor
            .update_in(cx, |state, window, cx| {
                let middle = state.text().len_lines(ropey::LineType::LF) / 2;
                state.set_cursor_position(Position::new(middle as u32, 0), window, cx);
            })
            .ok();
        cx.background_executor().timer(bench::SETTLE).await;
        let report = document
            .read_with(cx, |doc, cx| {
                let state = doc.editor().read(cx);
                let top = state.visible_row_range()?.start;
                let byte = state.text().line_to_byte_idx(top, ropey::LineType::LF);
                let preview = doc.preview().read(cx);
                let (blocks, at_top) = preview.position();
                Some(format!(
                    "blocks={blocks} editor_top_line={top} preview_top_block={at_top} expected_block={:?}",
                    preview.block_index_at(byte)
                ))
            });
        println!(
            "BENCH check preview {}",
            report.as_deref().unwrap_or("no layout")
        );

        // Outline: open it, jump to a heading the way a click does, check the caret.
        cx.update(|window, cx| window.dispatch_action(Box::new(ToggleOutline), cx))
            .ok();
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let target = document.read_with(cx, |doc, _| {
            doc.index().and_then(|index| {
                index
                    .headings
                    .get(10)
                    .map(|h| (index.headings.len(), h.range.start))
            })
        });
        if let Some((count, start)) = target {
            this.update_in(cx, |this, window, cx| this.jump_to(start, window, cx))
                .ok();
            cx.background_executor()
                .timer(bench::KEYSTROKE_INTERVAL)
                .await;
            let caret = editor.read_with(cx, |state, _| state.cursor());
            let words = document.read_with(cx, |doc, _| doc.word_count());
            println!(
                "BENCH check outline headings={count} jump_ok={} words={words}",
                caret == start
            );
        }

        // Find / replace through the bar: count must equal the engine's, replace-all must stick.
        let query = "Section";
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(Find), cx)
        })
        .ok();
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let counts = this
            .update_in(cx, |this, window, cx| {
                let text = editor.read(cx).text().to_string();
                let options = mdpp_core::search::SearchOptions::default();
                let expected = mdpp_core::search::Search::new(query, options)
                    .map(|s| s.find_all(&text, None, usize::MAX).len())
                    .unwrap_or_default();
                this.find_bar.update(cx, |bar, cx| {
                    bar.set_query(query, window, cx);
                    bar.set_replacement("Part", window, cx);
                    let found = bar.match_count();
                    bar.replace_every_match(window, cx);
                    (found, expected)
                })
            })
            .ok();
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let left = editor.read_with(cx, |state, _| {
            state.text().to_string().matches(query).count()
        });
        if let Some((found, expected)) = counts {
            println!(
                "BENCH check find matches={found} engine={expected} left_after_replace_all={left}"
            );
        }

        let Some(path) = document.read_with(cx, |doc, _| doc.path().map(PathBuf::from)) else {
            return;
        };

        // Line endings: convert to the other style (the tab becomes unsaved), save, check bytes.
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(Save), cx)
        })
        .ok();
        cx.background_executor().timer(bench::SETTLE).await;
        let target = match document.read_with(cx, |doc, _| doc.eol()) {
            Eol::CrLf => Eol::Lf,
            _ => Eol::CrLf,
        };
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(ConvertEol { eol: target }), cx)
        })
        .ok();
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let dirty_after_convert = document.read_with(cx, |doc, _| doc.is_dirty());
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(Save), cx)
        })
        .ok();
        cx.background_executor().timer(bench::SETTLE).await;
        let bytes = std::fs::read(&path).unwrap_or_default();
        let lf = bytes.iter().filter(|b| **b == b'\n').count();
        let crlf = bytes.windows(2).filter(|w| w == b"\r\n").count();
        let converted = lf > 0
            && if target == Eol::CrLf {
                lf == crlf
            } else {
                crlf == 0
            };
        let clean = document.read_with(cx, |doc, _| !doc.is_dirty());
        println!(
            "BENCH check eol to={target} dirty_after_convert={dirty_after_convert} converted={converted} clean_after_save={clean}"
        );

        // Close the tab and bring it back with Ctrl+Shift+T, at the same line.
        let line_before = editor.read_with(cx, |state, _| state.cursor_position().line);
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(CloseTab), cx)
        })
        .ok();
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        let closed = this
            .update(cx, |this, cx| {
                !this
                    .documents
                    .iter()
                    .any(|d| d.read(cx).path().is_some_and(|p| same_file(p, &path)))
            })
            .unwrap_or(false);
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(ReopenClosedTab), cx)
        })
        .ok();
        cx.background_executor().timer(bench::SETTLE).await;
        let reopened = this
            .update(cx, |this, cx| {
                let doc = this.active_document().clone();
                let (same, line) = {
                    let doc = doc.read(cx);
                    let same = doc.path().is_some_and(|p| same_file(p, &path));
                    (same, doc.editor().read(cx).cursor_position().line)
                };
                let recent = this.recent.first().is_some_and(|p| same_file(p, &path));
                (doc, same, line, recent)
            })
            .ok();
        let Some((document, same, line_after, recent)) = reopened else {
            return;
        };
        println!(
            "BENCH check reopen closed={closed} reopened={same} same_line={} recent_first={recent}",
            line_after == line_before
        );
        let editor = &document.read_with(cx, |doc, _| doc.editor().clone());

        // External changes: clean tab reloads silently; dirty tab gets a conflict; deletion too.
        let settle = bench::SETTLE;
        this.update_in(cx, |_, window, cx| {
            window.dispatch_action(Box::new(Save), cx)
        })
        .ok();
        cx.background_executor().timer(settle).await;
        std::fs::write(&path, "# Changed elsewhere\n").ok();
        cx.background_executor().timer(settle).await;
        let (text, dirty) = document.read_with(cx, |doc, cx| {
            (doc.editor().read(cx).text().to_string(), doc.is_dirty())
        });
        println!(
            "BENCH check external clean_reloaded={} dirty={dirty}",
            text == "# Changed elsewhere\n"
        );
        editor
            .update_in(cx, |state, window, cx| state.insert("y", window, cx))
            .ok();
        cx.background_executor()
            .timer(bench::KEYSTROKE_INTERVAL)
            .await;
        std::fs::write(&path, "# Changed again\n").ok();
        cx.background_executor().timer(settle).await;
        let conflict = document.read_with(cx, |doc, _| doc.conflict());
        std::fs::remove_file(&path).ok();
        cx.background_executor().timer(settle).await;
        let (deleted, dirty) = document.read_with(cx, |doc, _| (doc.conflict(), doc.is_dirty()));
        println!(
            "BENCH check external dirty_conflict={conflict:?} after_delete={deleted:?} dirty={dirty}"
        );

        // The settings command creates the user's file from the template (only when it does not
        // exist yet; the check removes what it created) and opens it in a tab.
        let settings_file = this
            .update(cx, |this, _| this.paths.as_ref().map(Paths::settings_file))
            .ok()
            .flatten()
            .filter(|file| !file.exists());
        if let Some(file) = settings_file {
            this.update_in(cx, |_, window, cx| {
                window.dispatch_action(Box::new(OpenSettingsFile), cx)
            })
            .ok();
            cx.background_executor().timer(bench::SETTLE).await;
            let opened = this
                .update(cx, |this, cx| {
                    let doc = this.active_document().read(cx);
                    doc.path().is_some_and(|p| same_file(p, &file))
                })
                .unwrap_or(false);
            let content = std::fs::read_to_string(&file).unwrap_or_default();
            println!(
                "BENCH check settings_file created={} opened={opened}",
                content == mdpp_core::settings::user_template()
            );
            std::fs::remove_file(&file).ok();

            // The settings window (Ctrl+,) writes a change into settings.toml and the editor
            // applies it at once; setting the default back removes the line again.
            this.update_in(cx, |_, window, cx| {
                window.dispatch_action(Box::new(OpenSettings), cx)
            })
            .ok();
            cx.background_executor().timer(bench::SETTLE).await;
            let view = this
                .update(cx, |this, _| this.settings_window.clone())
                .ok()
                .flatten();
            let opened = view.is_some();
            let (mut applied, mut line_removed, mut restored) = (false, false, false);
            if let Some((handle, view)) = view {
                let defaults: toml::Table =
                    toml::from_str(mdpp_core::settings::DEFAULT_SETTINGS).unwrap_or_default();
                let default = mdpp_core::settings_ui::lookup(&defaults, "font_size").cloned();
                let set = cx.update(|_, cx| view.read(cx).setter()).ok();
                if let Some(set) = &set {
                    cx.update(|_, cx| set("font_size", toml::Value::Integer(17), cx))
                        .ok();
                }
                cx.background_executor().timer(bench::SETTLE).await;
                applied = this
                    .update(cx, |this, _| this.settings.font_size == 17.0)
                    .unwrap_or(false);
                if let (Some(set), Some(default)) = (&set, default) {
                    cx.update(|_, cx| set("font_size", default, cx)).ok();
                }
                cx.background_executor().timer(bench::SETTLE).await;
                line_removed = !std::fs::read_to_string(&file)
                    .unwrap_or_default()
                    .contains("font_size");
                restored = this
                    .update(cx, |this, _| {
                        this.settings.font_size == Settings::default().font_size
                    })
                    .unwrap_or(false);
                handle
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            }
            println!(
                "BENCH check settings_window opened={opened} applied={applied} line_removed={line_removed} restored={restored}"
            );
            std::fs::remove_file(&file).ok();
        }

        // Hot exit off: closing the window with unsaved changes must ask, not lose them.
        let (with_hot_exit, without) = this
            .update_in(cx, |this, window, cx| {
                let dirty = this.documents.iter().any(|d| d.read(cx).is_dirty());
                let with_hot_exit = this.should_close(window, cx);
                let mut settings = (*this.settings).clone();
                settings.hot_exit = false;
                this.settings = Rc::new(settings);
                (with_hot_exit, dirty && !this.should_close(window, cx))
            })
            .unwrap_or_default();
        println!(
            "BENCH check hot_exit_off closes_with_hot_exit={with_hot_exit} asks_without={without}"
        );
    }
}
