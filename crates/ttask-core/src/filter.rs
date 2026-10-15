//! The filter query language used by `ttask ls`, the TUI `/` filter and saved
//! filters, e.g. `project:work and (due:<=today or !>=2) and not +someday`.
//!
//! Terms: `+tag`, `#project`, `!>=2`, `field:value`, bare words (full-text
//! search), `"quoted phrases"`. Combine with `and` (implicit), `or`, `not`/`-`
//! and parentheses. Dates accept every phrase `due:` does in quick-add.

use jiff::civil::Date;

use crate::date::parse_date;
use crate::task::{Priority, Status};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("filter: {message}")]
pub struct FilterError {
    pub message: String,
}

fn ferr<T>(message: impl Into<String>) -> Result<T, FilterError> {
    Err(FilterError {
        message: message.into(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    pub fn sql(self) -> &'static str {
        match self {
            Cmp::Eq => "=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pred {
    /// A project name or `parent/child` path; includes subprojects.
    Project(String),
    NoProject,
    Tag(String),
    NoTags,
    Status(Status),
    AnyStatus,
    Priority(Cmp, u8),
    Due(Cmp, Date),
    DueNone,
    DueAny,
    Sched(Cmp, Date),
    SchedNone,
    Created(Cmp, Date),
    Done(Cmp, Date),
    Text(String),
    HasNotes,
    IsRecurring,
    IsBlocked,
    IsSubtask,
    HasSubtasks,
    IsOverdue,
    Id(i64),
    Wait(Cmp, Date),
    WaitNone,
    /// Include waiting and someday tasks.
    WaitAny,
    /// `wait:` date after today.
    IsWaiting,
    IsSomeday,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    All,
    Pred(Pred),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

impl Expr {
    /// Whether any predicate mentions status, so callers know not to add
    /// their default `status:open`.
    pub fn constrains_status(&self) -> bool {
        match self {
            Expr::All => false,
            Expr::Pred(p) => matches!(
                p,
                Pred::Status(_) | Pred::AnyStatus | Pred::Done(..) | Pred::IsOverdue | Pred::Id(_)
            ),
            Expr::Not(e) => e.constrains_status(),
            Expr::And(es) | Expr::Or(es) => es.iter().any(Expr::constrains_status),
        }
    }

    /// Whether any predicate mentions waiting or someday, so callers know
    /// not to hide those tasks by default.
    pub fn constrains_visibility(&self) -> bool {
        match self {
            Expr::All => false,
            Expr::Pred(p) => matches!(
                p,
                Pred::Wait(..)
                    | Pred::WaitNone
                    | Pred::WaitAny
                    | Pred::IsWaiting
                    | Pred::IsSomeday
                    | Pred::Id(_)
            ),
            Expr::Not(e) => e.constrains_visibility(),
            Expr::And(es) | Expr::Or(es) => es.iter().any(Expr::constrains_visibility),
        }
    }

    pub fn and(self, other: Expr) -> Expr {
        match (self, other) {
            (Expr::All, e) | (e, Expr::All) => e,
            (Expr::And(mut a), Expr::And(b)) => {
                a.extend(b);
                Expr::And(a)
            }
            (Expr::And(mut a), e) => {
                a.push(e);
                Expr::And(a)
            }
            (a, b) => Expr::And(vec![a, b]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    LParen,
    RParen,
    Word(String),
    Quoted(String),
}

fn lex(input: &str) -> Result<Vec<Tok>, FilterError> {
    let mut toks = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' => {
                chars.next();
                toks.push(Tok::LParen);
            }
            ')' => {
                chars.next();
                toks.push(Tok::RParen);
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some(c) => s.push(c),
                        None => return ferr("unclosed quote"),
                    }
                }
                toks.push(Tok::Quoted(s));
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '(' || c == ')' {
                        break;
                    }
                    // `text:"two words"` keeps its quoted value in one word.
                    if c == '"' {
                        chars.next();
                        for c in chars.by_ref() {
                            if c == '"' {
                                break;
                            }
                            s.push(c);
                        }
                        continue;
                    }
                    s.push(c);
                    chars.next();
                }
                toks.push(Tok::Word(s));
            }
        }
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    today: Date,
}

/// Parses a filter. `today` resolves relative dates such as `due:<=tom`.
pub fn parse_filter(input: &str, today: Date) -> Result<Expr, FilterError> {
    let toks = lex(input)?;
    if toks.is_empty() {
        return Ok(Expr::All);
    }
    let mut p = Parser {
        toks,
        pos: 0,
        today,
    };
    let expr = p.or_expr()?;
    if p.pos < p.toks.len() {
        return ferr(format!("unexpected `{}`", p.describe(p.pos)));
    }
    Ok(expr)
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn describe(&self, pos: usize) -> String {
        match &self.toks[pos] {
            Tok::LParen => "(".into(),
            Tok::RParen => ")".into(),
            Tok::Word(w) => w.clone(),
            Tok::Quoted(q) => format!("\"{q}\""),
        }
    }

    fn is_keyword(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(kw))
    }

    fn or_expr(&mut self) -> Result<Expr, FilterError> {
        let mut terms = vec![self.and_expr()?];
        while self.is_keyword("or") || matches!(self.peek(), Some(Tok::Word(w)) if w == "||") {
            self.pos += 1;
            terms.push(self.and_expr()?);
        }
        Ok(if terms.len() == 1 {
            terms.pop().unwrap()
        } else {
            Expr::Or(terms)
        })
    }

    fn and_expr(&mut self) -> Result<Expr, FilterError> {
        let mut terms = vec![self.not_expr()?];
        loop {
            if self.is_keyword("and") || matches!(self.peek(), Some(Tok::Word(w)) if w == "&&") {
                self.pos += 1;
                terms.push(self.not_expr()?);
                continue;
            }
            match self.peek() {
                None | Some(Tok::RParen) => break,
                Some(_) if self.is_keyword("or") => break,
                Some(Tok::Word(w)) if w == "||" => break,
                Some(_) => terms.push(self.not_expr()?),
            }
        }
        Ok(if terms.len() == 1 {
            terms.pop().unwrap()
        } else {
            Expr::And(terms)
        })
    }

    fn not_expr(&mut self) -> Result<Expr, FilterError> {
        if self.is_keyword("not") {
            self.pos += 1;
            return Ok(Expr::Not(Box::new(self.not_expr()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, FilterError> {
        let Some(tok) = self.peek().cloned() else {
            return ferr("expected a term at the end");
        };
        self.pos += 1;
        match tok {
            Tok::LParen => {
                let inner = self.or_expr()?;
                match self.peek() {
                    Some(Tok::RParen) => {
                        self.pos += 1;
                        Ok(inner)
                    }
                    _ => ferr("missing `)`"),
                }
            }
            Tok::RParen => ferr("unexpected `)`"),
            Tok::Quoted(text) => Ok(Expr::Pred(Pred::Text(text))),
            Tok::Word(word) => {
                for kw in ["and", "or", "&&", "||"] {
                    if word.eq_ignore_ascii_case(kw) {
                        return ferr(format!("`{word}` needs a term before it"));
                    }
                }
                if let Some(rest) = word.strip_prefix('-').filter(|r| !r.is_empty()) {
                    if rest.starts_with(['+', '#', '!']) || rest.contains(':') {
                        return Ok(Expr::Not(Box::new(Expr::Pred(self.term(rest)?))));
                    }
                }
                Ok(Expr::Pred(self.term(&word)?))
            }
        }
    }

    fn term(&self, word: &str) -> Result<Pred, FilterError> {
        if let Some(tag) = word.strip_prefix('+').filter(|t| !t.is_empty()) {
            return Ok(Pred::Tag(tag.to_owned()));
        }
        if let Some(project) = word.strip_prefix('#').filter(|t| !t.is_empty()) {
            return Ok(project_pred(project));
        }
        if let Some(priority) = word.strip_prefix('!').filter(|t| !t.is_empty()) {
            return self.priority(priority);
        }
        let Some((key, value)) = word.split_once(':') else {
            return Ok(Pred::Text(word.to_owned()));
        };
        if value.is_empty() {
            return ferr(format!("`{key}:` needs a value"));
        }
        let lower = value.to_ascii_lowercase();
        match key.to_ascii_lowercase().as_str() {
            "project" | "proj" | "p" => Ok(project_pred(value)),
            "tag" | "t" => Ok(if lower == "none" {
                Pred::NoTags
            } else {
                Pred::Tag(value.to_owned())
            }),
            "status" | "s" => match lower.as_str() {
                "all" | "any" => Ok(Pred::AnyStatus),
                other => other.parse::<Status>().map(Pred::Status).or_else(|_| {
                    ferr(format!(
                        "status is open, done, cancelled or all, not `{value}`"
                    ))
                }),
            },
            "priority" | "pri" => self.priority(value),
            "due" => match lower.as_str() {
                "none" => Ok(Pred::DueNone),
                "any" => Ok(Pred::DueAny),
                _ => self.date_cmp(value).map(|(c, d)| Pred::Due(c, d)),
            },
            "sched" | "scheduled" | "start" => match lower.as_str() {
                "none" => Ok(Pred::SchedNone),
                _ => self.date_cmp(value).map(|(c, d)| Pred::Sched(c, d)),
            },
            "wait" => match lower.as_str() {
                "none" => Ok(Pred::WaitNone),
                "any" | "all" => Ok(Pred::WaitAny),
                "someday" => Ok(Pred::IsSomeday),
                _ => self.date_cmp(value).map(|(c, d)| Pred::Wait(c, d)),
            },
            "created" => self.date_cmp(value).map(|(c, d)| Pred::Created(c, d)),
            "done" | "completed" => self.date_cmp(value).map(|(c, d)| Pred::Done(c, d)),
            "text" => Ok(Pred::Text(value.to_owned())),
            "has" => match lower.as_str() {
                "notes" => Ok(Pred::HasNotes),
                "due" => Ok(Pred::DueAny),
                "subtasks" => Ok(Pred::HasSubtasks),
                _ => ferr(format!("has: is notes, due or subtasks, not `{value}`")),
            },
            "is" => match lower.as_str() {
                "recurring" => Ok(Pred::IsRecurring),
                "blocked" => Ok(Pred::IsBlocked),
                "subtask" => Ok(Pred::IsSubtask),
                "overdue" => Ok(Pred::IsOverdue),
                "waiting" => Ok(Pred::IsWaiting),
                "someday" => Ok(Pred::IsSomeday),
                "open" => Ok(Pred::Status(Status::Open)),
                "done" => Ok(Pred::Status(Status::Done)),
                _ => ferr(format!(
                    "is: is recurring, blocked, subtask, overdue, waiting, someday, open or done, not `{value}`"
                )),
            },
            "id" => value
                .parse()
                .map(Pred::Id)
                .or_else(|_| ferr(format!("id needs a number, not `{value}`"))),
            other => ferr(format!(
                "unknown field `{other}` (quote text containing `:`)"
            )),
        }
    }

    fn priority(&self, value: &str) -> Result<Pred, FilterError> {
        let (cmp, rest) = split_cmp(value);
        let level = Priority::from_name(&rest.to_ascii_lowercase())
            .or_else(|| rest.parse::<u8>().ok().and_then(Priority::new));
        match level {
            Some(p) => Ok(Pred::Priority(cmp, p.value())),
            None => ferr(format!(
                "priority is 0-3 or low/med/high/urgent, not `{value}`"
            )),
        }
    }

    fn date_cmp(&self, value: &str) -> Result<(Cmp, Date), FilterError> {
        let (cmp, rest) = split_cmp(value);
        let spec = parse_date(rest, self.today).map_err(|e| FilterError {
            message: e.to_string(),
        })?;
        Ok((cmp, spec.date))
    }
}

fn project_pred(value: &str) -> Pred {
    if matches!(value.to_ascii_lowercase().as_str(), "none" | "inbox") {
        Pred::NoProject
    } else {
        Pred::Project(value.to_owned())
    }
}

fn split_cmp(value: &str) -> (Cmp, &str) {
    for (prefix, cmp) in [
        ("<=", Cmp::Le),
        (">=", Cmp::Ge),
        ("<", Cmp::Lt),
        (">", Cmp::Gt),
        ("=", Cmp::Eq),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return (cmp, rest);
        }
    }
    (Cmp::Eq, value)
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    const TODAY: Date = date(2026, 10, 15);

    fn parse(s: &str) -> Expr {
        parse_filter(s, TODAY).unwrap()
    }

    fn pred(p: Pred) -> Expr {
        Expr::Pred(p)
    }

    #[test]
    fn design_example() {
        let e = parse("project:work and (due:<=today or !>=2) and not +someday");
        assert_eq!(
            e,
            Expr::And(vec![
                pred(Pred::Project("work".into())),
                Expr::Or(vec![
                    pred(Pred::Due(Cmp::Le, TODAY)),
                    pred(Pred::Priority(Cmp::Ge, 2)),
                ]),
                Expr::Not(Box::new(pred(Pred::Tag("someday".into())))),
            ])
        );
    }

    #[test]
    fn implicit_and_and_or_precedence() {
        let e = parse("+a +b or +c");
        assert_eq!(
            e,
            Expr::Or(vec![
                Expr::And(vec![
                    pred(Pred::Tag("a".into())),
                    pred(Pred::Tag("b".into()))
                ]),
                pred(Pred::Tag("c".into())),
            ])
        );
    }

    #[test]
    fn text_terms_and_quotes() {
        assert_eq!(parse("invoice"), pred(Pred::Text("invoice".into())));
        assert_eq!(parse("\"two words\""), pred(Pred::Text("two words".into())));
        assert_eq!(
            parse("text:\"two words\""),
            pred(Pred::Text("two words".into()))
        );
    }

    #[test]
    fn dash_negates() {
        assert_eq!(
            parse("-+someday"),
            Expr::Not(Box::new(pred(Pred::Tag("someday".into()))))
        );
        assert_eq!(parse("-x"), pred(Pred::Text("-x".into())));
    }

    #[test]
    fn fields() {
        assert_eq!(parse("#inbox"), pred(Pred::NoProject));
        assert_eq!(parse("due:none"), pred(Pred::DueNone));
        assert_eq!(
            parse("due:tom"),
            pred(Pred::Due(Cmp::Eq, date(2026, 10, 16)))
        );
        assert_eq!(parse("!high"), pred(Pred::Priority(Cmp::Eq, 2)));
        assert_eq!(parse("status:all"), pred(Pred::AnyStatus));
        assert_eq!(parse("is:overdue"), pred(Pred::IsOverdue));
        assert_eq!(
            parse("done:>=-7d"),
            pred(Pred::Done(Cmp::Ge, date(2026, 10, 8)))
        );
        assert_eq!(parse(""), Expr::All);
    }

    #[test]
    fn waiting_terms() {
        assert_eq!(parse("is:waiting"), pred(Pred::IsWaiting));
        assert_eq!(parse("wait:someday"), pred(Pred::IsSomeday));
        assert_eq!(
            parse("wait:<=fri"),
            pred(Pred::Wait(Cmp::Le, date(2026, 10, 16)))
        );
        assert!(parse("is:someday or +x").constrains_visibility());
        assert!(!parse("+x").constrains_visibility());
    }

    #[test]
    fn status_detection() {
        assert!(!parse("+a or #b").constrains_status());
        assert!(parse("+a or status:done").constrains_status());
        assert!(parse("not is:overdue").constrains_status());
    }

    #[test]
    fn errors() {
        for bad in [
            "(+a",
            "+a)",
            "and +a",
            "+a or",
            "foo:bar",
            "due:someday",
            "!9",
            "status:maybe",
            "\"open",
        ] {
            assert!(parse_filter(bad, TODAY).is_err(), "{bad} should fail");
        }
    }
}
