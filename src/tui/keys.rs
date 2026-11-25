use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Inbox,
    Review,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Down,
    Up,
    Top,
    Bottom,
    ScrollDown,
    ScrollUp,
    Help,
    Quit,
    Write,
    Toggle,
    ClearSelection,
    Delete,
    ToReview,
    ToFailed,
    OpenSource,
    Adopt,
    AdoptWithReason,
    Reject,
    Change,
    Next,
    Prev,
    Undo,
    Redo,
    Merge,
    Back,
    Retry,
    Discard,
    RetryAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    Action(Action),
    Pending,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeySeq {
    Single(KeyCode, KeyModifiers),
    Pair((KeyCode, KeyModifiers), (KeyCode, KeyModifiers)),
}

#[derive(Debug, Clone)]
struct Binding {
    screen: Option<Screen>,
    seq: KeySeq,
    action: Action,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Default for Keymap {
    fn default() -> Self {
        let mut bindings = Vec::new();
        let common = |bindings: &mut Vec<Binding>, seq: KeySeq, action: Action| {
            bindings.push(Binding { screen: None, seq, action });
        };
        let on = |bindings: &mut Vec<Binding>, screen: Screen, seq: KeySeq, action: Action| {
            bindings.push(Binding { screen: Some(screen), seq, action });
        };
        let ch = |c: char| KeySeq::Single(KeyCode::Char(c), KeyModifiers::NONE);
        let ctrl = |c: char| KeySeq::Single(KeyCode::Char(c), KeyModifiers::CONTROL);
        let key = |k: KeyCode| KeySeq::Single(k, KeyModifiers::NONE);

        // どの画面でも共通
        common(&mut bindings, ch('j'), Action::Down);
        common(&mut bindings, key(KeyCode::Down), Action::Down);
        common(&mut bindings, ch('k'), Action::Up);
        common(&mut bindings, key(KeyCode::Up), Action::Up);
        common(
            &mut bindings,
            KeySeq::Pair((KeyCode::Char('g'), KeyModifiers::NONE), (KeyCode::Char('g'), KeyModifiers::NONE)),
            Action::Top,
        );
        common(&mut bindings, ch('G'), Action::Bottom);
        common(&mut bindings, ctrl('d'), Action::ScrollDown);
        common(&mut bindings, ctrl('u'), Action::ScrollUp);
        common(&mut bindings, ch('?'), Action::Help);
        common(&mut bindings, ch('q'), Action::Quit);

        // inbox
        on(&mut bindings, Screen::Inbox, ch('i'), Action::Write);
        on(&mut bindings, Screen::Inbox, ch(' '), Action::Toggle);
        on(&mut bindings, Screen::Inbox, key(KeyCode::Esc), Action::ClearSelection);
        on(&mut bindings, Screen::Inbox, ch('d'), Action::Delete);
        on(&mut bindings, Screen::Inbox, ch('r'), Action::ToReview);
        on(&mut bindings, Screen::Inbox, ch('f'), Action::ToFailed);
        on(&mut bindings, Screen::Inbox, ch('o'), Action::OpenSource);

        // review
        on(&mut bindings, Screen::Review, ch('a'), Action::Adopt);
        on(&mut bindings, Screen::Review, ch('A'), Action::AdoptWithReason);
        on(&mut bindings, Screen::Review, ch('d'), Action::Reject);
        on(&mut bindings, Screen::Review, ch('c'), Action::Change);
        on(&mut bindings, Screen::Review, ch('o'), Action::OpenSource);
        on(&mut bindings, Screen::Review, ch('n'), Action::Next);
        on(&mut bindings, Screen::Review, ch('N'), Action::Prev);
        on(&mut bindings, Screen::Review, ch('u'), Action::Undo);
        on(&mut bindings, Screen::Review, ctrl('r'), Action::Redo);
        on(&mut bindings, Screen::Review, ch('m'), Action::Merge);
        on(&mut bindings, Screen::Review, key(KeyCode::Esc), Action::Back);

        // failed
        on(&mut bindings, Screen::Failed, key(KeyCode::Enter), Action::Retry);
        on(&mut bindings, Screen::Failed, ch('x'), Action::Discard);
        on(&mut bindings, Screen::Failed, ch('R'), Action::RetryAll);
        on(&mut bindings, Screen::Failed, key(KeyCode::Esc), Action::Back);

        Self { bindings }
    }
}

fn normalize_key(key: KeyEvent) -> (KeyCode, KeyModifiers) {
    let mut mods = key.modifiers;
    if matches!(key.code, KeyCode::Char(_)) {
        // 大文字や記号は文字自体が Shift を含むため、SHIFT フラグを外して揃える
        mods.remove(KeyModifiers::SHIFT);
    }
    (key.code, mods)
}

impl Keymap {
    pub fn resolve(&self, screen: Screen, pending: Option<KeyEvent>, key: KeyEvent) -> Resolved {
        if key.kind == KeyEventKind::Release {
            return Resolved::Nothing;
        }
        let cur = normalize_key(key);

        if let Some(prev_ev) = pending {
            let prev = normalize_key(prev_ev);
            if let Some(b) = self.bindings.iter().find(|b| {
                (b.screen.is_none() || b.screen == Some(screen))
                    && matches!(b.seq, KeySeq::Pair(a, c) if a == prev && c == cur)
            }) {
                return Resolved::Action(b.action);
            }
        }

        if self.bindings.iter().any(|b| {
            (b.screen.is_none() || b.screen == Some(screen))
                && matches!(b.seq, KeySeq::Pair(a, _) if a == cur)
        }) {
            return Resolved::Pending;
        }

        if let Some(b) = self.bindings.iter().find(|b| {
            (b.screen.is_none() || b.screen == Some(screen))
                && matches!(b.seq, KeySeq::Single(c, m) if (c, m) == cur)
        }) {
            return Resolved::Action(b.action);
        }

        Resolved::Nothing
    }

    pub fn label(&self, screen: Screen, action: Action) -> Option<String> {
        let b = self
            .bindings
            .iter()
            .find(|b| b.screen == Some(screen) && b.action == action)
            .or_else(|| self.bindings.iter().find(|b| b.screen.is_none() && b.action == action))?;
        Some(match b.seq {
            KeySeq::Single(code, mods) => format_key(code, mods),
            KeySeq::Pair((c1, m1), (c2, m2)) => format!("{}{}", format_key(c1, m1), format_key(c2, m2)),
        })
    }
}

fn format_key(code: KeyCode, mods: KeyModifiers) -> String {
    let base = match code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Up => "↑".to_string(),
        KeyCode::Down => "↓".to_string(),
        KeyCode::Left => "←".to_string(),
        KeyCode::Right => "→".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "Shift-Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        _ => format!("{code:?}"),
    };
    if mods.contains(KeyModifiers::CONTROL) {
        format!("Ctrl-{base}")
    } else {
        base
    }
}
