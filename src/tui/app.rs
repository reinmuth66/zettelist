use crate::notes::{InboxItem, Near, NewNote, NoteKind, SourceSummary};
use crate::ops::WriteOutcome;
use crate::source_ref::{kind_hint, source_url};
use crate::tui::draft;
use crate::tui::input::LineInput;
use crate::tui::keys::{Action, Keymap, Resolved, Screen};
use crate::tui::store::Store;
use crate::tui::theme::Theme;
use chrono::{DateTime, Local, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteField {
    Body,
    Source,
    Locator,
}

pub struct WriteForm {
    pub body: LineInput,
    pub source: LineInput,
    pub locator: LineInput,
    pub focus: WriteField,
    sources: Vec<SourceSummary>,
}

impl WriteForm {
    /// 出典の入力に前方一致する最初の登録済み slug の残りを返す。
    pub fn source_suggestion(&self) -> Option<&str> {
        let input = self.source.text();
        if input.is_empty() {
            return None;
        }
        self.sources
            .iter()
            .find_map(|s| s.slug.strip_prefix(input).filter(|rest| !rest.is_empty()))
    }

    /// 位置の欄に出す薄い入力例を返す。位置に入力があれば出さない。
    pub fn locator_hint(&self) -> Option<&'static str> {
        if !self.locator.text().is_empty() {
            return None;
        }
        let src = self.source.text().trim();
        if src.is_empty() {
            return None;
        }
        let kind = self
            .sources
            .iter()
            .find(|s| s.slug == src)
            .map(|s| s.kind.as_str())
            .or_else(|| kind_hint(src))?;
        match kind {
            "book" => Some("p.42 / ch.3"),
            "paper" => Some("sec.3.2 / fig.4"),
            "article" => Some("見出し"),
            "video" => Some("12:30"),
            "talk" => Some("slide 14"),
            _ => None,
        }
    }
}

enum PromoteState {
    Title {
        from: Vec<(NoteKind, String)>,
        items: Vec<InboxItem>,
        input: LineInput,
    },
    Editing {
        from: Vec<(NoteKind, String)>,
        title: String,
    },
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
    write_form: Option<WriteForm>,
    promote: Option<PromoteState>,
    confirm_delete: Option<Vec<(NoteKind, String)>>,
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
            write_form: None,
            promote: None,
            confirm_delete: None,
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

        if self.write_form.is_some() {
            self.handle_write_key(key);
            return None;
        }
        if matches!(self.promote, Some(PromoteState::Title { .. })) {
            return self.handle_promote_title_key(key);
        }
        if self.confirm_delete.is_some() {
            self.handle_confirm_delete_key(key);
            return None;
        }

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
            Action::Write => {
                if self.selected.is_empty() {
                    self.open_write_form();
                } else {
                    self.open_promote_title();
                }
            }
            Action::Delete => {
                self.open_confirm_delete();
            }
            Action::OpenSource => {
                if let Some(item) = self.current_item() {
                    if item.kind == NoteKind::Literature {
                        if let Some(url) = item.source_id.as_deref().and_then(source_url) {
                            return Some(Effect::Open(url));
                        }
                    }
                }
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

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
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

    pub fn write_form(&self) -> Option<&WriteForm> {
        self.write_form.as_ref()
    }

    pub fn promote_title(&self) -> Option<&LineInput> {
        match &self.promote {
            Some(PromoteState::Title { input, .. }) => Some(input),
            _ => None,
        }
    }

    pub fn confirm_delete_count(&self) -> Option<usize> {
        self.confirm_delete.as_ref().map(Vec::len)
    }

    fn open_promote_title(&mut self) {
        let items: Vec<InboxItem> = self
            .inbox
            .iter()
            .filter(|i| self.is_selected(i))
            .cloned()
            .collect();
        if items.is_empty() {
            return;
        }
        let from = items.iter().map(|i| (i.kind, i.slug.clone())).collect();
        self.pending_key = None;
        self.promote = Some(PromoteState::Title {
            from,
            items,
            input: LineInput::new(""),
        });
    }

    fn handle_promote_title_key(&mut self, key: KeyEvent) -> Option<Effect> {
        match key.code {
            KeyCode::Esc => {
                self.promote = None;
                None
            }
            KeyCode::Enter => {
                let Some(PromoteState::Title { from, items, input }) = self.promote.take() else {
                    return None;
                };
                let title = input.text().trim().to_string();
                if title.is_empty() {
                    return None;
                }
                let draft = draft::build(&items);
                self.promote = Some(PromoteState::Editing { from, title });
                Some(Effect::Edit { draft })
            }
            _ => {
                if let Some(PromoteState::Title { input, .. }) = self.promote.as_mut() {
                    input.handle(key);
                }
                None
            }
        }
    }

    /// `$EDITOR` 終了後の結果を受け取って昇格を完了または取り消す。
    pub fn finish_promote(&mut self, body: Result<String, String>) {
        let Some(PromoteState::Editing { from, title }) = self.promote.take() else {
            return;
        };
        let text = match body {
            Ok(t) => t,
            Err(reason) => {
                self.message = Some(if reason.trim().is_empty() {
                    "昇格を取り消しました".to_string()
                } else {
                    format!("昇格を取り消しました: {reason}")
                });
                return;
            }
        };
        let Some(parsed) = draft::parse(&text) else {
            self.message = Some("昇格を取り消しました".to_string());
            return;
        };
        match self.store.promote(&from, &title, &parsed) {
            Ok(done) => {
                let mut msg = format!("昇格しました {}", done.value);
                for w in [done.queue_warning, done.worker_warning].into_iter().flatten() {
                    msg.push_str(" / ");
                    msg.push_str(&w);
                }
                self.message = Some(msg);
                self.selected.clear();
                self.reload_inbox();
            }
            Err(e) => {
                self.message = Some(e.to_string());
            }
        }
    }

    fn open_confirm_delete(&mut self) {
        if self.inbox.is_empty() {
            return;
        }
        let targets: Vec<(NoteKind, String)> = if self.selected.is_empty() {
            self.current_item()
                .map(|i| vec![(i.kind, i.slug.clone())])
                .unwrap_or_default()
        } else {
            self.inbox
                .iter()
                .filter(|i| self.is_selected(i))
                .map(|i| (i.kind, i.slug.clone()))
                .collect()
        };
        if targets.is_empty() {
            return;
        }
        self.pending_key = None;
        self.confirm_delete = Some(targets);
    }

    fn handle_confirm_delete_key(&mut self, key: KeyEvent) {
        let Some(targets) = self.confirm_delete.take() else {
            return;
        };
        if matches!((key.code, key.modifiers), (KeyCode::Char('y'), KeyModifiers::NONE)) {
            match self.store.delete(&targets) {
                Ok(()) => {
                    self.selected.clear();
                    self.reload_inbox();
                }
                Err(e) => {
                    self.message = Some(e.to_string());
                }
            }
        }
    }

    fn open_write_form(&mut self) {
        let sources = match self.store.sources() {
            Ok(list) => list,
            Err(e) => {
                self.record_error(e.to_string());
                Vec::new()
            }
        };
        self.pending_key = None;
        self.write_form = Some(WriteForm {
            body: LineInput::new(""),
            source: LineInput::new(""),
            locator: LineInput::new(""),
            focus: WriteField::Body,
            sources,
        });
    }

    fn handle_write_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.write_form = None;
                return;
            }
            KeyCode::Enter => {
                self.submit_write_form();
                return;
            }
            KeyCode::BackTab => {
                self.step_write_focus(false);
                return;
            }
            KeyCode::Tab => {
                let forward = !key.modifiers.contains(KeyModifiers::SHIFT);
                self.step_write_focus(forward);
                return;
            }
            _ => {}
        }

        let Some(form) = self.write_form.as_mut() else {
            return;
        };

        if form.focus == WriteField::Source
            && matches!(
                (key.code, key.modifiers),
                (KeyCode::Right, KeyModifiers::NONE) | (KeyCode::Char('e'), KeyModifiers::CONTROL)
            )
            && form.source.is_at_end()
        {
            if let Some(rest) = form.source_suggestion() {
                let full = format!("{}{}", form.source.text(), rest);
                form.source = LineInput::new(&full);
                return;
            }
        }

        let target = match form.focus {
            WriteField::Body => &mut form.body,
            WriteField::Source => &mut form.source,
            WriteField::Locator => &mut form.locator,
        };
        target.handle(key);
    }

    fn step_write_focus(&mut self, forward: bool) {
        let has_locator = self.settings.locator;
        let Some(form) = self.write_form.as_mut() else {
            return;
        };
        form.focus = match (form.focus, forward, has_locator) {
            (WriteField::Body, true, _) => WriteField::Source,
            (WriteField::Source, true, true) => WriteField::Locator,
            (WriteField::Source, true, false) => WriteField::Body,
            (WriteField::Locator, true, _) => WriteField::Body,
            (WriteField::Body, false, true) => WriteField::Locator,
            (WriteField::Body, false, false) => WriteField::Source,
            (WriteField::Source, false, _) => WriteField::Body,
            (WriteField::Locator, false, _) => WriteField::Source,
        };
    }

    fn submit_write_form(&mut self) {
        let Some(form) = self.write_form.as_ref() else {
            return;
        };
        let body = form.body.text().to_string();
        if body.trim().is_empty() {
            return;
        }
        let source_str = form.source.text().trim();
        let source = if source_str.is_empty() {
            None
        } else {
            Some(source_str.to_string())
        };
        let locator_str = form.locator.text().trim();
        let locator = if source.is_some() && !locator_str.is_empty() {
            Some(locator_str.to_string())
        } else {
            None
        };

        let note = NewNote {
            body,
            source,
            locator,
            created: Local::now(),
        };

        match self.store.write(note) {
            Ok(done) => {
                self.write_form = None;
                let mut msg = match done.value {
                    WriteOutcome::Saved(slug) => format!("保存しました {slug}"),
                    WriteOutcome::Queued(e) => {
                        if e.is_transient() {
                            "omnigraph-server に届かないので待ち行列に保存しました".to_string()
                        } else {
                            format!("書けなかったので待ち行列に保存しました: {e}")
                        }
                    }
                };
                for w in [done.queue_warning, done.worker_warning].into_iter().flatten() {
                    msg.push_str(" / ");
                    msg.push_str(&w);
                }
                self.message = Some(msg);
                self.reload_inbox();
            }
            Err(e) => {
                self.message = Some(e.to_string());
            }
        }
    }

    pub fn usable_keys(&self) -> Vec<(String, &'static str)> {
        if self.write_form.is_some() {
            return vec![
                ("Enter".to_string(), "保存"),
                ("Tab".to_string(), "次の欄"),
                ("Esc".to_string(), "中止"),
            ];
        }
        if self.promote_title().is_some() {
            return vec![
                ("Enter".to_string(), "決定"),
                ("Esc".to_string(), "中止"),
            ];
        }
        if self.confirm_delete.is_some() {
            return vec![
                ("y".to_string(), "削除"),
                ("他".to_string(), "中止"),
            ];
        }
        if self.screen == Screen::Inbox {
            let mut pairs: Vec<(Action, &'static str)> = vec![
                (
                    Action::Write,
                    if self.selected.is_empty() { "書く" } else { "昇格" },
                ),
                (Action::Toggle, "選ぶ"),
                (Action::ClearSelection, "選択解除"),
            ];
            if !self.inbox.is_empty() {
                pairs.push((Action::Delete, "削除"));
            }
            if self
                .current_item()
                .is_some_and(|i| i.kind == NoteKind::Literature && i.source_id.as_deref().and_then(source_url).is_some())
            {
                pairs.push((Action::OpenSource, "出典を開く"));
            }
            pairs.extend([
                (Action::ToReview, "レビュー"),
                (Action::ToFailed, "失敗"),
                (Action::Help, "ヘルプ"),
                (Action::Quit, "終了"),
            ]);
            return pairs
                .into_iter()
                .filter_map(|(act, desc)| {
                    self.settings
                        .keys
                        .label(self.screen, act)
                        .map(|k| (k, desc))
                })
                .collect();
        }
        let pairs: &[(Action, &'static str)] = &[
            (Action::Back, "戻る"),
            (Action::Help, "ヘルプ"),
        ];
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

    fn type_text(app: &mut App, s: &str) {
        for c in s.chars() {
            app.on_key(key(c));
        }
    }

    #[test]
    fn write_saves_fleeting() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('i'));
        type_text(&mut app, "箇条書きは考えを止める");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let st = store.borrow();
        assert!(st.calls.iter().any(|c| c == "write"));
        assert_eq!(st.written.len(), 1);
        assert_eq!(st.written[0].body, "箇条書きは考えを止める");
        assert_eq!(st.written[0].source, None);
        assert_eq!(st.written[0].locator, None);
        assert!(app.message().is_some_and(|m| m.contains("保存しました")));
        assert!(app.write_form().is_none());
    }

    #[test]
    fn write_with_source_saves_literature() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('i'));
        type_text(&mut app, "メモ本文");
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        type_text(&mut app, "ahrens2017");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let st = store.borrow();
        assert_eq!(st.written.len(), 1);
        assert_eq!(st.written[0].source.as_deref(), Some("ahrens2017"));
        assert_eq!(st.written[0].locator, None);
    }

    #[test]
    fn write_empty_body_does_nothing() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('i'));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(store.borrow().written.is_empty());
        assert!(app.write_form().is_some());
    }

    #[test]
    fn write_esc_cancels() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('i'));
        type_text(&mut app, "消える");
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(store.borrow().written.is_empty());
        assert!(app.write_form().is_none());
    }

    #[test]
    fn source_completion_accepts_with_right_arrow() {
        use crate::notes::SourceSummary;

        let mut store = three_store();
        store.sources = vec![SourceSummary {
            slug: "ahrens2017".into(),
            kind: "book".into(),
        }];
        let mut app = App::new(Box::new(store), settings());
        app.on_key(key('i'));
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        type_text(&mut app, "ahr");
        app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));

        assert_eq!(app.write_form().unwrap().source.text(), "ahrens2017");
    }

    #[test]
    fn locator_field_only_when_enabled() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(key('i'));
        assert_eq!(app.write_form().unwrap().focus, WriteField::Body);
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.write_form().unwrap().focus, WriteField::Source);
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.write_form().unwrap().focus, WriteField::Body);

        let mut s = settings();
        s.locator = true;
        let mut app2 = App::new(Box::new(three_store()), s);
        app2.on_key(key('i'));
        app2.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app2.write_form().unwrap().focus, WriteField::Source);
        app2.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app2.write_form().unwrap().focus, WriteField::Locator);
        app2.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app2.write_form().unwrap().focus, WriteField::Body);
    }

    #[test]
    fn queued_write_reports_queue() {
        use crate::ops::{Done, WriteOutcome};
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut store = three_store();
        store.write_result = Some(Ok(Done {
            value: WriteOutcome::Queued(Error::Transient("timeout".into())),
            queue_warning: None,
            worker_warning: None,
        }));
        let store = Rc::new(RefCell::new(store));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('i'));
        type_text(&mut app, "あとで送る");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(app.message().is_some_and(|m| m.contains("待ち行列に保存しました")));
        assert!(app.write_form().is_none());
    }

    #[test]
    fn i_with_selection_does_not_open_write_form() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(key(' '));
        app.on_key(key('i'));
        assert!(app.write_form().is_none());
    }

    #[test]
    fn promote_flow_calls_store() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key(' '));
        app.on_key(key('j'));
        app.on_key(key(' '));
        app.on_key(key('i'));
        type_text(&mut app, "小さいノート");
        let eff = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(eff, Some(Effect::Edit { .. })));

        app.finish_promote(Ok("本文\n# ---- この行から下は保存時に消える ----".to_string()));
        let st = store.borrow();
        assert!(st.calls.iter().any(|c| c == "promote"));
        assert!(!app.is_selected(&app.inbox()[0]));
        assert!(app.message().is_some_and(|m| m.contains("昇格しました")));
    }

    #[test]
    fn empty_title_cancels_promotion() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key(' '));
        app.on_key(key('i'));
        let eff = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(eff, None);
        assert!(!store.borrow().calls.iter().any(|c| c == "promote"));
    }

    #[test]
    fn editor_failure_cancels_promotion() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key(' '));
        app.on_key(key('i'));
        type_text(&mut app, "題名");
        let eff = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(eff, Some(Effect::Edit { .. })));

        app.finish_promote(Err("vi が見つかりません".to_string()));
        assert!(!store.borrow().calls.iter().any(|c| c == "promote"));
        assert!(app.message().is_some_and(|m| m.contains("vi が見つかりません")));
    }

    #[test]
    fn delete_asks_and_y_deletes() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('d'));
        assert!(!store.borrow().calls.iter().any(|c| c == "delete"));
        app.on_key(key('y'));
        let st = store.borrow();
        assert!(st.calls.iter().any(|c| c == "delete"));
        assert_eq!(st.inbox.len(), 2);
    }

    #[test]
    fn delete_other_key_cancels() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('d'));
        app.on_key(key('n'));
        let st = store.borrow();
        assert!(!st.calls.iter().any(|c| c == "delete"));
        assert_eq!(st.inbox.len(), 3);
    }

    #[test]
    fn open_only_for_literature_with_url() {
        let mut store = three_store();
        store.inbox.push(InboxItem {
            kind: NoteKind::Literature,
            slug: "20261120000001".into(),
            body: "論文メモ".into(),
            created: Utc::now(),
            source_slug: Some("smith2020".into()),
            source_id: Some("doi:10.1/x".into()),
            locator: None,
        });
        let mut app = App::new(Box::new(store), settings());
        assert_eq!(app.on_key(key('o')), None);
        app.on_key(key('G'));
        assert_eq!(
            app.on_key(key('o')),
            Some(Effect::Open("https://doi.org/10.1/x".to_string()))
        );
    }
}
