use crate::error::Error;
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
    if matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab) {
        // 大文字や記号、BackTab はキー自体が Shift を含むため、SHIFT フラグを外して揃える
        mods.remove(KeyModifiers::SHIFT);
    }
    (key.code, mods)
}

impl Keymap {
    /// 設定ファイルの `[keys]` からキー割り当てを作る。書かれた動作は既定を置き換え、衝突を検査する。
    pub fn from_config(val: &toml::Value) -> Result<Keymap, Error> {
        let Some(map) = val.as_table() else {
            return Err(Error::Invalid("[keys] must be a table".to_string()));
        };
        let mut keymap = Keymap::default();
        for (section_name, section_val) in map {
            let screen = match section_name.as_str() {
                "common" => None,
                "inbox" => Some(Screen::Inbox),
                "review" => Some(Screen::Review),
                "failed" => Some(Screen::Failed),
                _ => {
                    return Err(Error::Invalid(format!(
                        "[keys] has no section '{section_name}'"
                    )))
                }
            };
            let Some(actions) = section_val.as_table() else {
                return Err(Error::Invalid(format!(
                    "[keys.{section_name}] must be a table"
                )));
            };
            for (action_name, keys_val) in actions {
                let action = parse_action(screen, action_name).ok_or_else(|| {
                    Error::Invalid(format!(
                        "[keys.{section_name}] has no action '{action_name}'"
                    ))
                })?;
                let Some(keys) = keys_val.as_array() else {
                    return Err(Error::Invalid(format!(
                        "[keys.{section_name}] {action_name} must be an array"
                    )));
                };
                keymap
                    .bindings
                    .retain(|b| !(b.screen == screen && b.action == action));
                for item in keys {
                    let Some(raw) = item.as_str() else {
                        return Err(Error::Invalid(format!(
                            "[keys.{section_name}] {action_name}: keys must be strings"
                        )));
                    };
                    let seq = parse_key_seq(raw).map_err(|msg| {
                        Error::Invalid(format!(
                            "[keys.{section_name}] {action_name}: key '{raw}' {msg}"
                        ))
                    })?;
                    keymap.bindings.push(Binding { screen, seq, action });
                }
            }
        }
        check_conflicts(&keymap.bindings)?;
        Ok(keymap)
    }

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

fn format_seq(seq: KeySeq) -> String {
    match seq {
        KeySeq::Single(code, mods) => format_key(code, mods),
        KeySeq::Pair((c1, m1), (c2, m2)) => format!("{} {}", format_key(c1, m1), format_key(c2, m2)),
    }
}

fn parse_action(screen: Option<Screen>, name: &str) -> Option<Action> {
    match (screen, name) {
        (None, "down") => Some(Action::Down),
        (None, "up") => Some(Action::Up),
        (None, "top") => Some(Action::Top),
        (None, "bottom") => Some(Action::Bottom),
        (None, "scroll_down") => Some(Action::ScrollDown),
        (None, "scroll_up") => Some(Action::ScrollUp),
        (None, "help") => Some(Action::Help),
        (None, "quit") => Some(Action::Quit),

        (Some(Screen::Inbox), "write") => Some(Action::Write),
        (Some(Screen::Inbox), "toggle") => Some(Action::Toggle),
        (Some(Screen::Inbox), "clear_selection") => Some(Action::ClearSelection),
        (Some(Screen::Inbox), "delete") => Some(Action::Delete),
        (Some(Screen::Inbox), "to_review") => Some(Action::ToReview),
        (Some(Screen::Inbox), "to_failed") => Some(Action::ToFailed),
        (Some(Screen::Inbox), "open_source") => Some(Action::OpenSource),

        (Some(Screen::Review), "adopt") => Some(Action::Adopt),
        (Some(Screen::Review), "adopt_with_reason") => Some(Action::AdoptWithReason),
        (Some(Screen::Review), "reject") => Some(Action::Reject),
        (Some(Screen::Review), "change") => Some(Action::Change),
        (Some(Screen::Review), "open_source") => Some(Action::OpenSource),
        (Some(Screen::Review), "next") => Some(Action::Next),
        (Some(Screen::Review), "prev") => Some(Action::Prev),
        (Some(Screen::Review), "undo") => Some(Action::Undo),
        (Some(Screen::Review), "redo") => Some(Action::Redo),
        (Some(Screen::Review), "merge") => Some(Action::Merge),
        (Some(Screen::Review), "back") => Some(Action::Back),

        (Some(Screen::Failed), "retry") => Some(Action::Retry),
        (Some(Screen::Failed), "discard") => Some(Action::Discard),
        (Some(Screen::Failed), "retry_all") => Some(Action::RetryAll),
        (Some(Screen::Failed), "back") => Some(Action::Back),

        _ => None,
    }
}

fn parse_key_seq(raw: &str) -> Result<KeySeq, &'static str> {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    match parts.as_slice() {
        [a] => {
            let (c, m) = parse_single_key(a)?;
            Ok(KeySeq::Single(c, m))
        }
        [a, b] => Ok(KeySeq::Pair(parse_single_key(a)?, parse_single_key(b)?)),
        [] => Err("is empty"),
        _ => Err("must be at most 2 keys"),
    }
}

fn parse_single_key(s: &str) -> Result<(KeyCode, KeyModifiers), &'static str> {
    match s {
        "space" => return Ok((KeyCode::Char(' '), KeyModifiers::NONE)),
        "enter" => return Ok((KeyCode::Enter, KeyModifiers::NONE)),
        "esc" => return Ok((KeyCode::Esc, KeyModifiers::NONE)),
        "tab" => return Ok((KeyCode::Tab, KeyModifiers::NONE)),
        "backtab" => return Ok((KeyCode::BackTab, KeyModifiers::NONE)),
        "up" => return Ok((KeyCode::Up, KeyModifiers::NONE)),
        "down" => return Ok((KeyCode::Down, KeyModifiers::NONE)),
        "left" => return Ok((KeyCode::Left, KeyModifiers::NONE)),
        "right" => return Ok((KeyCode::Right, KeyModifiers::NONE)),
        "backspace" => return Ok((KeyCode::Backspace, KeyModifiers::NONE)),
        _ => {}
    }
    if let Some(rest) = s.strip_prefix("ctrl-") {
        let mut chars = rest.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Ok((KeyCode::Char(c), KeyModifiers::CONTROL));
        }
        return Err("cannot be parsed");
    }
    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok((KeyCode::Char(c), KeyModifiers::NONE));
    }
    Err("cannot be parsed")
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::Down => "down",
        Action::Up => "up",
        Action::Top => "top",
        Action::Bottom => "bottom",
        Action::ScrollDown => "scroll_down",
        Action::ScrollUp => "scroll_up",
        Action::Help => "help",
        Action::Quit => "quit",
        Action::Write => "write",
        Action::Toggle => "toggle",
        Action::ClearSelection => "clear_selection",
        Action::Delete => "delete",
        Action::ToReview => "to_review",
        Action::ToFailed => "to_failed",
        Action::OpenSource => "open_source",
        Action::Adopt => "adopt",
        Action::AdoptWithReason => "adopt_with_reason",
        Action::Reject => "reject",
        Action::Change => "change",
        Action::Next => "next",
        Action::Prev => "prev",
        Action::Undo => "undo",
        Action::Redo => "redo",
        Action::Merge => "merge",
        Action::Back => "back",
        Action::Retry => "retry",
        Action::Discard => "discard",
        Action::RetryAll => "retry_all",
    }
}

fn check_conflicts(bindings: &[Binding]) -> Result<(), Error> {
    for screen in [Screen::Inbox, Screen::Review, Screen::Failed] {
        let active: Vec<&Binding> = bindings
            .iter()
            .filter(|b| b.screen.is_none() || b.screen == Some(screen))
            .collect();
        for (i, b1) in active.iter().enumerate() {
            for b2 in &active[i + 1..] {
                if b1.seq == b2.seq {
                    return Err(Error::Invalid(format!(
                        "{} screen: key '{}' is bound to both {} and {}",
                        screen_name(screen),
                        format_seq(b1.seq),
                        action_name(b1.action),
                        action_name(b2.action)
                    )));
                }
                let prefix_conflict = match (b1.seq, b2.seq) {
                    (KeySeq::Single(c, m), KeySeq::Pair(first, _)) => (c, m) == first,
                    (KeySeq::Pair(first, _), KeySeq::Single(c, m)) => (c, m) == first,
                    _ => false,
                };
                if prefix_conflict {
                    return Err(Error::Invalid(format!(
                        "{} screen: key '{}' ({}) is a prefix of '{}' ({})",
                        screen_name(screen),
                        format_seq(b1.seq),
                        action_name(b1.action),
                        format_seq(b2.seq),
                        action_name(b2.action)
                    )));
                }
            }
        }
    }
    Ok(())
}

fn screen_name(screen: Screen) -> &'static str {
    match screen {
        Screen::Inbox => "inbox",
        Screen::Review => "review",
        Screen::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toml_val(s: &str) -> toml::Value {
        toml::from_str(s).unwrap()
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn keys_override_replaces_default() {
        let km = Keymap::from_config(&toml_val("[inbox]\ndelete = [\"D\"]")).unwrap();
        assert_eq!(km.resolve(Screen::Inbox, None, key('D')), Resolved::Action(Action::Delete));
        assert_eq!(km.resolve(Screen::Inbox, None, key('d')), Resolved::Nothing);
    }

    #[test]
    fn keys_two_key_sequence() {
        let km = Keymap::from_config(&toml_val("[common]\ntop = [\"t t\"]")).unwrap();
        assert_eq!(km.resolve(Screen::Inbox, None, key('t')), Resolved::Pending);
        assert_eq!(
            km.resolve(Screen::Inbox, Some(key('t')), key('t')),
            Resolved::Action(Action::Top)
        );
        assert_eq!(km.label(Screen::Inbox, Action::Top).as_deref(), Some("tt"));
    }

    #[test]
    fn keys_reject_conflict_with_snake_case_names() {
        let err = Keymap::from_config(&toml_val("[inbox]\ndelete = [\"j\"]")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("down") && msg.contains("delete"), "msg: {msg}");
        assert!(!msg.contains("Down") && !msg.contains("Delete"), "msg: {msg}");
    }

    #[test]
    fn keys_reject_prefix_conflict() {
        let err = Keymap::from_config(&toml_val("[inbox]\ndelete = [\"g\"]")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("top") && msg.contains("delete"), "msg: {msg}");
    }

    #[test]
    fn keys_reject_unknown_action_and_key() {
        assert!(Keymap::from_config(&toml_val("[inbox]\nnosuch = [\"x\"]")).is_err());
        assert!(Keymap::from_config(&toml_val("[inbox]\ndelete = [\"ctrl-\"]")).is_err());
    }

    #[test]
    fn keys_reject_invalid_shape() {
        let err = Keymap::from_config(&toml_val("[inbox]\ndown = \"j\"")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("inbox") && msg.contains("down"), "msg: {msg}");
    }

    #[test]
    fn backtab_matches_with_shift_modifier() {
        let km = Keymap::from_config(&toml_val("[review]\nprev = [\"backtab\"]")).unwrap();
        let ev = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(km.resolve(Screen::Review, None, ev), Resolved::Action(Action::Prev));
    }

    #[test]
    fn ctrl_key_parses() {
        let km = Keymap::from_config(&toml_val("[review]\nredo = [\"ctrl-y\"]")).unwrap();
        assert_eq!(km.resolve(Screen::Review, None, ctrl('y')), Resolved::Action(Action::Redo));
    }
}
