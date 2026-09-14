//! Small formatting helpers: colors, dates, truncation, fuzzy matching and a
//! lightweight markdown-to-ratatui renderer.

use chrono::{DateTime, Local, TimeZone, Utc};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const ACCENT: Color = Color::Rgb(122, 162, 247);
pub const DIM: Color = Color::Rgb(110, 115, 130);
pub const MUTED: Color = Color::Rgb(160, 165, 180);
pub const OK: Color = Color::Rgb(120, 200, 140);
pub const WARN: Color = Color::Rgb(240, 190, 80);
pub const ERR: Color = Color::Rgb(240, 100, 110);

pub fn hex_color(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    let s: String = if s.len() == 3 {
        s.chars().flat_map(|c| [c, c]).collect()
    } else {
        s.to_string()
    };
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    // Pure white/black backgrounds from ClickUp are meaningless as fg colors.
    if (r, g, b) == (255, 255, 255) || (r, g, b) == (0, 0, 0) {
        return None;
    }
    Some(Color::Rgb(r, g, b))
}

/// Color for a status name when the workspace didn't tell us one.
pub fn fallback_status_color(status: &str, kind: Option<&str>) -> Color {
    let s = status.to_ascii_lowercase();
    match kind {
        Some("done") | Some("closed") => return OK,
        Some("open") => return DIM,
        _ => {}
    }
    if s.contains("done") || s.contains("closed") || s.contains("complete") || s.contains("ready") {
        OK
    } else if s.contains("progress") || s.contains("doing") || s.contains("active") {
        Color::Rgb(16, 144, 224)
    } else if s.contains("review") || s.contains("qa") || s.contains("test") {
        Color::Rgb(95, 85, 238)
    } else if s.contains("block") || s.contains("hold") {
        ERR
    } else {
        DIM
    }
}

pub fn is_done_status(status: &str) -> bool {
    let s = status.to_ascii_lowercase();
    s.contains("done") || s.contains("closed") || s.contains("complete") || s == "ready"
}

pub fn priority_color(p: &str) -> Color {
    match p.to_ascii_lowercase().as_str() {
        "urgent" | "1" => ERR,
        "high" | "2" => WARN,
        "normal" | "3" => Color::Rgb(111, 221, 221),
        "low" | "4" => DIM,
        _ => DIM,
    }
}

pub fn priority_glyph(p: &str) -> &'static str {
    match p.to_ascii_lowercase().as_str() {
        "urgent" | "1" => "urgent",
        "high" | "2" => "high",
        "normal" | "3" => "normal",
        "low" | "4" => "low",
        _ => "",
    }
}

pub fn priority_rank(p: &str) -> u8 {
    match p.to_ascii_lowercase().as_str() {
        "urgent" | "1" => 0,
        "high" | "2" => 1,
        "normal" | "3" => 2,
        "low" | "4" => 3,
        _ => 4,
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

pub fn ms_to_local(ms: i64) -> Option<DateTime<Local>> {
    Utc.timestamp_millis_opt(ms).single().map(|d| d.with_timezone(&Local))
}

pub fn fmt_ms_date(ms: i64) -> String {
    ms_to_local(ms).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// Accepts an epoch-ms string (what cup emits) and formats it as a date.
pub fn fmt_date(s: &Option<String>) -> String {
    match s {
        Some(v) if !v.is_empty() => match v.parse::<i64>() {
            Ok(ms) => fmt_ms_date(ms),
            Err(_) => v.clone(),
        },
        _ => String::new(),
    }
}

pub fn fmt_datetime(s: &Option<String>) -> String {
    match s {
        Some(v) if !v.is_empty() => match v.parse::<i64>() {
            Ok(ms) => ms_to_local(ms)
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default(),
            Err(_) => v.clone(),
        },
        _ => String::new(),
    }
}

pub fn relative(s: &Option<String>) -> String {
    let Some(v) = s else { return String::new() };
    let Ok(ms) = v.parse::<i64>() else { return v.clone() };
    let then = Utc.timestamp_millis_opt(ms).single();
    let Some(then) = then else { return String::new() };
    let secs = (Utc::now() - then).num_seconds();
    let (n, unit) = if secs < 60 {
        return "just now".into();
    } else if secs < 3600 {
        (secs / 60, "m")
    } else if secs < 86_400 {
        (secs / 3600, "h")
    } else if secs < 86_400 * 30 {
        (secs / 86_400, "d")
    } else if secs < 86_400 * 365 {
        (secs / (86_400 * 30), "mo")
    } else {
        (secs / (86_400 * 365), "y")
    };
    format!("{n}{unit} ago")
}

pub fn is_past_ms(ms: i64) -> bool {
    Utc::now().timestamp_millis() > ms
}

pub fn fmt_duration_ms(v: &Value) -> String {
    let Some(ms) = crate::model::value_i64(v) else { return String::new() };
    if ms <= 0 {
        return String::new();
    }
    let mins = ms / 60_000;
    let h = mins / 60;
    let m = mins % 60;
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// Case-insensitive fuzzy match. Lower score is better; `None` = no match.
pub fn fuzzy_score(hay: &str, needle: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let h = hay.to_lowercase();
    let n = needle.to_lowercase();
    if let Some(pos) = h.find(&n) {
        return Some(pos as i64);
    }
    // subsequence match; penalize gaps
    let mut score = 1000i64;
    let mut hi = h.chars();
    let mut last = 0usize;
    for nc in n.chars() {
        let mut found = false;
        let mut idx = last;
        for hc in hi.by_ref() {
            idx += 1;
            if hc == nc {
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
        score += (idx - last) as i64;
        last = idx;
    }
    Some(score)
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Very small markdown renderer: headings, bullets, quotes, fenced code and
/// inline `**bold**` / `` `code` ``. Good enough for task descriptions.
pub fn markdown_lines(src: &str) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_code = false;
    let code_style = Style::default().fg(Color::Rgb(200, 200, 210)).bg(Color::Rgb(38, 40, 48));
    for raw in src.lines() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            let lang = trimmed.trim_start_matches('`').trim();
            if in_code && !lang.is_empty() {
                out.push(Line::from(Span::styled(format!("  {lang}"), Style::default().fg(DIM))));
            }
            continue;
        }
        if in_code {
            out.push(Line::from(Span::styled(format!("  {line}"), code_style)));
            continue;
        }
        if trimmed.is_empty() {
            out.push(Line::default());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            let level = 1 + rest.chars().take_while(|c| *c == '#').count();
            let title = rest.trim_start_matches('#').trim();
            let style = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
            let prefix = if level <= 1 { "" } else { "" };
            out.push(Line::from(Span::styled(format!("{prefix}{title}"), style)));
            continue;
        }
        let indent = line.len() - trimmed.len();
        let indent_str = " ".repeat(indent);
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let (marker, body) = if let Some(b) = rest.strip_prefix("[ ] ") {
                ("☐ ", b)
            } else if let Some(b) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) {
                ("☑ ", b)
            } else {
                ("• ", rest)
            };
            let mut spans = vec![Span::styled(format!("{indent_str}{marker}"), Style::default().fg(ACCENT))];
            spans.extend(inline_spans(body, Style::default()));
            out.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ") {
            let mut spans = vec![Span::styled(format!("{indent_str}│ "), Style::default().fg(DIM))];
            spans.extend(inline_spans(rest, Style::default().fg(MUTED).add_modifier(Modifier::ITALIC)));
            out.push(Line::from(spans));
            continue;
        }
        if trimmed.starts_with("---") || trimmed.starts_with("***") {
            out.push(Line::from(Span::styled("─".repeat(40), Style::default().fg(DIM))));
            continue;
        }
        let mut spans = Vec::new();
        if !indent_str.is_empty() {
            spans.push(Span::raw(indent_str));
        }
        spans.extend(inline_spans(trimmed, Style::default()));
        out.push(Line::from(spans));
    }
    out
}

fn inline_spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut code = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let flush = |buf: &mut String, spans: &mut Vec<Span<'static>>, bold: bool, code: bool| {
        if buf.is_empty() {
            return;
        }
        let mut st = base;
        if bold {
            st = st.add_modifier(Modifier::BOLD);
        }
        if code {
            st = st.fg(Color::Rgb(230, 180, 120));
        }
        spans.push(Span::styled(std::mem::take(buf), st));
    };
    while i < chars.len() {
        let c = chars[i];
        if !code && c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            flush(&mut buf, &mut spans, bold, code);
            bold = !bold;
            i += 2;
            continue;
        }
        if c == '`' {
            flush(&mut buf, &mut spans, bold, code);
            code = !code;
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut spans, bold, code);
    spans
}
