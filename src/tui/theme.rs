use crate::error::Error;
use ratatui::style::Color;
use std::collections::BTreeMap;

/// 端末の 16 色だけで構成する配色。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub cursor: Color,
    pub selected: Color,
    pub similar: Color,
    pub old: Color,
    pub dim: Color,
    pub header: Color,
    pub error: Color,
    pub adopted: Color,
    pub rejected: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            cursor: Color::Cyan,
            selected: Color::Yellow,
            similar: Color::Magenta,
            old: Color::Red,
            dim: Color::DarkGray,
            header: Color::Blue,
            error: Color::Red,
            adopted: Color::Green,
            rejected: Color::Red,
        }
    }
}

impl Theme {
    /// 設定ファイルの `[theme]` から配色を作る。書かれていない項目は既定値を使う。
    pub fn from_config(map: &BTreeMap<String, String>) -> Result<Theme, Error> {
        let mut theme = Theme::default();
        for (item, value) in map {
            let target = match item.as_str() {
                "cursor" => &mut theme.cursor,
                "selected" => &mut theme.selected,
                "similar" => &mut theme.similar,
                "old" => &mut theme.old,
                "dim" => &mut theme.dim,
                "header" => &mut theme.header,
                "error" => &mut theme.error,
                "adopted" => &mut theme.adopted,
                "rejected" => &mut theme.rejected,
                _ => return Err(Error::Invalid(format!("[theme] の項目 '{item}' はありません"))),
            };
            *target = parse_color(item, value)?;
        }
        Ok(theme)
    }
}

fn parse_color(item: &str, name: &str) -> Result<Color, Error> {
    match name {
        "black" => Ok(Color::Black),
        "red" => Ok(Color::Red),
        "green" => Ok(Color::Green),
        "yellow" => Ok(Color::Yellow),
        "blue" => Ok(Color::Blue),
        "magenta" => Ok(Color::Magenta),
        "cyan" => Ok(Color::Cyan),
        "white" => Ok(Color::Gray),
        "bright_black" => Ok(Color::DarkGray),
        "bright_red" => Ok(Color::LightRed),
        "bright_green" => Ok(Color::LightGreen),
        "bright_yellow" => Ok(Color::LightYellow),
        "bright_blue" => Ok(Color::LightBlue),
        "bright_magenta" => Ok(Color::LightMagenta),
        "bright_cyan" => Ok(Color::LightCyan),
        "bright_white" => Ok(Color::White),
        "default" => Ok(Color::Reset),
        _ => Err(Error::Invalid(format!(
            "[theme] の {item} の色 '{name}' は使えません"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn theme_overrides_one_color() {
        let mut map = BTreeMap::new();
        map.insert("similar".to_string(), "red".to_string());
        let t = Theme::from_config(&map).unwrap();
        assert_eq!(t.similar, Color::Red);
        assert_eq!(t.old, Theme::default().old);
    }

    #[test]
    fn theme_rejects_unknown_color_and_item() {
        let mut bad_color = BTreeMap::new();
        bad_color.insert("similar".to_string(), "pink".to_string());
        let err = Theme::from_config(&bad_color).unwrap_err();
        assert!(err.to_string().contains("similar"), "err: {err}");

        let mut bad_item = BTreeMap::new();
        bad_item.insert("nosuch".to_string(), "red".to_string());
        let err = Theme::from_config(&bad_item).unwrap_err();
        assert!(err.to_string().contains("nosuch"), "err: {err}");
    }
}
