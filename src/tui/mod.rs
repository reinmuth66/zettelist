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
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use ratatui::crossterm::execute;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use store::CtxStore;
use theme::Theme;

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
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
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let _guard = TerminalGuard;

    loop {
        terminal
            .draw(|f| view::draw(f, &app))
            .map_err(|e| Error::Permanent(format!("could not draw the screen: {e}")))?;

        let ev = event::read().map_err(|e| Error::Permanent(format!("could not read a key: {e}")))?;
        match ev {
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if let Some(effect) = app.on_key(key) {
                    match effect {
                        Effect::Quit => break,
                        Effect::Edit { draft } => {
                            let _ = execute!(std::io::stdout(), DisableBracketedPaste);
                            ratatui::restore();
                            let res = edit_in_editor(&draft);
                            terminal = ratatui::init();
                            let _ = execute!(std::io::stdout(), EnableBracketedPaste);
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
            Event::Paste(text) => app.on_paste(&text),
            _ => {}
        }
    }

    Ok(())
}

fn edit_in_editor(draft: &str) -> Result<String, String> {
    let mut tmp = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()
        .map_err(|e| format!("could not create a temporary file: {e}"))?;
    tmp.write_all(draft.as_bytes())
        .and_then(|_| tmp.flush())
        .map_err(|e| format!("could not write the temporary file: {e}"))?;

    let editor = std::env::var("EDITOR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "vi".to_string());

    run_editor(&editor, tmp.path())?;

    std::fs::read_to_string(tmp.path()).map_err(|e| format!("could not read the temporary file: {e}"))
}

fn run_editor(editor: &str, path: &Path) -> Result<(), String> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(path)
        .status()
        .map_err(|e| format!("could not start {editor}: {e}"))?;

    if !status.success() {
        return Err(format!("{editor} exited abnormally ({status})"));
    }
    Ok(())
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
        .map_err(|e| format!("could not start {cmd}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn run_editor_handles_quoted_path_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("my editor");
        std::fs::create_dir_all(&sub).unwrap();
        let script = sub.join("ed.sh");
        std::fs::write(&script, "#!/bin/sh\necho edited > \"$1\"\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let target = dir.path().join("note.md");
        std::fs::write(&target, "before").unwrap();
        let editor = format!("'{}'", script.display());
        run_editor(&editor, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "edited\n");
    }
}
