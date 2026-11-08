//! cybercore CYBERGRID palette as ratatui colors (same pattern as cyberwatch).
//! `CYBERGRID_THEME=<slug>` picks another cybercore theme.

use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub white: Color,
    pub acid_green: Color,
    pub hot_pink: Color,
    pub purple: Color,
    pub cyan: Color,
    pub orange: Color,
    pub red: Color,
    pub panel: Color,
    pub line: Color,
    pub muted: Color,
}

impl Theme {
    pub fn from_cybercore() -> Self {
        let p = &cybercore::schema::load().palette;
        Self {
            bg: hex(&p.bg),
            white: hex(&p.white),
            acid_green: hex(&p.acid_green),
            hot_pink: hex(&p.hot_pink),
            purple: hex(&p.purple),
            cyan: hex(&p.cyan),
            orange: hex(&p.orange),
            red: hex(&p.red),
            panel: hex(&p.panel),
            line: hex(&p.line),
            muted: hex(&p.muted),
        }
    }
}

fn hex(s: &str) -> Color {
    let s = s.trim_start_matches('#');
    let ch = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    match (ch(0), ch(2), ch(4)) {
        (Some(r), Some(g), Some(b)) => Color::Rgb(r, g, b),
        _ => Color::Reset,
    }
}
