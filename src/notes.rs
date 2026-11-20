//! ノートの種類と、書き込み前のノートの形。

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum NoteKind {
    Fleeting,
    Literature,
    Permanent,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NewNote {
    pub body: String,
    pub source: Option<String>,
    pub locator: Option<String>,
    pub created: chrono::DateTime<chrono::Local>,
}
