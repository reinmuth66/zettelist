use crate::notes::{InboxItem, Near, NoteKind};
use crate::tui::keys::{Action, Keymap, Resolved, Screen};
use crate::tui::store::Store;
use crate::tui::theme::Theme;
use chrono::{DateTime, Utc};
use ratatui::crossterm::event::{KeyEvent, KeyEventKind};
use std::collections::{HashMap, HashSet};

pub struct Settings {
    pub inbox_similarity: f32,
    pub locator: bool,
    pub theme: Theme,
    pub keys: Keymap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Quit,
    Edit { draft: String },
    Open(String),
}

pub struct App {
    store: Box<dyn Store>,
    settings: Settings,
    screen: Screen,
    inbox: Vec<InboxItem>,
    embeddings: HashMap<(NoteKind, String), Vec<f32>>,
    similar_cache: HashMap<(NoteKind, String), Vec<Near>>,
    selected: HashSet<(NoteKind, String)>,
    cursor: usize,
    detail_scroll: u16,
    pending_key: Option<KeyEvent>,
    message: Option<String>,
    review_count: usize,
    failed_count: usize,
    show_help: bool,
}

impl App {
    pub fn new(store: Box<dyn Store>, settings: Settings) -> Self {
        let mut app = Self {
            store,
            settings,
            screen: Screen::Inbox,
            inbox: Vec::new(),
            embeddings: HashMap::new(),
            similar_cache: HashMap::new(),
            selected: HashSet::new(),
            cursor: 0,
            detail_scroll: 0,
            pending_key: None,
            message: None,
            review_count: 0,
            failed_count: 0,
            show_help: false,
        };
        app.reload_inbox();
        app
    }

    fn record_error(&mut self, msg: String) {
        if self.message.is_none() {
            self.message = Some(msg);
        }
    }

    pub fn reload_inbox(&mut self) {
        self.similar_cache.clear();
        self.embeddings.clear();
        self.detail_scroll = 0;

        match self.store.inbox() {
            Ok(mut items) => {
                items.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.slug.cmp(&b.slug)));
                self.inbox = items;
                if self.inbox.is_empty() {
                    self.cursor = 0;
                    self.selected.clear();
                } else {
                    self.cursor = self.cursor.min(self.inbox.len() - 1);
                    self.selected
                        .retain(|k| self.inbox.iter().any(|i| i.kind == k.0 && i.slug == k.1));
                }
            }
            Err(e) => {
                self.inbox.clear();
                self.cursor = 0;
                self.selected.clear();
                self.record_error(e.to_string());
            }
        }

        match self.store.inbox_embeddings() {
            Ok(emb) => self.embeddings = emb,
            Err(e) => self.record_error(e.to_string()),
        }

        match self.store.proposals() {
            Ok((props, _state)) => self.review_count = props.len(),
            Err(e) => self.record_error(e.to_string()),
        }

        match self.store.failed() {
            Ok(entries) => self.failed_count = entries.len(),
            Err(e) => self.record_error(e.to_string()),
        }

        self.ensure_similar_permanent();
    }

    fn ensure_similar_permanent(&mut self) {
        let Some(item) = self.inbox.get(self.cursor) else {
            return;
        };
        let key = (item.kind, item.slug.clone());
        if self.similar_cache.contains_key(&key) {
            return;
        }
        match self.store.similar_permanent(item.kind, &item.slug) {
            Ok(list) => {
                self.similar_cache.insert(key, list);
            }
            Err(e) => self.record_error(e.to_string()),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<Effect> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        self.message = None;

        let pending = self.pending_key.take();
        match self.settings.keys.resolve(self.screen, pending, key) {
            Resolved::Pending => {
                self.pending_key = Some(key);
                None
            }
            Resolved::Nothing => None,
            Resolved::Action(action) => self.handle_action(action),
        }
    }

    fn handle_action(&mut self, action: Action) -> Option<Effect> {
        if self.screen != Screen::Inbox {
            match action {
                Action::Quit | Action::Back => {
                    self.screen = Screen::Inbox;
                    self.show_help = false;
                    self.reload_inbox();
                }
                Action::Help => {
                    self.show_help = !self.show_help;
                }
                _ => {}
            }
            return None;
        }

        match action {
            Action::Down => {
                if self.cursor + 1 < self.inbox.len() {
                    self.cursor += 1;
                    self.detail_scroll = 0;
                    self.ensure_similar_permanent();
                }
            }
            Action::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.detail_scroll = 0;
                    self.ensure_similar_permanent();
                }
            }
            Action::Top => {
                if !self.inbox.is_empty() && self.cursor != 0 {
                    self.cursor = 0;
                    self.detail_scroll = 0;
                    self.ensure_similar_permanent();
                }
            }
            Action::Bottom => {
                if !self.inbox.is_empty() {
                    let last = self.inbox.len() - 1;
                    if self.cursor != last {
                        self.cursor = last;
                        self.detail_scroll = 0;
                        self.ensure_similar_permanent();
                    }
                }
            }
            Action::ScrollDown => {
                self.detail_scroll = self.detail_scroll.saturating_add(5);
            }
            Action::ScrollUp => {
                self.detail_scroll = self.detail_scroll.saturating_sub(5);
            }
            Action::Help => {
                self.show_help = !self.show_help;
            }
            Action::Quit => {
                return Some(Effect::Quit);
            }
            Action::Toggle => {
                if let Some(item) = self.inbox.get(self.cursor) {
                    let key = (item.kind, item.slug.clone());
                    if !self.selected.remove(&key) {
                        self.selected.insert(key);
                    }
                }
            }
            Action::ClearSelection => {
                self.selected.clear();
            }
            Action::ToReview => {
                self.screen = Screen::Review;
                self.show_help = false;
            }
            Action::ToFailed => {
                self.screen = Screen::Failed;
                self.show_help = false;
            }
            _ => {}
        }
        None
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn inbox(&self) -> &[InboxItem] {
        &self.inbox
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn current_item(&self) -> Option<&InboxItem> {
        self.inbox.get(self.cursor)
    }

    pub fn is_selected(&self, item: &InboxItem) -> bool {
        self.selected.contains(&(item.kind, item.slug.clone()))
    }

    pub fn is_similar_to_cursor(&self, item: &InboxItem) -> bool {
        let Some(cur) = self.current_item() else {
            return false;
        };
        if cur.kind == item.kind && cur.slug == item.slug {
            return false;
        }
        let Some(va) = self.embeddings.get(&(cur.kind, cur.slug.clone())) else {
            return false;
        };
        let Some(vb) = self.embeddings.get(&(item.kind, item.slug.clone())) else {
            return false;
        };
        cosine_similarity(va, vb).is_some_and(|s| s >= self.settings.inbox_similarity)
    }

    pub fn current_similar_permanent(&self) -> &[Near] {
        let Some(item) = self.current_item() else {
            return &[];
        };
        self.similar_cache
            .get(&(item.kind, item.slug.clone()))
            .map(|v| &v[..v.len().min(3)])
            .unwrap_or(&[])
    }

    pub fn old_count(&self, now: DateTime<Utc>) -> usize {
        self.inbox.iter().filter(|i| i.is_old(now)).count()
    }

    pub fn review_count(&self) -> usize {
        self.review_count
    }

    pub fn failed_count(&self) -> usize {
        self.failed_count
    }

    pub fn detail_scroll(&self) -> u16 {
        self.detail_scroll
    }

    pub fn show_help(&self) -> bool {
        self.show_help
    }

    pub fn usable_keys(&self) -> Vec<(String, &'static str)> {
        let pairs: &[(Action, &'static str)] = match self.screen {
            Screen::Inbox => &[
                (Action::Toggle, "選ぶ"),
                (Action::ClearSelection, "選択解除"),
                (Action::ToReview, "レビュー"),
                (Action::ToFailed, "失敗"),
                (Action::Help, "ヘルプ"),
                (Action::Quit, "終了"),
            ],
            Screen::Review | Screen::Failed => &[
                (Action::Back, "戻る"),
                (Action::Help, "ヘルプ"),
            ],
        };
        pairs
            .iter()
            .filter_map(|(act, desc)| {
                self.settings
                    .keys
                    .label(self.screen, *act)
                    .map(|k| (k, *desc))
            })
            .collect()
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        None
    } else {
        Some(dot / denom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::tui::store::FakeStore;
    use chrono::Duration;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    fn item(slug: &str, body: &str, days_ago: i64) -> InboxItem {
        InboxItem {
            kind: NoteKind::Fleeting,
            slug: slug.to_string(),
            body: body.to_string(),
            created: Utc::now() - Duration::days(days_ago),
            source_slug: None,
            source_id: None,
            locator: None,
        }
    }

    fn settings() -> Settings {
        Settings {
            inbox_similarity: 0.5,
            locator: false,
            theme: Theme::default(),
            keys: Keymap::default(),
        }
    }

    fn three_store() -> FakeStore {
        FakeStore {
            inbox: vec![
                item("20261117000001", "1つ目", 3),
                item("20261118000001", "2つ目", 2),
                item("20261119000001", "3つ目", 1),
            ],
            ..Default::default()
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn j_k_move_cursor_and_stop_at_ends() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(key('k'));
        assert_eq!(app.cursor(), 0);
        app.on_key(key('j'));
        app.on_key(key('j'));
        app.on_key(key('j'));
        assert_eq!(app.cursor(), 2);
    }

    #[test]
    fn gg_and_g_jump() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(key('G'));
        assert_eq!(app.cursor(), 2);
        app.on_key(key('g'));
        app.on_key(key('g'));
        assert_eq!(app.cursor(), 0);
        app.on_key(key('g'));
        app.on_key(key('j'));
        assert_eq!(app.cursor(), 1);
    }

    #[test]
    fn space_toggles_and_esc_clears_selection() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(key(' '));
        assert!(app.is_selected(&app.inbox()[0]));
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.is_selected(&app.inbox()[0]));
    }

    #[test]
    fn q_on_inbox_quits() {
        let mut app = App::new(Box::new(three_store()), settings());
        assert_eq!(app.on_key(key('q')), Some(Effect::Quit));
    }

    #[test]
    fn q_on_review_returns_to_inbox() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(key('r'));
        assert_eq!(app.screen(), Screen::Review);
        assert_eq!(app.on_key(key('q')), None);
        assert_eq!(app.screen(), Screen::Inbox);
    }

    #[test]
    fn similar_mark_follows_cursor() {
        let mut store = three_store();
        store.embeddings.insert((NoteKind::Fleeting, "20261117000001".into()), vec![1.0, 0.0]);
        store.embeddings.insert((NoteKind::Fleeting, "20261118000001".into()), vec![1.0, 0.0]);
        store.embeddings.insert((NoteKind::Fleeting, "20261119000001".into()), vec![0.0, 1.0]);
        let mut app = App::new(Box::new(store), settings());
        assert!(!app.is_similar_to_cursor(&app.inbox()[0]));
        assert!(app.is_similar_to_cursor(&app.inbox()[1]));
        assert!(!app.is_similar_to_cursor(&app.inbox()[2]));
        app.on_key(key('G'));
        assert!(!app.is_similar_to_cursor(&app.inbox()[0]));
        assert!(!app.is_similar_to_cursor(&app.inbox()[1]));
    }

    #[test]
    fn server_down_shows_message_and_keeps_running() {
        let store = FakeStore {
            fail_with: Some(Error::Transient("届きません".into())),
            ..Default::default()
        };
        let mut app = App::new(Box::new(store), settings());
        assert!(app.message().is_some_and(|m| m.contains("届きません")));
        app.on_key(key('j'));
        assert_eq!(app.on_key(key('q')), Some(Effect::Quit));
    }

    #[test]
    fn unbound_key_does_nothing() {
        let mut app = App::new(Box::new(three_store()), settings());
        assert_eq!(app.on_key(key('z')), None);
        assert_eq!(app.cursor(), 0);
        assert_eq!(app.message(), None);
    }
}
