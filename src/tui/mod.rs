pub mod app;
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
    let settings = Settings {
        inbox_similarity: ctx.config.link.inbox_similarity,
        locator: ctx.config.ui.locator,
        theme: Theme::default(),
        keys: Keymap::default(),
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
                    Effect::Edit { .. } | Effect::Open(_) => {}
                }
            }
        }
    }

    Ok(())
}
