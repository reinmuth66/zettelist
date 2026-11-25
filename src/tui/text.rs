use unicode_width::UnicodeWidthChar;

/// 表示幅が `width` を超えたら、`…` を含めて `width` 以内に切る。
/// 全角文字を半分で切らない。
pub fn truncate(s: &str, width: usize) -> String {
    let total_w: usize = s.chars().map(|c| c.width().unwrap_or(0)).sum();
    if total_w <= width {
        return s.to_string();
    }

    let ellipsis_w = '…'.width().unwrap_or(1);
    if width < ellipsis_w {
        return String::new();
    }

    let limit = width - ellipsis_w;
    let mut out = String::new();
    let mut acc = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if acc + cw > limit {
            break;
        }
        out.push(c);
        acc += cw;
    }
    out.push('…');
    out
}

/// ノートの slug (`YYYYMMDDHHMMSS`) から `MMDD` を返す。14 桁でなければそのまま。
pub fn short_date(slug: &str) -> &str {
    if slug.len() == 14 && slug.bytes().all(|b| b.is_ascii_digit()) {
        &slug[4..8]
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_ascii() {
        assert_eq!(truncate("abcdef", 4), "abc…");
    }

    #[test]
    fn truncate_does_not_split_wide_char() {
        assert_eq!(truncate("あいう", 5), "あい…");
        assert_eq!(truncate("あいう", 4), "あ…");
    }

    #[test]
    fn truncate_keeps_short_text() {
        assert_eq!(truncate("あい", 4), "あい");
    }

    #[test]
    fn short_date_from_slug() {
        assert_eq!(short_date("20261118093000"), "1118");
        assert_eq!(short_date("x"), "x");
    }
}
