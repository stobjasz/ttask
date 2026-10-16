//! Renders task notes as light Markdown for the detail pane: headings,
//! lists and task lists, quotes, code, rules, emphasis and links. Anything
//! else shows as written, so plain-text notes look the same as before.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

pub fn render(text: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let g = theme.glyphs;
    let code = theme.dim.add_modifier(Modifier::ITALIC);
    let mut lines = Vec::new();
    let mut in_code = false;
    for raw in text.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(Line::from(Span::styled(format!("  {raw}"), code)));
            continue;
        }
        let indent = &raw[..raw.len() - trimmed.len()];
        if let Some((level, heading)) = heading(trimmed) {
            let style = theme.accent.add_modifier(Modifier::BOLD);
            let style = if level > 1 {
                style
            } else {
                style.add_modifier(Modifier::UNDERLINED)
            };
            lines.push(Line::from(inline(heading, style, theme)));
            continue;
        }
        if is_rule(trimmed) {
            lines.push(Line::from(Span::styled(
                g.rule.repeat(width.clamp(3, 40)),
                theme.dim,
            )));
            continue;
        }
        if let Some(quote) = trimmed.strip_prefix('>') {
            let mut spans = vec![Span::styled(format!("{indent}{} ", g.quote), theme.dim)];
            spans.extend(inline(
                quote.trim_start(),
                theme.base.add_modifier(Modifier::ITALIC),
                theme,
            ));
            lines.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = ["- ", "* ", "+ "]
            .iter()
            .find_map(|m| trimmed.strip_prefix(m))
        {
            let (marker, rest, style) = match task_box(rest) {
                Some((true, rest)) => (g.box_done, rest, theme.done),
                Some((false, rest)) => (g.box_open, rest, theme.base),
                None => (g.bullet, rest, theme.base),
            };
            let mut spans = vec![Span::styled(format!("{indent}{marker} "), theme.accent)];
            spans.extend(inline(rest, style, theme));
            lines.push(Line::from(spans));
            continue;
        }
        let mut spans = vec![Span::raw(indent.to_owned())];
        spans.extend(inline(trimmed, theme.base, theme));
        lines.push(Line::from(spans));
    }
    lines
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    let rest = line[level..].strip_prefix(' ')?;
    (1..=6).contains(&level).then_some((level, rest.trim()))
}

fn is_rule(line: &str) -> bool {
    let line: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    line.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|m| line.chars().all(|c| c == *m))
}

/// `[ ] rest` / `[x] rest` after a list marker.
fn task_box(rest: &str) -> Option<(bool, &str)> {
    let done = match rest.get(..3)? {
        "[ ]" => false,
        "[x]" | "[X]" => true,
        _ => return None,
    };
    Some((done, rest[3..].trim_start()))
}

/// Emphasis, `code` and links within one line.
fn inline(text: &str, base: Style, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = text.chars().collect();
    let flush = |plain: &mut String, spans: &mut Vec<Span<'static>>| {
        if !plain.is_empty() {
            spans.push(Span::styled(std::mem::take(plain), base));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
        // `code`
        if c == '`' {
            if let Some(end) = find(&chars, i + 1, "`") {
                flush(&mut plain, &mut spans);
                let code: String = chars[i + 1..end].iter().collect();
                spans.push(Span::styled(code, theme.tag));
                i = end + 1;
                continue;
            }
        }
        // **bold** / __bold__
        if (c == '*' || c == '_') && chars.get(i + 1) == Some(&c) && !prev_alnum {
            let marker: String = [c, c].iter().collect();
            if let Some(end) = find(&chars, i + 2, &marker).filter(|&e| e > i + 2) {
                flush(&mut plain, &mut spans);
                let inner: String = chars[i + 2..end].iter().collect();
                spans.extend(inline(&inner, base.add_modifier(Modifier::BOLD), theme));
                i = end + 2;
                continue;
            }
        }
        // *italic* / _italic_ (not inside snake_case words)
        if (c == '*' || c == '_')
            && !prev_alnum
            && chars.get(i + 1).is_some_and(|n| !n.is_whitespace())
        {
            if let Some(end) = find(&chars, i + 1, &c.to_string())
                .filter(|&e| e > i + 1 && chars.get(e + 1).is_none_or(|n| !n.is_alphanumeric()))
            {
                flush(&mut plain, &mut spans);
                let inner: String = chars[i + 1..end].iter().collect();
                spans.extend(inline(&inner, base.add_modifier(Modifier::ITALIC), theme));
                i = end + 1;
                continue;
            }
        }
        // [text](url)
        if c == '[' {
            if let Some(close) = find(&chars, i + 1, "](") {
                if let Some(end) = find(&chars, close + 2, ")") {
                    flush(&mut plain, &mut spans);
                    let label: String = chars[i + 1..close].iter().collect();
                    let url: String = chars[close + 2..end].iter().collect();
                    spans.push(Span::styled(label, base.add_modifier(Modifier::UNDERLINED)));
                    spans.push(Span::styled(format!(" ({url})"), theme.dim));
                    i = end + 1;
                    continue;
                }
            }
        }
        // Bare links.
        if !prev_alnum && starts_with(&chars, i, "http") {
            let rest: String = chars[i..].iter().collect();
            if let Some(link) = ttask_core::links::find_links(&rest)
                .into_iter()
                .next()
                .filter(|l| rest.starts_with(l.as_str()))
            {
                flush(&mut plain, &mut spans);
                i += link.chars().count();
                spans.push(Span::styled(link, base.add_modifier(Modifier::UNDERLINED)));
                continue;
            }
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut plain, &mut spans);
    spans
}

fn starts_with(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, p)| {
        chars
            .get(at + k)
            .is_some_and(|c| c.eq_ignore_ascii_case(&p))
    })
}

/// Index of the next `pat` at or after `from`.
fn find(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    (from..chars.len()).find(|&i| starts_with(chars, i, pat))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeName;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn styled<'a>(line: &'a Line, text: &str) -> &'a Span<'a> {
        line.spans
            .iter()
            .find(|s| s.content == text)
            .unwrap_or_else(|| panic!("no span `{text}` in {line:?}"))
    }

    #[test]
    fn blocks() {
        let theme = Theme::new(ThemeName::Dark, false);
        let text = "# Plan\n- [ ] buy paint\n- [x] sand\n* one\n  - nested\n> quoted\n---\n```\nlet x = 1;\n```\n1. first";
        let lines = render(text, &theme, 10);
        assert_eq!(
            plain(&lines),
            [
                "Plan",
                "☐ buy paint",
                "☑ sand",
                "• one",
                "  • nested",
                "│ quoted",
                "──────────",
                "  let x = 1;",
                "1. first",
            ]
        );
        assert!(
            lines[0].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn inline_styles_links_and_snake_case() {
        let theme = Theme::new(ThemeName::Dark, false);
        let lines = render(
            "**Bold** and *it* with `code`, [docs](https://x.org) see https://a.com. my_var_name",
            &theme,
            40,
        );
        assert_eq!(
            plain(&lines),
            ["Bold and it with code, docs (https://x.org) see https://a.com. my_var_name"]
        );
        let line = &lines[0];
        assert!(
            styled(line, "Bold")
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            styled(line, "it")
                .style
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        assert!(
            styled(line, "docs")
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        assert!(
            styled(line, "https://a.com")
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        assert_eq!(styled(line, "code").style, theme.tag);
    }

    #[test]
    fn ascii_theme_glyphs() {
        let theme = Theme::new(ThemeName::Ascii, false);
        assert_eq!(plain(&render("- [ ] a\n- b", &theme, 5)), ["[ ] a", "* b"]);
    }
}
