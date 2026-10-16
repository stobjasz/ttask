//! Finding links in task text, for `ttask open` and the TUI's `o`.
//!
//! Only `http`, `https` and `mailto` links count: they open in a browser or
//! mail client. `file:` and other schemes could launch programs, so they are
//! never opened.

const SCHEMES: &[&str] = &["https://", "http://", "mailto:"];

/// Links in `text`, in order of appearance, without duplicates. Trailing
/// punctuation and an unbalanced closing bracket (Markdown `[x](url)`, or a
/// link in parentheses) are not part of the link.
pub fn find_links(text: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut pos = 0;
    while pos < text.len() {
        let next = SCHEMES
            .iter()
            .filter_map(|scheme| lower[pos..].find(scheme).map(|i| (pos + i, scheme.len())))
            .min();
        let Some((start, scheme_len)) = next else {
            break;
        };
        // `xhttps://` is not a link; `(https://` and `<https://` are.
        let boundary = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
            .map_or(text.len(), |i| start + i);
        let link = trim_link(&text[start..end]);
        if boundary && link.len() > scheme_len && !links.iter().any(|l| l == link) {
            links.push(link.to_owned());
        }
        pos = end.max(start + scheme_len);
    }
    links
}

/// Whether `link` is one ttask is willing to hand to the OS opener.
pub fn is_openable(link: &str) -> bool {
    let lower = link.to_ascii_lowercase();
    SCHEMES
        .iter()
        .any(|s| lower.starts_with(s) && link.len() > s.len())
        && !link.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn trim_link(mut link: &str) -> &str {
    loop {
        let Some(last) = link.chars().next_back() else {
            return link;
        };
        let unbalanced = |open: char, close: char| {
            last == close && link.matches(close).count() > link.matches(open).count()
        };
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '*')
            || unbalanced('(', ')')
            || unbalanced('[', ']')
        {
            link = &link[..link.len() - last.len_utf8()];
        } else {
            return link;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_in_prose_and_markdown() {
        let text = "See https://example.com/a, the [spec](https://x.org/spec_(v2)) \
                    and (http://old.example.com/). Mail mailto:me@example.com!";
        assert_eq!(
            find_links(text),
            [
                "https://example.com/a",
                "https://x.org/spec_(v2)",
                "http://old.example.com/",
                "mailto:me@example.com",
            ]
        );
    }

    #[test]
    fn ignores_other_schemes_duplicates_and_bare_schemes() {
        let text = "file:///etc/passwd ftp://x https:// HTTPS://A.com https://A.com xhttps://no";
        assert_eq!(find_links(text), ["HTTPS://A.com", "https://A.com"]);
        assert!(find_links("no links here").is_empty());
    }

    #[test]
    fn openable_links() {
        assert!(is_openable("https://example.com/?q=a&b=c"));
        assert!(is_openable("mailto:me@example.com"));
        assert!(!is_openable("file:///C:/Windows/notepad.exe"));
        assert!(!is_openable("https://"));
        assert!(!is_openable("https://a.com/\nrm"));
    }
}
