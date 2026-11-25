pub mod app;
pub mod draft;
pub mod input;
pub mod keys;
pub mod store;
pub mod text;
pub mod theme;
pub mod view;

use crate::error::Error;
use crate::Ctx;
use app::{App, Effect, Settings};
use keys::Keymap;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::io::Write;
use std::process::{Command, Stdio};
use store::CtxStore;
use theme::Theme;

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// TUI を起動し、終了するまで画面を描画してキー入力を処理する。
pub fn run(ctx: Ctx) -> Result<(), Error> {
    let theme = Theme::from_config(&ctx.config.theme)?;
    let keys = Keymap::from_config(&ctx.config.keys)?;
    let settings = Settings {
        inbox_similarity: ctx.config.link.inbox_similarity,
        locator: ctx.config.ui.locator,
        theme,
        keys,
    };
    let mut app = App::new(Box::new(CtxStore(ctx)), settings);
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;

    loop {
        terminal
            .draw(|f| view::draw(f, &app))
            .map_err(|e| Error::Permanent(format!("画面の描画に失敗しました: {e}")))?;

        let ev = event::read().map_err(|e| Error::Permanent(format!("キーの読み込みに失敗しました: {e}")))?;
        if let Event::Key(key) = ev {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if let Some(effect) = app.on_key(key) {
                match effect {
                    Effect::Quit => break,
                    Effect::Edit { draft } => {
                        ratatui::restore();
                        let res = edit_in_editor(&draft);
                        terminal = ratatui::init();
                        app.finish_promote(res);
                    }
                    Effect::Open(url) => {
                        if let Err(msg) = open_url(&url) {
                            app.set_message(msg);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn edit_in_editor(draft: &str) -> Result<String, String> {
    let mut tmp = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()
        .map_err(|e| format!("一時ファイルを作れませんでした: {e}"))?;
    tmp.write_all(draft.as_bytes())
        .and_then(|_| tmp.flush())
        .map_err(|e| format!("一時ファイルに書けませんでした: {e}"))?;

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let cmd = parts.next().unwrap_or("vi");

    let status = Command::new(cmd)
        .args(parts)
        .arg(tmp.path())
        .status()
        .map_err(|e| format!("{cmd} を起動できませんでした: {e}"))?;

    if !status.success() {
        return Err(format!("{cmd} が異常終了しました ({status})"));
    }

    std::fs::read_to_string(tmp.path()).map_err(|e| format!("一時ファイルを読めませんでした: {e}"))
}

fn open_url(url: &str) -> Result<(), String> {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(cmd)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{cmd} を起動できませんでした: {e}"))
}
