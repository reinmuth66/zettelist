use ratatui::style::Color;

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
