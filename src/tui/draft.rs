use crate::notes::{InboxItem, NoteKind};
use crate::tui::text::short_date;

const PLACEHOLDER: &str = "(ここに Permanent を書く)";
const MARKER: &str = "# ---- この行から下は保存時に消える ----";

/// 選んだ inbox ノートから、昇格用の `$EDITOR` 下書きを組み立てる。
pub fn build(items: &[InboxItem]) -> String {
    let mut lines = vec![
        PLACEHOLDER.to_string(),
        String::new(),
        MARKER.to_string(),
    ];

    for item in items {
        let date = short_date(&item.slug);
        let kind = match item.kind {
            NoteKind::Fleeting => "F",
            NoteKind::Literature => "L",
            NoteKind::Permanent => "P",
        };
        let mut meta = vec![date.to_string(), kind.to_string()];
        if let Some(src) = item.source_slug.as_deref().or(item.source_id.as_deref()) {
            if !src.is_empty() {
                meta.push(src.to_string());
            }
        }
        if let Some(loc) = item.locator.as_deref().filter(|s| !s.is_empty()) {
            meta.push(loc.to_string());
        }
        let body = item.body.replace("\r\n", " ").replace('\n', " ");
        lines.push(format!("# [{}] {}", meta.join(" "), body));
    }

    lines.join("\n")
}

/// エディタで編集された下書きから本文を取り出す。空なら `None`。
pub fn parse(text: &str) -> Option<String> {
    let body = text
        .lines()
        .take_while(|line| line.trim_end() != MARKER)
        .filter(|line| line.trim() != PLACEHOLDER)
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = body.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::NoteKind;
    use chrono::Utc;

    fn fleeting(slug: &str, body: &str) -> InboxItem {
        InboxItem {
            kind: NoteKind::Fleeting,
            slug: slug.to_string(),
            body: body.to_string(),
            created: Utc::now(),
            source_slug: None,
            source_id: None,
            locator: None,
        }
    }

    fn literature(slug: &str, body: &str, source_slug: Option<&str>, locator: Option<&str>) -> InboxItem {
        InboxItem {
            kind: NoteKind::Literature,
            slug: slug.to_string(),
            body: body.to_string(),
            created: Utc::now(),
            source_slug: source_slug.map(str::to_string),
            source_id: None,
            locator: locator.map(str::to_string),
        }
    }

    #[test]
    fn build_lists_selected_notes_as_comments() {
        let items = vec![
            fleeting("20261118090000", "a\nb"),
            literature("20261117090000", "箇条書きは考えを止める", Some("ahrens2017"), Some("p.42")),
        ];
        let text = build(&items);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "(ここに Permanent を書く)");
        assert!(lines.contains(&"# [1118 F] a b"), "{text}");
        assert!(lines.contains(&"# [1117 L ahrens2017 p.42] 箇条書きは考えを止める"), "{text}");
    }

    #[test]
    fn parse_drops_comment_section() {
        let text = "本文\n\n# ---- この行から下は保存時に消える ----\n# x";
        assert_eq!(parse(text).as_deref(), Some("本文"));
    }

    #[test]
    fn placeholder_only_cancels() {
        let items = vec![fleeting("20261118090000", "メモ")];
        let text = build(&items);
        assert_eq!(parse(&text), None);
    }

    #[test]
    fn parse_without_marker_keeps_all() {
        assert_eq!(parse("  本文のみ\n2行目  ").as_deref(), Some("本文のみ\n2行目"));
        assert_eq!(parse("   \n  "), None);
    }
}
