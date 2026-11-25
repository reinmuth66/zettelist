use crate::error::Error;
use crate::notes::{self, InboxItem, Near, NewNote, Note, NoteKind, SourceSummary};
use crate::ops::{self, Done, WriteOutcome};
use crate::queue::Entry;
use crate::review::{self, Decision, HubSummary, Proposal, Rehubbed, ReviewState, Summary};
use crate::Ctx;
use std::collections::HashMap;

/// TUI と裏のデータ (Omnigraph、waiting queue、review.json) の境界。
pub trait Store {
    fn inbox(&mut self) -> Result<Vec<InboxItem>, Error>;
    fn inbox_embeddings(&mut self) -> Result<HashMap<(NoteKind, String), Vec<f32>>, Error>;
    fn similar_permanent(&mut self, kind: NoteKind, slug: &str) -> Result<Vec<Near>, Error>;
    fn sources(&mut self) -> Result<Vec<SourceSummary>, Error>;
    fn write(&mut self, note: NewNote) -> Result<Done<WriteOutcome>, Error>;
    fn promote(&mut self, from: &[(NoteKind, String)], title: &str, body: &str) -> Result<Done<String>, Error>;
    fn delete(&mut self, notes: &[(NoteKind, String)]) -> Result<(), Error>;
    fn proposals(&mut self) -> Result<(Vec<Proposal>, ReviewState), Error>;
    fn note(&mut self, slug: &str) -> Result<Note, Error>;
    fn hubs(&mut self) -> Result<Vec<HubSummary>, Error>;
    fn decide(&mut self, key: &str, decision: Option<Decision>) -> Result<(), Error>;
    fn rehub(&mut self, key: &str, hub: &str) -> Result<Rehubbed, Error>;
    fn unrehub(&mut self, r: &Rehubbed) -> Result<(), Error>;
    fn merge(&mut self) -> Result<Done<Summary>, Error>;
    fn failed(&mut self) -> Result<Vec<Entry>, Error>;
    fn retry(&mut self, id: &str) -> Result<Done<()>, Error>;
    fn retry_all(&mut self) -> Result<Done<usize>, Error>;
    fn discard(&mut self, id: &str) -> Result<(), Error>;
}

/// 実環境向けの `Store`。`Ctx` を通して Omnigraph と待ち行列へつなぐ。
pub struct CtxStore(pub Ctx);

impl Store for CtxStore {
    fn inbox(&mut self) -> Result<Vec<InboxItem>, Error> {
        notes::inbox(&self.0.og)
    }

    fn inbox_embeddings(&mut self) -> Result<HashMap<(NoteKind, String), Vec<f32>>, Error> {
        notes::inbox_embeddings(&self.0.og)
    }

    fn similar_permanent(&mut self, kind: NoteKind, slug: &str) -> Result<Vec<Near>, Error> {
        ops::similar_permanent(&self.0, kind, slug)
    }

    fn sources(&mut self) -> Result<Vec<SourceSummary>, Error> {
        notes::sources(&self.0.og)
    }

    fn write(&mut self, note: NewNote) -> Result<Done<WriteOutcome>, Error> {
        ops::write(&self.0, note)
    }

    fn promote(&mut self, from: &[(NoteKind, String)], title: &str, body: &str) -> Result<Done<String>, Error> {
        ops::promote(&self.0, from, title, body)
    }

    fn delete(&mut self, notes: &[(NoteKind, String)]) -> Result<(), Error> {
        notes::delete_notes(&self.0.og, notes)
    }

    fn proposals(&mut self) -> Result<(Vec<Proposal>, ReviewState), Error> {
        let props = review::list_proposals(&self.0.og)?;
        let state = ReviewState::load(&self.0.paths.review_json())?;
        Ok((props, state))
    }

    fn note(&mut self, slug: &str) -> Result<Note, Error> {
        notes::show_note(&self.0.og, slug)
    }

    fn hubs(&mut self) -> Result<Vec<HubSummary>, Error> {
        review::hubs(&self.0.og)
    }

    fn decide(&mut self, key: &str, decision: Option<Decision>) -> Result<(), Error> {
        review::decide(&self.0.og, &self.0.paths.review_json(), &self.0.paths.review_lock(), key, decision)
    }

    fn rehub(&mut self, key: &str, hub: &str) -> Result<Rehubbed, Error> {
        review::rehub(&self.0.og, &self.0.paths.review_json(), &self.0.paths.review_lock(), key, hub)
    }

    fn unrehub(&mut self, r: &Rehubbed) -> Result<(), Error> {
        review::unrehub(&self.0.og, &self.0.paths.review_json(), &self.0.paths.review_lock(), r)
    }

    fn merge(&mut self) -> Result<Done<Summary>, Error> {
        ops::merge(&self.0)
    }

    fn failed(&mut self) -> Result<Vec<Entry>, Error> {
        self.0.queue.failed()
    }

    fn retry(&mut self, id: &str) -> Result<Done<()>, Error> {
        ops::retry(&self.0, id)
    }

    fn retry_all(&mut self) -> Result<Done<usize>, Error> {
        ops::retry_all(&self.0)
    }

    fn discard(&mut self, id: &str) -> Result<(), Error> {
        self.0.queue.discard_failed(id)
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct FakeStore {
    pub inbox: Vec<InboxItem>,
    pub embeddings: HashMap<(NoteKind, String), Vec<f32>>,
    pub similar_permanent: HashMap<(NoteKind, String), Vec<Near>>,
    pub sources: Vec<SourceSummary>,
    pub proposals: Vec<Proposal>,
    pub review_state: ReviewState,
    pub notes: HashMap<String, Note>,
    pub hubs: Vec<HubSummary>,
    pub failed: Vec<Entry>,
    pub calls: Vec<String>,
    pub fail_with: Option<Error>,
}

#[cfg(test)]
impl FakeStore {
    fn check_err(&mut self, name: &str) -> Result<(), Error> {
        self.calls.push(name.to_string());
        if let Some(ref e) = self.fail_with {
            return Err(match e {
                Error::Transient(m) => Error::Transient(m.clone()),
                Error::Permanent(m) => Error::Permanent(m.clone()),
                Error::Invalid(m) => Error::Invalid(m.clone()),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
impl Store for FakeStore {
    fn inbox(&mut self) -> Result<Vec<InboxItem>, Error> {
        self.check_err("inbox")?;
        Ok(self.inbox.clone())
    }

    fn inbox_embeddings(&mut self) -> Result<HashMap<(NoteKind, String), Vec<f32>>, Error> {
        self.check_err("inbox_embeddings")?;
        Ok(self.embeddings.clone())
    }

    fn similar_permanent(&mut self, kind: NoteKind, slug: &str) -> Result<Vec<Near>, Error> {
        self.check_err("similar_permanent")?;
        Ok(self.similar_permanent.get(&(kind, slug.to_string())).cloned().unwrap_or_default())
    }

    fn sources(&mut self) -> Result<Vec<SourceSummary>, Error> {
        self.check_err("sources")?;
        Ok(self.sources.clone())
    }

    fn write(&mut self, note: NewNote) -> Result<Done<WriteOutcome>, Error> {
        self.check_err("write")?;
        let slug = notes::note_slug(note.created.naive_local());
        Ok(Done {
            value: WriteOutcome::Saved(slug),
            queue_warning: None,
            worker_warning: None,
        })
    }

    fn promote(&mut self, _from: &[(NoteKind, String)], _title: &str, _body: &str) -> Result<Done<String>, Error> {
        self.check_err("promote")?;
        Ok(Done {
            value: "20261125000000".to_string(),
            queue_warning: None,
            worker_warning: None,
        })
    }

    fn delete(&mut self, notes: &[(NoteKind, String)]) -> Result<(), Error> {
        self.check_err("delete")?;
        self.inbox.retain(|item| !notes.iter().any(|(k, s)| *k == item.kind && s == &item.slug));
        Ok(())
    }

    fn proposals(&mut self) -> Result<(Vec<Proposal>, ReviewState), Error> {
        self.check_err("proposals")?;
        Ok((self.proposals.clone(), self.review_state.clone()))
    }

    fn note(&mut self, slug: &str) -> Result<Note, Error> {
        self.check_err("note")?;
        self.notes
            .get(slug)
            .cloned()
            .ok_or_else(|| Error::Permanent(format!("ノートが見つかりません: {slug}")))
    }

    fn hubs(&mut self) -> Result<Vec<HubSummary>, Error> {
        self.check_err("hubs")?;
        Ok(self.hubs.clone())
    }

    fn decide(&mut self, key: &str, decision: Option<Decision>) -> Result<(), Error> {
        self.check_err("decide")?;
        match decision {
            Some(d) => {
                self.review_state.decisions.insert(key.to_string(), d);
            }
            None => {
                self.review_state.decisions.remove(key);
            }
        }
        Ok(())
    }

    fn rehub(&mut self, key: &str, hub: &str) -> Result<Rehubbed, Error> {
        self.check_err("rehub")?;
        Ok(Rehubbed {
            old_key: key.to_string(),
            new_key: format!("hub:{hub}:"),
            created: true,
            previous: None,
        })
    }

    fn unrehub(&mut self, _r: &Rehubbed) -> Result<(), Error> {
        self.check_err("unrehub")?;
        Ok(())
    }

    fn merge(&mut self) -> Result<Done<Summary>, Error> {
        self.check_err("merge")?;
        Ok(Done {
            value: Summary { links: 0, hubs: 0 },
            queue_warning: None,
            worker_warning: None,
        })
    }

    fn failed(&mut self) -> Result<Vec<Entry>, Error> {
        self.check_err("failed")?;
        Ok(self.failed.clone())
    }

    fn retry(&mut self, id: &str) -> Result<Done<()>, Error> {
        self.check_err("retry")?;
        self.failed.retain(|e| e.id != id);
        Ok(Done { value: (), queue_warning: None, worker_warning: None })
    }

    fn retry_all(&mut self) -> Result<Done<usize>, Error> {
        self.check_err("retry_all")?;
        let count = self.failed.len();
        self.failed.clear();
        Ok(Done { value: count, queue_warning: None, worker_warning: None })
    }

    fn discard(&mut self, id: &str) -> Result<(), Error> {
        self.check_err("discard")?;
        self.failed.retain(|e| e.id != id);
        Ok(())
    }
}
