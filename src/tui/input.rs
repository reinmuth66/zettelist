use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_width::UnicodeWidthChar;

/// 1 行のテキスト入力欄。
pub struct LineInput {
    text: String,
    cursor: usize,
}

impl LineInput {
    /// 初期文字列から入力欄を作る。カーソルは末尾に置く。
    pub fn new(text: &str) -> LineInput {
        LineInput {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    /// 現在の文字列を返す。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// カーソルが末尾にあるか返す。
    pub fn is_at_end(&self) -> bool {
        self.cursor == self.text.chars().count()
    }

    /// キー入力を処理する。処理したら true、それ以外は false。
    pub fn handle(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Enter | KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab, _) => false,
            (KeyCode::Left, KeyModifiers::NONE) => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                true
            }
            (KeyCode::Right, KeyModifiers::NONE) => {
                if self.cursor < self.text.chars().count() {
                    self.cursor += 1;
                }
                true
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.cursor = 0;
                true
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.cursor = self.text.chars().count();
                true
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                let byte = self.byte_index(self.cursor);
                self.text.drain(..byte);
                self.cursor = 0;
                true
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                self.delete_previous_word();
                true
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                if self.cursor > 0 {
                    let end = self.byte_index(self.cursor);
                    let start = self.byte_index(self.cursor - 1);
                    self.text.drain(start..end);
                    self.cursor -= 1;
                }
                true
            }
            (KeyCode::Char(c), mods)
                if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) =>
            {
                let idx = self.byte_index(self.cursor);
                self.text.insert(idx, c);
                self.cursor += 1;
                true
            }
            _ => false,
        }
    }

    /// 幅 `width` に収まる見える部分と、その中のカーソルの桁を返す。
    pub fn view(&self, width: u16) -> (String, u16) {
        let max_w = width as usize;
        if max_w == 0 {
            return (String::new(), 0);
        }

        let chars: Vec<(char, usize)> = self
            .text
            .chars()
            .map(|c| (c, c.width().unwrap_or(0)))
            .collect();

        let cursor = self.cursor.min(chars.len());

        // カーソルが幅の中に収まるよう、左端の文字位置を決める。
        let mut start = 0;
        let mut prefix_w: usize = chars[..cursor].iter().map(|(_, w)| *w).sum();
        while prefix_w >= max_w && start < cursor {
            prefix_w -= chars[start].1;
            start += 1;
        }

        let mut visible = String::new();
        let mut total_w = 0;
        for &(c, w) in &chars[start..] {
            if total_w + w > max_w {
                break;
            }
            visible.push(c);
            total_w += w;
        }

        (visible, prefix_w as u16)
    }

    fn byte_index(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    fn delete_previous_word(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut pos = self.cursor;
        while pos > 0 && chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        while pos > 0 && !chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        let start = self.byte_index(pos);
        let end = self.byte_index(self.cursor);
        self.text.drain(start..end);
        self.cursor = pos;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn typing_inserts_at_cursor() {
        let mut input = LineInput::new("ab");
        assert!(input.handle(key(KeyCode::Left)));
        assert!(input.handle(key(KeyCode::Char('x'))));
        assert_eq!(input.text(), "axb");
    }

    #[test]
    fn ctrl_w_deletes_previous_word() {
        let mut input = LineInput::new("foo bar");
        assert!(input.handle(ctrl('w')));
        assert_eq!(input.text(), "foo ");
    }

    #[test]
    fn ctrl_u_deletes_to_line_start() {
        let mut input = LineInput::new("foo bar");
        for _ in 0..3 {
            assert!(input.handle(key(KeyCode::Left)));
        }
        assert!(input.handle(ctrl('u')));
        assert_eq!(input.text(), "bar");
    }

    #[test]
    fn enter_esc_tab_are_not_consumed() {
        let mut input = LineInput::new("abc");
        for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Tab, KeyCode::BackTab] {
            assert!(!input.handle(key(code)));
        }
        assert_eq!(input.text(), "abc");
    }

    #[test]
    fn wide_chars_move_cursor_two_columns() {
        let input = LineInput::new("あい");
        assert_eq!(input.view(20), ("あい".to_string(), 4));
    }

    #[test]
    fn view_scrolls_to_keep_cursor_visible() {
        use unicode_width::UnicodeWidthStr;

        let input = LineInput::new("あいうえおかきくけこ");
        let (visible, col) = input.view(9);
        assert!(visible.width() <= 9, "visible={visible:?}");
        assert!(visible.ends_with('こ'), "visible={visible:?}");
        assert!(col <= 8, "col={col}");
    }

    #[test]
    fn ctrl_a_ctrl_e_right_and_backspace_work() {
        let mut input = LineInput::new("abc");
        assert!(input.handle(ctrl('a')));
        assert!(input.handle(key(KeyCode::Right)));
        assert!(input.handle(key(KeyCode::Backspace)));
        assert_eq!(input.text(), "bc");
        assert!(input.handle(ctrl('e')));
        assert!(input.handle(key(KeyCode::Char('d'))));
        assert_eq!(input.text(), "bcd");
    }
}
