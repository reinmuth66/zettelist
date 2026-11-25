use crate::error::Error;
use crate::notes::{InboxItem, Near, NewNote, Note, NoteKind, SourceSummary};
use crate::ops::WriteOutcome;
use crate::queue::Entry;
use crate::review::{Decision, HubSummary, Proposal, Rehubbed};
use crate::source_ref::{kind_hint, source_url};
use crate::tui::draft;
use crate::tui::input::LineInput;
use crate::tui::keys::{Action, Keymap, Resolved, Screen};
use crate::tui::store::Store;
use crate::tui::theme::Theme;
use chrono::{DateTime, Local, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    kind: MessageKind,
    text: String,
}

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
        if self.sources.iter().any(|s| s.slug == input) {
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

pub struct HubPicker {
    pub hubs: Vec<HubSummary>,
    pub cursor: usize,
}

enum ReviewEdit {
    Decide {
        cursor: usize,
        key: String,
        prev: Option<Decision>,
        next: Option<Decision>,
    },
    Rehub {
        cursor: usize,
        hub_slug: String,
        rehubbed: Rehubbed,
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
    detail_scroll: Cell<u16>,
    pending_key: Option<KeyEvent>,
    message: Option<Message>,
    review_count: usize,
    failed_count: usize,
    show_help: bool,
    write_form: Option<WriteForm>,
    promote: Option<PromoteState>,
    saved_promote_body: Option<String>,
    confirm_delete: Option<Vec<(NoteKind, String)>>,
    proposals: Vec<Proposal>,
    review_decisions: BTreeMap<String, Decision>,
    review_cursor: usize,
    review_notes: HashMap<String, Note>,
    hubs: Vec<HubSummary>,
    review_undo: Vec<ReviewEdit>,
    review_redo: Vec<ReviewEdit>,
    review_reason: Option<LineInput>,
    hub_picker: Option<HubPicker>,
    confirm_merge: bool,
    failed: Vec<Entry>,
    failed_cursor: usize,
    failed_scroll: u16,
    confirm_discard: Option<String>,
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
            detail_scroll: Cell::new(0),
            pending_key: None,
            message: None,
            review_count: 0,
            failed_count: 0,
            show_help: false,
            write_form: None,
            promote: None,
            saved_promote_body: None,
            confirm_delete: None,
            proposals: Vec::new(),
            review_decisions: BTreeMap::new(),
            review_cursor: 0,
            review_notes: HashMap::new(),
            hubs: Vec::new(),
            review_undo: Vec::new(),
            review_redo: Vec::new(),
            review_reason: None,
            hub_picker: None,
            confirm_merge: false,
            failed: Vec::new(),
            failed_cursor: 0,
            failed_scroll: 0,
            confirm_discard: None,
        };
        app.reload_inbox();
        app
    }

    fn set_info(&mut self, msg: impl Into<String>) {
        self.message = Some(Message {
            kind: MessageKind::Info,
            text: msg.into(),
        });
    }

    fn set_error(&mut self, msg: impl Into<String>) {
        self.message = Some(Message {
            kind: MessageKind::Error,
            text: msg.into(),
        });
    }

    fn record_error(&mut self, msg: String) {
        if self.message.is_none() {
            self.set_error(msg);
        }
    }

    pub fn reload_inbox(&mut self) {
        self.similar_cache.clear();
        self.embeddings.clear();
        self.detail_scroll.set(0);

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

        if self.show_help {
            self.show_help = false;
            self.pending_key = None;
            return None;
        }

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
        if self.review_reason.is_some() {
            self.handle_review_reason_key(key);
            return None;
        }
        if self.hub_picker.is_some() {
            self.handle_hub_picker_key(key);
            return None;
        }
        if self.confirm_merge {
            self.handle_confirm_merge_key(key);
            return None;
        }
        if self.confirm_discard.is_some() {
            self.handle_confirm_discard_key(key);
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

    pub fn on_paste(&mut self, text: &str) {
        if self.show_help {
            return;
        }
        if let Some(form) = self.write_form.as_mut() {
            let target = match form.focus {
                WriteField::Body => &mut form.body,
                WriteField::Source => &mut form.source,
                WriteField::Locator => &mut form.locator,
            };
            target.insert_str(text);
            return;
        }
        if let Some(PromoteState::Title { input, .. }) = self.promote.as_mut() {
            input.insert_str(text);
            return;
        }
        if let Some(input) = self.review_reason.as_mut() {
            input.insert_str(text);
        }
    }

    fn handle_action(&mut self, action: Action) -> Option<Effect> {
        if self.screen == Screen::Review {
            self.handle_review_action(action);
            return None;
        }
        if self.screen == Screen::Failed {
            self.handle_failed_action(action);
            return None;
        }

        match action {
            Action::Down => {
                if self.cursor + 1 < self.inbox.len() {
                    self.cursor += 1;
                    self.detail_scroll.set(0);
                    self.ensure_similar_permanent();
                }
            }
            Action::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.detail_scroll.set(0);
                    self.ensure_similar_permanent();
                }
            }
            Action::Top => {
                if !self.inbox.is_empty() && self.cursor != 0 {
                    self.cursor = 0;
                    self.detail_scroll.set(0);
                    self.ensure_similar_permanent();
                }
            }
            Action::Bottom => {
                if !self.inbox.is_empty() {
                    let last = self.inbox.len() - 1;
                    if self.cursor != last {
                        self.cursor = last;
                        self.detail_scroll.set(0);
                        self.ensure_similar_permanent();
                    }
                }
            }
            Action::ScrollDown => {
                self.detail_scroll.set(self.detail_scroll.get().saturating_add(5));
            }
            Action::ScrollUp => {
                self.detail_scroll.set(self.detail_scroll.get().saturating_sub(5));
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
                self.review_cursor = 0;
                self.review_undo.clear();
                self.review_redo.clear();
                self.review_reason = None;
                self.hub_picker = None;
                self.confirm_merge = false;
                self.reload_review();
            }
            Action::ToFailed => {
                self.screen = Screen::Failed;
                self.show_help = false;
                self.failed_cursor = 0;
                self.failed_scroll = 0;
                self.confirm_discard = None;
                self.reload_failed();
            }
            _ => {}
        }
        None
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_ref().map(|m| m.text.as_str())
    }

    pub fn message_kind(&self) -> Option<MessageKind> {
        self.message.as_ref().map(|m| m.kind)
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.set_error(msg);
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
        self.detail_scroll.get()
    }

    pub fn clamp_detail_scroll(&self, max: u16) {
        self.detail_scroll.set(self.detail_scroll.get().min(max));
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

    pub fn failed(&self) -> &[Entry] {
        &self.failed
    }

    pub fn failed_cursor(&self) -> usize {
        self.failed_cursor
    }

    pub fn current_failed(&self) -> Option<&Entry> {
        self.failed.get(self.failed_cursor)
    }

    pub fn failed_scroll(&self) -> u16 {
        self.failed_scroll
    }

    pub fn confirm_discard(&self) -> bool {
        self.confirm_discard.is_some()
    }

    pub fn proposals(&self) -> &[Proposal] {
        &self.proposals
    }

    pub fn review_cursor(&self) -> usize {
        self.review_cursor
    }

    pub fn current_proposal(&self) -> Option<&Proposal> {
        self.proposals.get(self.review_cursor)
    }

    pub fn current_decision(&self) -> Option<&Decision> {
        let prop = self.current_proposal()?;
        self.review_decisions.get(&prop.key())
    }

    pub fn review_counts(&self) -> (usize, usize) {
        let mut links = 0;
        let mut hubs = 0;
        for p in &self.proposals {
            match p {
                Proposal::Link { .. } => links += 1,
                Proposal::Hub { .. } => hubs += 1,
            }
        }
        (links, hubs)
    }

    pub fn review_note(&self, slug: &str) -> Option<&Note> {
        self.review_notes.get(slug)
    }

    pub fn hub_title(&self, slug: &str) -> Option<&str> {
        self.hubs
            .iter()
            .find(|h| h.slug == slug)
            .map(|h| h.title.as_str())
    }

    pub fn review_reason(&self) -> Option<&LineInput> {
        self.review_reason.as_ref()
    }

    pub fn hub_picker(&self) -> Option<&HubPicker> {
        self.hub_picker.as_ref()
    }

    pub fn confirm_merge_counts(&self) -> Option<(usize, usize)> {
        if !self.confirm_merge {
            return None;
        }
        let mut adopted = 0;
        let mut rejected = 0;
        for p in &self.proposals {
            match self.review_decisions.get(&p.key()) {
                Some(Decision::Adopt { .. }) => adopted += 1,
                Some(Decision::Reject) => rejected += 1,
                None => {}
            }
        }
        Some((adopted, rejected))
    }

    pub fn reload_review(&mut self) {
        self.detail_scroll.set(0);
        match self.store.proposals() {
            Ok((props, state)) => {
                self.review_count = props.len();
                self.proposals = props;
                self.review_decisions = state.decisions;
                if self.proposals.is_empty() {
                    self.review_cursor = 0;
                } else {
                    self.review_cursor = self.review_cursor.min(self.proposals.len() - 1);
                }
            }
            Err(e) => {
                self.proposals.clear();
                self.review_decisions.clear();
                self.review_cursor = 0;
                self.record_error(e.to_string());
            }
        }
        match self.store.hubs() {
            Ok(hubs) => self.hubs = hubs,
            Err(e) => self.record_error(e.to_string()),
        }
        self.ensure_review_notes();
    }

    fn ensure_review_notes(&mut self) {
        let Some(prop) = self.proposals.get(self.review_cursor).cloned() else {
            return;
        };
        match prop {
            Proposal::Link { from, to, .. } => {
                self.ensure_note(&from);
                self.ensure_note(&to);
            }
            Proposal::Hub { note, .. } => {
                self.ensure_note(&note);
            }
        }
    }

    fn ensure_note(&mut self, slug: &str) {
        if self.review_notes.contains_key(slug) {
            return;
        }
        match self.store.note(slug) {
            Ok(note) => {
                self.review_notes.insert(slug.to_string(), note);
            }
            Err(e) => self.record_error(e.to_string()),
        }
    }

    fn handle_review_action(&mut self, action: Action) {
        match action {
            Action::Quit | Action::Back => {
                self.screen = Screen::Inbox;
                self.show_help = false;
                self.reload_inbox();
                return;
            }
            Action::Help => {
                self.show_help = !self.show_help;
                return;
            }
            _ => {}
        }

        if self.proposals.is_empty() {
            return;
        }

        match action {
            Action::Next => {
                if self.review_cursor + 1 < self.proposals.len() {
                    self.review_cursor += 1;
                    self.detail_scroll.set(0);
                    self.ensure_review_notes();
                }
            }
            Action::Prev => {
                if self.review_cursor > 0 {
                    self.review_cursor -= 1;
                    self.detail_scroll.set(0);
                    self.ensure_review_notes();
                }
            }
            Action::ScrollDown => {
                self.detail_scroll.set(self.detail_scroll.get().saturating_add(5));
            }
            Action::ScrollUp => {
                self.detail_scroll.set(self.detail_scroll.get().saturating_sub(5));
            }
            Action::Adopt => {
                self.apply_review_decision(Decision::Adopt { reason: None });
            }
            Action::AdoptWithReason => {
                if matches!(self.current_proposal(), Some(Proposal::Link { .. })) {
                    self.pending_key = None;
                    self.review_reason = Some(LineInput::new(""));
                }
            }
            Action::Reject => {
                self.apply_review_decision(Decision::Reject);
            }
            Action::Change => {
                if let Some(Proposal::Hub { hub, .. }) = self.current_proposal() {
                    let hubs = self
                        .hubs
                        .iter()
                        .filter(|h| h.slug != *hub)
                        .cloned()
                        .collect();
                    self.pending_key = None;
                    self.hub_picker = Some(HubPicker { hubs, cursor: 0 });
                }
            }
            Action::Undo => self.undo_review(),
            Action::Redo => self.redo_review(),
            Action::Merge => {
                self.pending_key = None;
                self.confirm_merge = true;
            }
            _ => {}
        }
    }

    fn apply_review_decision(&mut self, decision: Decision) {
        let Some(prop) = self.current_proposal().cloned() else {
            return;
        };
        let key = prop.key();
        let prev = self.review_decisions.get(&key).cloned();
        match self.store.decide(&key, Some(decision.clone())) {
            Ok(()) => {
                self.review_decisions.insert(key.clone(), decision.clone());
                self.review_undo.push(ReviewEdit::Decide {
                    cursor: self.review_cursor,
                    key,
                    prev,
                    next: Some(decision),
                });
                self.review_redo.clear();
                if self.review_cursor + 1 < self.proposals.len() {
                    self.review_cursor += 1;
                    self.detail_scroll.set(0);
                    self.ensure_review_notes();
                }
            }
            Err(e) => self.handle_review_error(e),
        }
    }

    fn set_cached_decision(&mut self, key: &str, decision: Option<Decision>) {
        match decision {
            Some(d) => {
                self.review_decisions.insert(key.to_string(), d);
            }
            None => {
                self.review_decisions.remove(key);
            }
        }
    }

    fn handle_review_error(&mut self, err: Error) {
        match err {
            Error::Invalid(msg) => {
                self.set_error(msg);
                self.reload_review();
            }
            other => {
                self.set_error(other.to_string());
            }
        }
    }

    fn undo_review(&mut self) {
        let Some(edit) = self.review_undo.pop() else {
            return;
        };
        match edit {
            ReviewEdit::Decide { cursor, key, prev, next } => {
                match self.store.decide(&key, prev.clone()) {
                    Ok(()) => {
                        self.set_cached_decision(&key, prev.clone());
                        self.review_redo.push(ReviewEdit::Decide { cursor, key, prev, next });
                        if !self.proposals.is_empty() {
                            self.review_cursor = cursor.min(self.proposals.len() - 1);
                        }
                        self.detail_scroll.set(0);
                        self.ensure_review_notes();
                    }
                    Err(e) => {
                        self.review_undo.push(ReviewEdit::Decide { cursor, key, prev, next });
                        self.handle_review_error(e);
                    }
                }
            }
            ReviewEdit::Rehub { cursor, hub_slug, rehubbed } => {
                match self.store.unrehub(&rehubbed) {
                    Ok(()) => {
                        self.review_redo.push(ReviewEdit::Rehub { cursor, hub_slug, rehubbed });
                        self.review_cursor = cursor;
                        self.reload_review();
                    }
                    Err(e) => {
                        self.review_undo.push(ReviewEdit::Rehub { cursor, hub_slug, rehubbed });
                        self.handle_review_error(e);
                    }
                }
            }
        }
    }

    fn redo_review(&mut self) {
        let Some(edit) = self.review_redo.pop() else {
            return;
        };
        match edit {
            ReviewEdit::Decide { cursor, key, prev, next } => {
                match self.store.decide(&key, next.clone()) {
                    Ok(()) => {
                        self.set_cached_decision(&key, next.clone());
                        self.review_undo.push(ReviewEdit::Decide { cursor, key, prev, next });
                        if !self.proposals.is_empty() {
                            self.review_cursor = cursor.min(self.proposals.len() - 1);
                        }
                        self.detail_scroll.set(0);
                        self.ensure_review_notes();
                    }
                    Err(e) => {
                        self.review_redo.push(ReviewEdit::Decide { cursor, key, prev, next });
                        self.handle_review_error(e);
                    }
                }
            }
            ReviewEdit::Rehub { cursor, hub_slug, rehubbed } => {
                match self.store.rehub(&rehubbed.old_key, &hub_slug) {
                    Ok(new_rehubbed) => {
                        self.review_undo.push(ReviewEdit::Rehub {
                            cursor,
                            hub_slug,
                            rehubbed: new_rehubbed,
                        });
                        self.review_cursor = cursor;
                        self.reload_review();
                    }
                    Err(e) => {
                        self.review_redo.push(ReviewEdit::Rehub { cursor, hub_slug, rehubbed });
                        self.handle_review_error(e);
                    }
                }
            }
        }
    }

    fn handle_review_reason_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.review_reason = None;
            }
            KeyCode::Enter => {
                let Some(input) = self.review_reason.take() else {
                    return;
                };
                let trimmed = input.text().trim();
                let reason = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
                self.apply_review_decision(Decision::Adopt { reason });
            }
            _ => {
                if let Some(input) = self.review_reason.as_mut() {
                    input.handle(key);
                }
            }
        }
    }

    fn handle_hub_picker_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                self.hub_picker = None;
            }
            (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => {
                if let Some(p) = self.hub_picker.as_mut() {
                    if p.cursor + 1 < p.hubs.len() {
                        p.cursor += 1;
                    }
                }
            }
            (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => {
                if let Some(p) = self.hub_picker.as_mut() {
                    if p.cursor > 0 {
                        p.cursor -= 1;
                    }
                }
            }
            (KeyCode::Enter, _) => {
                let Some(picker) = self.hub_picker.take() else {
                    return;
                };
                let Some(chosen) = picker.hubs.get(picker.cursor).cloned() else {
                    return;
                };
                let Some(prop) = self.current_proposal().cloned() else {
                    return;
                };
                let key = prop.key();
                match self.store.rehub(&key, &chosen.slug) {
                    Ok(rehubbed) => {
                        self.review_undo.push(ReviewEdit::Rehub {
                            cursor: self.review_cursor,
                            hub_slug: chosen.slug,
                            rehubbed,
                        });
                        self.review_redo.clear();
                        self.reload_review();
                    }
                    Err(e) => self.handle_review_error(e),
                }
            }
            _ => {}
        }
    }

    fn handle_confirm_merge_key(&mut self, key: KeyEvent) {
        self.confirm_merge = false;
        if matches!((key.code, key.modifiers), (KeyCode::Char('y'), KeyModifiers::NONE)) {
            match self.store.merge() {
                Ok(done) => {
                    let s = done.value;
                    let mut msg = format!("マージしました (リンク {} 本、ハブ {} 件)", s.links, s.hubs);
                    let has_warning = done.queue_warning.is_some() || done.worker_warning.is_some();
                    for w in [done.queue_warning, done.worker_warning].into_iter().flatten() {
                        msg.push_str(" / ");
                        msg.push_str(&w);
                    }
                    if has_warning {
                        self.set_error(msg);
                    } else {
                        self.set_info(msg);
                    }
                    self.review_undo.clear();
                    self.review_redo.clear();
                    self.reload_review();
                }
                Err(e) => self.handle_review_error(e),
            }
        }
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
                let draft = draft::build_with_body(&items, self.saved_promote_body.as_deref());
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
                if reason.trim().is_empty() {
                    self.set_info("昇格を取り消しました");
                } else {
                    self.set_error(format!("昇格を取り消しました: {reason}"));
                }
                return;
            }
        };
        let Some(parsed) = draft::parse(&text) else {
            self.set_info("昇格を取り消しました");
            return;
        };
        match self.store.promote(&from, &title, &parsed) {
            Ok(done) => {
                self.saved_promote_body = None;
                let mut msg = format!("昇格しました {}", done.value);
                let has_warning = done.queue_warning.is_some() || done.worker_warning.is_some();
                for w in [done.queue_warning, done.worker_warning].into_iter().flatten() {
                    msg.push_str(" / ");
                    msg.push_str(&w);
                }
                if has_warning {
                    self.set_error(msg);
                } else {
                    self.set_info(msg);
                }
                self.selected.clear();
                self.reload_inbox();
            }
            Err(e) => {
                self.saved_promote_body = Some(parsed);
                self.set_error(format!("{e} (書いた本文は次の昇格の下書きに残します)"));
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
                    self.set_error(e.to_string());
                    self.reload_inbox();
                }
            }
        }
    }

    fn reload_failed(&mut self) {
        self.failed_scroll = 0;
        match self.store.failed() {
            Ok(entries) => {
                self.failed = entries;
                self.failed_count = self.failed.len();
                if self.failed.is_empty() {
                    self.failed_cursor = 0;
                } else {
                    self.failed_cursor = self.failed_cursor.min(self.failed.len() - 1);
                }
            }
            Err(e) => {
                self.failed.clear();
                self.failed_cursor = 0;
                self.record_error(e.to_string());
            }
        }
    }

    fn record_done_warnings(&mut self, queue_warning: Option<String>, worker_warning: Option<String>) {
        let warnings: Vec<String> = [queue_warning, worker_warning].into_iter().flatten().collect();
        if !warnings.is_empty() {
            self.set_error(warnings.join(" / "));
        }
    }

    fn handle_failed_action(&mut self, action: Action) {
        match action {
            Action::Quit | Action::Back => {
                self.screen = Screen::Inbox;
                self.show_help = false;
                self.confirm_discard = None;
                self.reload_inbox();
            }
            Action::Help => {
                self.show_help = !self.show_help;
            }
            Action::Down => {
                if self.failed_cursor + 1 < self.failed.len() {
                    self.failed_cursor += 1;
                    self.failed_scroll = 0;
                }
            }
            Action::Up => {
                if self.failed_cursor > 0 {
                    self.failed_cursor -= 1;
                    self.failed_scroll = 0;
                }
            }
            Action::Top => {
                if !self.failed.is_empty() && self.failed_cursor != 0 {
                    self.failed_cursor = 0;
                    self.failed_scroll = 0;
                }
            }
            Action::Bottom => {
                if !self.failed.is_empty() {
                    let last = self.failed.len() - 1;
                    if self.failed_cursor != last {
                        self.failed_cursor = last;
                        self.failed_scroll = 0;
                    }
                }
            }
            Action::ScrollDown => {
                self.failed_scroll = self.failed_scroll.saturating_add(5);
            }
            Action::ScrollUp => {
                self.failed_scroll = self.failed_scroll.saturating_sub(5);
            }
            Action::Retry => {
                if let Some(entry) = self.failed.get(self.failed_cursor) {
                    let id = entry.id.clone();
                    match self.store.retry(&id) {
                        Ok(done) => self.record_done_warnings(done.queue_warning, done.worker_warning),
                        Err(e) => self.record_error(e.to_string()),
                    }
                    self.reload_failed();
                }
            }
            Action::Discard => {
                if let Some(entry) = self.failed.get(self.failed_cursor) {
                    self.pending_key = None;
                    self.confirm_discard = Some(entry.id.clone());
                }
            }
            Action::RetryAll if !self.failed.is_empty() => {
                match self.store.retry_all() {
                    Ok(done) => self.record_done_warnings(done.queue_warning, done.worker_warning),
                    Err(e) => self.record_error(e.to_string()),
                }
                self.reload_failed();
            }
            _ => {}
        }
    }

    fn handle_confirm_discard_key(&mut self, key: KeyEvent) {
        let Some(id) = self.confirm_discard.take() else {
            return;
        };
        if matches!((key.code, key.modifiers), (KeyCode::Char('y'), KeyModifiers::NONE)) {
            if let Err(e) = self.store.discard(&id) {
                self.record_error(e.to_string());
            }
            self.reload_failed();
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
                let (mut msg, mut is_err) = match done.value {
                    WriteOutcome::Saved(slug) => (format!("保存しました {slug}"), false),
                    WriteOutcome::Queued(e) => {
                        let m = if e.is_transient() {
                            "omnigraph-server に届かないので待ち行列に保存しました".to_string()
                        } else {
                            format!("書けなかったので待ち行列に保存しました: {e}")
                        };
                        (m, true)
                    }
                };
                for w in [done.queue_warning, done.worker_warning].into_iter().flatten() {
                    is_err = true;
                    msg.push_str(" / ");
                    msg.push_str(&w);
                }
                if is_err {
                    self.set_error(msg);
                } else {
                    self.set_info(msg);
                }
                self.reload_inbox();
            }
            Err(e) => {
                self.set_error(e.to_string());
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
        if self.review_reason.is_some() {
            return vec![
                ("Enter".to_string(), "決定"),
                ("Esc".to_string(), "中止"),
            ];
        }
        if self.hub_picker.is_some() {
            return vec![
                ("j/k".to_string(), "選ぶ"),
                ("Enter".to_string(), "決定"),
                ("Esc".to_string(), "中止"),
            ];
        }
        if self.confirm_merge {
            return vec![
                ("y".to_string(), "マージ"),
                ("他".to_string(), "中止"),
            ];
        }
        if self.confirm_discard.is_some() {
            return vec![
                ("y".to_string(), "捨てる"),
                ("他".to_string(), "中止"),
            ];
        }
        if self.screen == Screen::Review {
            let mut pairs: Vec<(Action, &'static str)> = Vec::new();
            if let Some(prop) = self.current_proposal() {
                pairs.push((Action::Adopt, "採用"));
                if matches!(prop, Proposal::Link { .. }) {
                    pairs.push((Action::AdoptWithReason, "理由付き採用"));
                }
                pairs.push((Action::Reject, "却下"));
                if matches!(prop, Proposal::Hub { .. }) {
                    pairs.push((Action::Change, "Hub変更"));
                }
                pairs.extend([
                    (Action::Next, "次へ"),
                    (Action::Prev, "前へ"),
                    (Action::Undo, "取消"),
                    (Action::Redo, "やり直し"),
                    (Action::Merge, "マージ"),
                ]);
            }
            pairs.extend([
                (Action::Back, "戻る"),
                (Action::Help, "ヘルプ"),
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
        if self.screen == Screen::Inbox {
            let mut pairs: Vec<(Action, &'static str)> = vec![(
                Action::Write,
                if self.selected.is_empty() { "書く" } else { "昇格" },
            )];
            if !self.inbox.is_empty() {
                pairs.push((Action::Toggle, "選ぶ"));
            }
            if !self.selected.is_empty() {
                pairs.push((Action::ClearSelection, "選択解除"));
            }
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
        let mut pairs: Vec<(Action, &'static str)> = Vec::new();
        if !self.failed.is_empty() {
            pairs.extend([
                (Action::Retry, "やり直し"),
                (Action::Discard, "捨てる"),
                (Action::RetryAll, "全部やり直し"),
            ]);
        }
        pairs.extend([(Action::Back, "戻る"), (Action::Help, "ヘルプ")]);
        pairs
            .into_iter()
            .filter_map(|(act, desc)| {
                self.settings
                    .keys
                    .label(self.screen, act)
                    .map(|k| (k, desc))
            })
            .collect()
    }

    pub fn help_keys(&self) -> Vec<(String, &'static str)> {
        let mut out: Vec<(String, &'static str)> = Vec::new();
        let label = |act: Action| self.settings.keys.label(self.screen, act);

        match self.screen {
            Screen::Inbox => {
                let pairs = [
                    (
                        Action::Write,
                        if self.selected.is_empty() {
                            "新しいメモを書く"
                        } else {
                            "選んだものから Permanent を書く"
                        },
                    ),
                    (Action::Toggle, "選ぶ / 外す"),
                    (Action::ClearSelection, "選択を全部外す"),
                ];
                for (act, desc) in pairs {
                    if let Some(k) = label(act) {
                        out.push((k, desc));
                    }
                }
                if !self.inbox.is_empty() {
                    if let Some(k) = label(Action::Delete) {
                        out.push((k, "削除 (確認あり)"));
                    }
                }
                for (act, desc) in [
                    (Action::ToReview, "レビューへ"),
                    (Action::ToFailed, "失敗の一覧へ"),
                ] {
                    if let Some(k) = label(act) {
                        out.push((k, desc));
                    }
                }
                if self.current_item().is_some_and(|i| {
                    i.kind == NoteKind::Literature
                        && i.source_id.as_deref().and_then(source_url).is_some()
                }) {
                    if let Some(k) = label(Action::OpenSource) {
                        out.push((k, "出典の元のページを開く"));
                    }
                }
            }
            Screen::Review => {
                if let Some(prop) = self.current_proposal() {
                    if let Some(k) = label(Action::Adopt) {
                        out.push((k, "採用して次へ"));
                    }
                    if matches!(prop, Proposal::Link { .. }) {
                        if let Some(k) = label(Action::AdoptWithReason) {
                            out.push((k, "理由を書いて採用"));
                        }
                    }
                    if let Some(k) = label(Action::Reject) {
                        out.push((k, "却下して次へ"));
                    }
                    if matches!(prop, Proposal::Hub { .. }) {
                        if let Some(k) = label(Action::Change) {
                            out.push((k, "別の Hub を選び直す"));
                        }
                    }
                    if let (Some(n), Some(p)) = (label(Action::Next), label(Action::Prev)) {
                        out.push((format!("{n} / {p}"), "判断せずに次 / 前へ"));
                    }
                    if let (Some(u), Some(r)) = (label(Action::Undo), label(Action::Redo)) {
                        out.push((format!("{u} / {r}"), "直前の判断を取り消す / やり直す"));
                    }
                    if let Some(k) = label(Action::Merge) {
                        out.push((k, "採用したものを main に入れる (確認あり)"));
                    }
                }
                if let Some(k) = label(Action::Back) {
                    out.push((k, "inbox へ"));
                }
            }
            Screen::Failed => {
                if !self.failed.is_empty() {
                    for (act, desc) in [
                        (Action::Retry, "やり直す"),
                        (Action::Discard, "捨てる (確認あり)"),
                        (Action::RetryAll, "全部やり直す"),
                    ] {
                        if let Some(k) = label(act) {
                            out.push((k, desc));
                        }
                    }
                }
                if let Some(k) = label(Action::Back) {
                    out.push((k, "inbox へ"));
                }
            }
        }

        if let (Some(d), Some(u)) = (label(Action::Down), label(Action::Up)) {
            out.push((format!("{d} / {u}"), "下 / 上へ移動"));
        }
        if let (Some(t), Some(b)) = (label(Action::Top), label(Action::Bottom)) {
            out.push((format!("{t} / {b}"), "先頭 / 末尾へ"));
        }
        if let (Some(sd), Some(su)) = (label(Action::ScrollDown), label(Action::ScrollUp)) {
            out.push((format!("{sd} / {su}"), "右の欄を下 / 上に送る"));
        }
        if let Some(k) = label(Action::Help) {
            out.push((k, "キーの一覧"));
        }
        if let Some(k) = label(Action::Quit) {
            let desc = if self.screen == Screen::Inbox {
                "終了"
            } else {
                "inbox に戻る"
            };
            out.push((k, desc));
        }

        out.extend([
            ("Enter / Esc".to_string(), "保存 / やめる"),
            ("Tab / Shift-Tab".to_string(), "次 / 前の欄へ"),
            ("← → Ctrl-a Ctrl-e".to_string(), "カーソル移動、行頭、行末"),
            ("Ctrl-w / Ctrl-u".to_string(), "1 語消す / 行頭まで消す"),
        ]);

        out
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
        assert_eq!(st.promoted.len(), 1);
        assert_eq!(
            st.promoted[0].from,
            vec![
                (NoteKind::Fleeting, "20261117000001".to_string()),
                (NoteKind::Fleeting, "20261118000001".to_string()),
            ]
        );
        assert_eq!(st.promoted[0].title, "小さいノート");
        assert_eq!(st.promoted[0].body, "本文");
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
        assert_eq!(st.deleted.len(), 1);
        assert_eq!(st.deleted[0], vec![(NoteKind::Fleeting, "20261117000001".to_string())]);
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

    fn review_store() -> FakeStore {
        use crate::notes::Note;
        use crate::review::{HubSummary, Proposal};

        let mut store = three_store();
        store.proposals = vec![
            Proposal::Link {
                from: "a".into(),
                to: "b".into(),
                similarity: 0.82,
            },
            Proposal::Link {
                from: "a".into(),
                to: "c".into(),
                similarity: 0.71,
            },
            Proposal::Hub {
                hub: "hub1".into(),
                note: "a".into(),
            },
        ];
        store.notes.insert(
            "a".into(),
            Note {
                kind: NoteKind::Permanent,
                slug: "a".into(),
                title: Some("ノートA".into()),
                body: "Aの本文".into(),
                created: Utc::now(),
            },
        );
        store.notes.insert(
            "b".into(),
            Note {
                kind: NoteKind::Permanent,
                slug: "b".into(),
                title: Some("ノートB".into()),
                body: "Bの本文".into(),
            created: Utc::now(),
            },
        );
        store.notes.insert(
            "c".into(),
            Note {
                kind: NoteKind::Permanent,
                slug: "c".into(),
                title: Some("ノートC".into()),
                body: "Cの本文".into(),
                created: Utc::now(),
            },
        );
        store.hubs = vec![
            HubSummary {
                slug: "hub1".into(),
                title: "最初のHub".into(),
            },
            HubSummary {
                slug: "hub2".into(),
                title: "別のHub".into(),
            },
        ];
        store
    }

    #[test]
    fn adopt_records_and_moves_next() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('a'));
        assert_eq!(app.review_cursor(), 1);
        let st = store.borrow();
        assert_eq!(
            st.review_state.decisions.get("link:a:b"),
            Some(&crate::review::Decision::Adopt { reason: None })
        );
    }

    #[test]
    fn adopt_with_reason() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('A'));
        type_text(&mut app, "同じ主張");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.review_cursor(), 1);
        let st = store.borrow();
        assert_eq!(
            st.review_state.decisions.get("link:a:b"),
            Some(&crate::review::Decision::Adopt {
                reason: Some("同じ主張".to_string())
            })
        );
    }

    #[test]
    fn adopt_with_blank_reason_is_none() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('A'));
        type_text(&mut app, "  ");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let st = store.borrow();
        assert_eq!(
            st.review_state.decisions.get("link:a:b"),
            Some(&crate::review::Decision::Adopt { reason: None })
        );
    }

    #[test]
    fn capital_a_does_nothing_on_hub() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('n'));
        app.on_key(key('n'));
        assert_eq!(app.review_cursor(), 2);
        app.on_key(key('A'));
        assert!(app.review_reason().is_none());
        assert!(!store.borrow().calls.iter().any(|c| c == "decide"));
    }

    #[test]
    fn reject_records_and_moves_next() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('d'));
        assert_eq!(app.review_cursor(), 1);
        let st = store.borrow();
        assert_eq!(
            st.review_state.decisions.get("link:a:b"),
            Some(&crate::review::Decision::Reject)
        );
    }

    #[test]
    fn undo_and_redo_decision() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('a'));
        assert_eq!(app.review_cursor(), 1);

        app.on_key(key('u'));
        assert_eq!(app.review_cursor(), 0);
        assert_eq!(store.borrow().review_state.decisions.get("link:a:b"), None);

        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(
            store.borrow().review_state.decisions.get("link:a:b"),
            Some(&crate::review::Decision::Adopt { reason: None })
        );
    }

    #[test]
    fn new_decision_clears_redo() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('a'));
        app.on_key(key('u'));
        app.on_key(key('d'));
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(
            store.borrow().review_state.decisions.get("link:a:b"),
            Some(&crate::review::Decision::Reject)
        );
    }

    #[test]
    fn change_hub_calls_rehub_and_undo_calls_unrehub() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('n'));
        app.on_key(key('n'));
        app.on_key(key('c'));
        assert!(app.hub_picker().is_some());
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.hub_picker().is_none());
        assert!(store.borrow().calls.iter().any(|c| c == "rehub"));

        app.on_key(key('u'));
        assert!(store.borrow().calls.iter().any(|c| c == "unrehub"));

        store.borrow_mut().calls.clear();
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(store.borrow().calls.iter().any(|c| c == "rehub"));
    }

    #[test]
    fn merge_asks_then_merges() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('a'));
        app.on_key(key('m'));
        assert!(!store.borrow().calls.iter().any(|c| c == "merge"));
        assert!(app.confirm_merge_counts().is_some());

        app.on_key(key('y'));
        assert!(store.borrow().calls.iter().any(|c| c == "merge"));
        assert!(app.message().is_some_and(|m| m.contains("マージしました")));
    }

    #[test]
    fn vanished_proposal_reloads() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        store.borrow_mut().calls.clear();
        store.borrow_mut().fail_with = Some(Error::Invalid("提案 'link:a:b' はありません".into()));

        app.on_key(key('a'));
        assert_eq!(app.message(), Some("提案 'link:a:b' はありません"));
        assert!(store.borrow().calls.iter().any(|c| c == "proposals"));
    }

    #[test]
    fn esc_returns_to_inbox() {
        let mut app = App::new(Box::new(review_store()), settings());
        app.on_key(key('r'));
        assert_eq!(app.screen(), Screen::Review);
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Inbox);
    }

    fn failed_store() -> FakeStore {
        use crate::queue::{Entry, Job};
        use std::time::SystemTime;

        let mut store = three_store();
        store.failed = vec![
            Entry {
                id: "100-1".into(),
                job: Job::Embed {
                    kind: NoteKind::Fleeting,
                    slug: "20261117000001".into(),
                },
                attempts: 3,
                not_before: SystemTime::UNIX_EPOCH,
                last_error: Some("1 回目のエラー\n詳しい理由".into()),
            },
            Entry {
                id: "200-1".into(),
                job: Job::Promote {
                    slug: "20261118000001".into(),
                },
                attempts: 3,
                not_before: SystemTime::UNIX_EPOCH,
                last_error: Some("2 回目のエラー".into()),
            },
        ];
        store
    }

    #[test]
    fn f_opens_failed_list() {
        let mut app = App::new(Box::new(failed_store()), settings());
        app.on_key(key('f'));
        assert_eq!(app.screen(), Screen::Failed);
        assert_eq!(app.failed().len(), 2);
    }

    #[test]
    fn enter_retries_and_reloads() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(failed_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('f'));
        store.borrow_mut().calls.clear();

        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let calls = &store.borrow().calls;
        let retry_pos = calls.iter().position(|c| c == "retry").expect("retry called");
        let failed_pos = calls.iter().rposition(|c| c == "failed").expect("failed reloaded");
        assert!(retry_pos < failed_pos);
        assert_eq!(app.failed().len(), 1);
    }

    #[test]
    fn x_asks_then_discards() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(failed_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('f'));
        store.borrow_mut().calls.clear();

        app.on_key(key('x'));
        assert!(app.confirm_discard());
        assert!(!store.borrow().calls.iter().any(|c| c == "discard"));

        app.on_key(key('y'));
        assert!(!app.confirm_discard());
        assert!(store.borrow().calls.iter().any(|c| c == "discard"));
        assert!(store.borrow().calls.iter().any(|c| c == "failed"));
        assert_eq!(app.failed().len(), 1);
    }

    #[test]
    fn capital_r_retries_all() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(failed_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('f'));
        store.borrow_mut().calls.clear();

        app.on_key(key('R'));
        let calls = &store.borrow().calls;
        let all_pos = calls.iter().position(|c| c == "retry_all").expect("retry_all called");
        let failed_pos = calls.iter().rposition(|c| c == "failed").expect("failed reloaded");
        assert!(all_pos < failed_pos);
        assert!(app.failed().is_empty());
    }

    #[test]
    fn help_lists_keys_and_closes_on_any_key() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(key('?'));
        assert!(app.show_help());
        assert!(!app.help_keys().is_empty());

        let eff = app.on_key(key('q'));
        assert!(eff.is_none());
        assert!(!app.show_help());

        app.on_key(key('?'));
        assert!(app.show_help());
        let eff2 = app.on_key(key('z'));
        assert!(eff2.is_none());
        assert!(!app.show_help());
    }

    #[test]
    fn on_paste_inserts_into_open_input_and_ignores_when_closed() {
        let mut app = App::new(Box::new(three_store()), settings());
        // 入力欄が閉じているときは何もしない
        app.on_paste("dy\nmy");
        assert!(app.confirm_delete_count().is_none());

        // 書く欄が開いているときは改行を空白にして差し込む
        app.on_key(key('i'));
        app.on_paste("1行目\r\n2行目\nd");
        assert_eq!(app.write_form().unwrap().body.text(), "1行目 2行目 d");
        assert!(app.write_form().is_some());

        // 題の欄でも同様
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.on_key(key(' '));
        app.on_key(key('i'));
        app.on_paste("題1\n題2");
        assert_eq!(app.promote_title().unwrap().text(), "題1 題2");

        // 理由の欄でも同様
        let mut review_app = App::new(Box::new(review_store()), settings());
        review_app.on_key(key('r'));
        review_app.on_key(key('A'));
        review_app.on_paste("理由1\n理由2");
        assert_eq!(review_app.review_reason().unwrap().text(), "理由1 理由2");
    }

    #[test]
    fn promote_failure_keeps_body_until_next_promote_succeeds() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key(' '));
        app.on_key(key('i'));
        app.on_key(key('題'));
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        store.borrow_mut().fail_with = Some(Error::Transient("一時失敗".into()));
        app.finish_promote(Ok("残すべき本文".into()));
        assert_eq!(app.message_kind(), Some(MessageKind::Error));
        assert!(
            app.message()
                .is_some_and(|m| m.contains("書いた本文は次の昇格の下書きに残します")),
            "msg={:?}",
            app.message()
        );

        // 同じ昇格をやり直すと、目印の文の代わりに残した本文が先頭に入る
        app.on_key(key('i'));
        app.on_key(key('題'));
        let eff = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let Effect::Edit { draft } = eff.unwrap() else {
            panic!("expected Edit");
        };
        assert!(draft.starts_with("残すべき本文\n"), "draft={draft}");
        assert!(!draft.contains("(ここに Permanent を書く)"), "draft={draft}");

        // 成功したら捨てられる
        store.borrow_mut().fail_with = None;
        app.finish_promote(Ok(draft));
        assert_eq!(app.message_kind(), Some(MessageKind::Info));

        app.on_key(key(' '));
        app.on_key(key('i'));
        app.on_key(key('別'));
        let eff2 = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let Effect::Edit { draft: draft2 } = eff2.unwrap() else {
            panic!("expected Edit");
        };
        assert!(draft2.starts_with("(ここに Permanent を書く)\n"), "draft2={draft2}");
    }

    #[test]
    fn delete_failure_reloads_inbox() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(three_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        store.borrow_mut().calls.clear();
        store.borrow_mut().fail_with = Some(Error::Transient("途中で失敗".into()));

        app.on_key(key('d'));
        app.on_key(key('y'));
        assert_eq!(app.message(), Some("途中で失敗"));
        assert!(store.borrow().calls.iter().any(|c| c == "inbox"));
    }

    #[test]
    fn message_kind_is_info_on_success_and_error_on_failure() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(key('i'));
        app.on_key(key('a'));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.message_kind(), Some(MessageKind::Info));
        assert!(app.message().is_some_and(|m| m.contains("保存しました")));

        app.set_message("失敗");
        assert_eq!(app.message_kind(), Some(MessageKind::Error));
    }

    #[test]
    fn source_completion_hides_when_exact_match_exists() {
        let mut store = three_store();
        store.sources = vec![
            SourceSummary {
                slug: "ahrens2017".into(),
                kind: "book".into(),
            },
            SourceSummary {
                slug: "ahrens2017b".into(),
                kind: "book".into(),
            },
        ];
        let mut app = App::new(Box::new(store), settings());
        app.on_key(key('i'));
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        for c in "ahrens2017".chars() {
            app.on_key(key(c));
        }
        assert_eq!(app.write_form().unwrap().source_suggestion(), None);
    }

    #[test]
    fn undo_and_redo_failure_restores_history() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let store = Rc::new(RefCell::new(review_store()));
        let mut app = App::new(Box::new(Rc::clone(&store)), settings());
        app.on_key(key('r'));
        app.on_key(key('a'));
        assert!(app.current_decision().is_none()); // カーソルが次に進んだ

        // 1 回目の undo が失敗しても履歴が消えず、復旧後に undo できる
        store.borrow_mut().fail_with = Some(Error::Transient("server down".into()));
        app.on_key(key('u'));
        assert_eq!(app.message(), Some("server down"));

        store.borrow_mut().fail_with = None;
        app.on_key(key('u'));
        assert_eq!(app.review_cursor(), 0);
        assert!(app.current_decision().is_none());

        // redo も失敗時に履歴が消えず、復旧後に redo できる
        store.borrow_mut().fail_with = Some(Error::Transient("server down".into()));
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(app.message(), Some("server down"));

        store.borrow_mut().fail_with = None;
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(matches!(app.current_decision(), Some(Decision::Adopt { .. })));
    }
}
