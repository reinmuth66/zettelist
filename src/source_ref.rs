//! `-s` に渡された値を、出典 ID かスラッグかに読み分ける。

/// 出典の指定。`Id` は正規化済みの出典 ID、`Slug` は前後の空白を除いた入力そのもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRef {
    Id(String),
    Slug(String),
}

impl SourceRef {
    /// 入力を読み分ける。判定の順序は URL (doi.org / arxiv.org を含む)、DOI、arXiv、ISBN、スラッグ。
    pub fn parse(input: &str) -> SourceRef {
        let s = input.trim();

        if has_prefix_ignore_case(s, "http://") || has_prefix_ignore_case(s, "https://") {
            if let Some(id) = parse_known_url(s) {
                return SourceRef::Id(id);
            }
            return SourceRef::Id(format!("url:{}", normalize_url(s)));
        }
        if let Some(doi) = parse_doi(s) {
            return SourceRef::Id(format!("doi:{doi}"));
        }
        if let Some(id) = parse_arxiv_id(s) {
            return SourceRef::Id(format!("arxiv:{id}"));
        }
        if let Some(isbn) = parse_isbn(s) {
            return SourceRef::Id(format!("isbn:{isbn}"));
        }
        SourceRef::Slug(s.to_string())
    }
}

fn has_prefix_ignore_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// `10.xxxx/yyyy` の形か、`doi:` 付きの DOI を小文字にして返す。
fn parse_doi(s: &str) -> Option<String> {
    let body = if has_prefix_ignore_case(s, "doi:") {
        s[4..].trim()
    } else {
        s
    };
    if body.starts_with("10.") && body.contains('/') && !body.contains(char::is_whitespace) {
        Some(body.to_lowercase())
    } else {
        None
    }
}

/// 新方式の arXiv ID (`NNNN.NNNN` か `NNNN.NNNNN`、版 `vN` は任意) から版を除いて返す。
fn parse_arxiv_id(s: &str) -> Option<String> {
    let (id, version) = match s.rfind('v') {
        Some(pos) => (&s[..pos], Some(&s[pos + 1..])),
        None => (s, None),
    };
    if let Some(v) = version {
        if v.is_empty() || !v.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    let (head, tail) = id.split_once('.')?;
    let digits = |t: &str| t.chars().all(|c| c.is_ascii_digit());
    if head.len() == 4 && digits(head) && (tail.len() == 4 || tail.len() == 5) && digits(tail) {
        Some(id.to_string())
    } else {
        None
    }
}

/// ハイフンと空白を除いた 10 桁か 13 桁の ISBN を、13 桁にそろえて返す。
/// 入力の検査数字は検証しない。
fn parse_isbn(s: &str) -> Option<String> {
    let compact: String = s
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .collect();
    match compact.len() {
        13 if compact.chars().all(|c| c.is_ascii_digit()) => Some(compact),
        // バイト長で分岐するので、多バイト文字を含む入力で split_at が panic しないよう ASCII に限る。
        10 if compact.is_ascii() => {
            let (body, last) = compact.split_at(9);
            let last_ok = last
                .chars()
                .all(|c| c.is_ascii_digit() || c == 'X' || c == 'x');
            if body.chars().all(|c| c.is_ascii_digit()) && last_ok {
                let head = format!("978{body}");
                let check = isbn13_check_digit(&head);
                Some(format!("{head}{check}"))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 先頭 12 桁から ISBN-13 の検査数字を求める。
fn isbn13_check_digit(first12: &str) -> u32 {
    let sum: u32 = first12
        .chars()
        .filter_map(|c| c.to_digit(10))
        .enumerate()
        .map(|(i, d)| if i % 2 == 0 { d } else { d * 3 })
        .sum();
    (10 - sum % 10) % 10
}

/// scheme、ホスト、パスに分ける。`#` 以降は先に落とす。
fn split_url(s: &str) -> (&str, &str, &str) {
    let s = s.split('#').next().unwrap_or(s);
    let scheme_end = s.find("://").unwrap_or(0);
    let (scheme, rest) = (&s[..scheme_end], &s[scheme_end + 3..]);
    let host_end = rest.find(['/', '?']).unwrap_or(rest.len());
    (scheme, &rest[..host_end], &rest[host_end..])
}

/// 前後の空白と `#` 以降を除き、scheme とホストを小文字にする。それ以外は変えない。
fn normalize_url(s: &str) -> String {
    let (scheme, authority, path) = split_url(s);
    // userinfo は大文字小文字を区別するので、ホスト部分だけ小文字にする。
    let authority = match authority.rfind('@') {
        Some(at) => format!(
            "{}{}",
            &authority[..=at],
            authority[at + 1..].to_lowercase()
        ),
        None => authority.to_lowercase(),
    };
    format!("{}://{}{}", scheme.to_lowercase(), authority, path)
}

/// doi.org と arxiv.org の URL を、それぞれ DOI と arXiv ID に読み替える。
fn parse_known_url(s: &str) -> Option<String> {
    let (_, authority, path) = split_url(s);
    let host = authority.to_lowercase();
    match host.as_str() {
        "doi.org" | "dx.doi.org" | "www.doi.org" => {
            parse_doi(path.trim_start_matches('/')).map(|d| format!("doi:{d}"))
        }
        "arxiv.org" | "www.arxiv.org" => {
            let rest = path
                .strip_prefix("/abs/")
                .or_else(|| path.strip_prefix("/pdf/"))?;
            let rest = rest.split('?').next().unwrap_or(rest);
            let rest = rest.strip_suffix(".pdf").unwrap_or(rest);
            parse_arxiv_id(rest).map(|id| format!("arxiv:{id}"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::SourceRef;
    use super::SourceRef::*;

    #[test]
    fn isbn13_with_hyphens() {
        assert_eq!(
            SourceRef::parse("978-4-480-06899-8"),
            Id("isbn:9784480068998".into())
        );
    }
    #[test]
    fn isbn10_becomes_13() {
        assert_eq!(
            SourceRef::parse("0-306-40615-2"),
            Id("isbn:9780306406157".into())
        );
    }
    #[test]
    fn doi_is_lowercased() {
        assert_eq!(
            SourceRef::parse("10.1145/ABC.123"),
            Id("doi:10.1145/abc.123".into())
        );
    }
    #[test]
    fn doi_org_url_becomes_doi() {
        assert_eq!(
            SourceRef::parse("https://doi.org/10.1145/ABC"),
            Id("doi:10.1145/abc".into())
        );
    }
    #[test]
    fn arxiv_drops_version() {
        assert_eq!(
            SourceRef::parse("2401.01234v2"),
            Id("arxiv:2401.01234".into())
        );
    }
    #[test]
    fn arxiv_url_becomes_arxiv() {
        assert_eq!(
            SourceRef::parse("https://arxiv.org/abs/2401.01234v3"),
            Id("arxiv:2401.01234".into())
        );
    }
    #[test]
    fn url_drops_fragment_and_lowercases_host() {
        assert_eq!(
            SourceRef::parse("https://Example.COM/a?b=1#c"),
            Id("url:https://example.com/a?b=1".into())
        );
    }
    #[test]
    fn other_text_is_slug() {
        assert_eq!(SourceRef::parse("ahrens2017"), Slug("ahrens2017".into()));
    }
    #[test]
    fn surrounding_spaces_are_trimmed() {
        assert_eq!(SourceRef::parse("  ahrens2017 "), Slug("ahrens2017".into()));
    }
    #[test]
    fn multibyte_slug_of_ten_bytes_does_not_panic() {
        assert_eq!(SourceRef::parse("a日本語"), Slug("a日本語".into()));
        assert_eq!(SourceRef::parse("1日本語"), Slug("1日本語".into()));
        assert_eq!(SourceRef::parse("abcd日本"), Slug("abcd日本".into()));
    }
}
