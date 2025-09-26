use ansi_colours::*;
use ratatui::style::Color;

pub trait ColorExt {
    fn to_rgb(self, is_fg: bool) -> (u8, u8, u8);
    fn mix(self, other: Color, t: f32, is_fg: bool) -> Color;
}

impl ColorExt for Color {
    fn to_rgb(self, is_fg: bool) -> (u8, u8, u8) {
        match self {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Black => rgb_from_ansi256(if is_fg { 30 } else { 40 }),
            Color::Red => rgb_from_ansi256(if is_fg { 31 } else { 41 }),
            Color::Green => rgb_from_ansi256(if is_fg { 32 } else { 42 }),
            Color::Yellow => rgb_from_ansi256(if is_fg { 33 } else { 43 }),
            Color::Blue => rgb_from_ansi256(if is_fg { 34 } else { 44 }),
            Color::Magenta => rgb_from_ansi256(if is_fg { 35 } else { 45 }),
            Color::Cyan => rgb_from_ansi256(if is_fg { 36 } else { 46 }),
            Color::Gray => rgb_from_ansi256(if is_fg { 37 } else { 47 }),
            Color::DarkGray => rgb_from_ansi256(if is_fg { 90 } else { 100 }),
            Color::LightRed => rgb_from_ansi256(if is_fg { 91 } else { 101 }),
            Color::LightGreen => rgb_from_ansi256(if is_fg { 92 } else { 102 }),
            Color::LightYellow => rgb_from_ansi256(if is_fg { 93 } else { 103 }),
            Color::LightBlue => rgb_from_ansi256(if is_fg { 94 } else { 104 }),
            Color::LightMagenta => rgb_from_ansi256(if is_fg { 95 } else { 105 }),
            Color::LightCyan => rgb_from_ansi256(if is_fg { 96 } else { 106 }),
            Color::White => rgb_from_ansi256(if is_fg { 97 } else { 107 }),
            Color::Reset => (0, 0, 0),
            Color::Indexed(i) => rgb_from_ansi256(i),
        }
    }

    fn mix(self, other: Color, t: f32, is_fg: bool) -> Color {
        match (self, other) {
            (Color::Reset, Color::Reset) => return Color::Reset,
            (Color::Reset, c) => return c,
            (c, Color::Reset) => return c,
            _ => {}
        }
        let t = t.clamp(0.0, 1.0);

        let (r1, g1, b1) = self.to_rgb(is_fg);
        let (r2, g2, b2) = other.to_rgb(is_fg);

        Color::Rgb(
            (r1 as f32 + (r2 as f32 - r1 as f32) * t).round() as u8,
            (g1 as f32 + (g2 as f32 - g1 as f32) * t).round() as u8,
            (b1 as f32 + (b2 as f32 - b1 as f32) * t).round() as u8,
        )
    }
}
