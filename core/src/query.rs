//! The filter query language.
//!
//! Every view in the app is this: Today is `due: today | overdue`, Pinboard is
//! `pinned`, a project is `#Work`. Writing one evaluator instead of a bespoke
//! filter object per view means the built-in views and the user's own saved
//! filters are the same code path, and a bug in one is a bug in both — which
//! is the point, because it means there is only one thing to get right.
//!
//! ```text
//! query   := or (',' or)*        a comma renders as separate lists
//! or      := and ('|' and)*
//! and     := unary ('&' unary)*
//! unary   := '!' unary | '(' query ')' | term
//! ```
//!
//! **Completed tasks are excluded unless you ask for them.** A filter that
//! never mentioned completion showing three months of ticked-off work would be
//! useless, so `completed` in the query is what turns them on. The rule is
//! applied in exactly one place, [`Query::includes_completed`], rather than
//! being remembered at each call site.

use std::fmt;

use chrono::NaiveDate;

use super::parse::date::parse_date;
use super::priority::Priority;
use super::store::Store;
use super::task::Task;

/// A parsed filter, ready to evaluate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Comma-separated filters, each rendered as its own list.
    pub lists: Vec<Filter>,
    includes_completed: bool,
}

/// A boolean expression over [`Term`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    All,
    Term(Term),
    Not(Box<Filter>),
    And(Box<Filter>, Box<Filter>),
    Or(Box<Filter>, Box<Filter>),
}

/// A single condition on a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// `due: friday`, `due before: friday`, `today`.
    Due(DateFilter),
    /// `deadline: friday`, `deadline after: monday`.
    Deadline(DateFilter),
    /// `overdue` — a due date in the past.
    Overdue,
    /// `no date`.
    NoDate,
    /// `no deadline`.
    NoDeadline,
    /// `recurring`.
    Recurring,
    /// `subtask` — has a parent.
    Subtask,
    /// `pinned`.
    Pinned,
    /// `completed`.
    Completed,
    /// `p1`–`p4`.
    Priority(Priority),
    /// `@label`.
    Label(String),
    /// `no labels`.
    NoLabels,
    /// `#Project`, or `##Project` to include its subprojects.
    Project {
        name: String,
        include_subprojects: bool,
    },
    /// `/Section`.
    Section(String),
    /// `search: text` — a case-insensitive substring of title or description.
    Search(String),
}

/// How a date term compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateFilter {
    On(DateSpec),
    Before(DateSpec),
    After(DateSpec),
}

/// The date a term compares against. Resolved at evaluation time, not at
/// parse time: a saved filter of `due: today` has to mean today whenever it
/// is run, not the day it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateSpec {
    /// A phrase [`parse_date`] understands, kept verbatim.
    Phrase(String),
}

/// Why a query would not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    /// Byte offset into the query where the problem is, for underlining it.
    pub at: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParseError {}

/// What a query is evaluated against.
pub struct Context<'a> {
    pub store: &'a Store,
    pub today: NaiveDate,
}

impl Query {
    /// Parse a filter query.
    pub fn parse(source: &str) -> Result<Self, ParseError> {
        let tokens = tokenize(source)?;
        let mut parser = Parser {
            tokens: &tokens,
            position: 0,
        };
        let lists = parser.parse_lists()?;
        parser.expect_end()?;

        let includes_completed = lists.iter().any(mentions_completed);
        Ok(Self {
            lists,
            includes_completed,
        })
    }

    /// A query matching everything.
    pub fn all() -> Self {
        Self {
            lists: vec![Filter::All],
            includes_completed: false,
        }
    }

    /// Whether this query asked to see completed tasks.
    pub fn includes_completed(&self) -> bool {
        self.includes_completed
    }

    /// Whether a task belongs in this query's results.
    ///
    /// A task matches if any of the comma-separated lists matches it. Use
    /// [`Query::lists`] directly to render them separately.
    pub fn matches(&self, task: &Task, cx: &Context<'_>) -> bool {
        if task.checked && !self.includes_completed {
            return false;
        }
        self.lists.iter().any(|list| list.matches(task, cx))
    }
}

/// Whether a completed task could possibly satisfy this filter.
fn mentions_completed(filter: &Filter) -> bool {
    match filter {
        Filter::All => false,
        Filter::Term(term) => matches!(term, Term::Completed),
        Filter::Not(inner) => mentions_completed(inner),
        Filter::And(left, right) | Filter::Or(left, right) => {
            mentions_completed(left) || mentions_completed(right)
        }
    }
}

impl Filter {
    /// Whether a task satisfies this filter, ignoring the completed rule.
    pub fn matches(&self, task: &Task, cx: &Context<'_>) -> bool {
        match self {
            Self::All => true,
            Self::Term(term) => term.matches(task, cx),
            Self::Not(inner) => !inner.matches(task, cx),
            Self::And(left, right) => left.matches(task, cx) && right.matches(task, cx),
            Self::Or(left, right) => left.matches(task, cx) || right.matches(task, cx),
        }
    }
}

impl Term {
    fn matches(&self, task: &Task, cx: &Context<'_>) -> bool {
        match self {
            Self::Due(filter) => task
                .due
                .as_ref()
                .is_some_and(|due| filter.matches(due.date, cx)),
            Self::Deadline(filter) => task
                .deadline
                .is_some_and(|deadline| filter.matches(deadline, cx)),
            Self::Overdue => task.is_overdue(cx.today),
            Self::NoDate => task.due.is_none(),
            Self::NoDeadline => task.deadline.is_none(),
            Self::Recurring => task.due.as_ref().is_some_and(|due| due.is_recurring()),
            Self::Subtask => task.is_subtask(),
            Self::Pinned => task.pinned,
            Self::Completed => task.checked,
            Self::Priority(priority) => task.priority == *priority,
            Self::NoLabels => task.labels.is_empty(),
            Self::Label(name) => cx
                .store
                .label_by_name(name)
                .is_some_and(|label| task.has_label(&label.id)),
            Self::Project {
                name,
                include_subprojects,
            } => match cx.store.project_by_name(name) {
                // An unknown project matches nothing rather than everything.
                // Renaming a project should empty a saved filter, not turn it
                // into "show me the lot".
                None => false,
                Some(project) if !include_subprojects => task.project_id == project.id,
                Some(project) => cx
                    .store
                    .project_and_descendants(&project.id)
                    .contains(&task.project_id),
            },
            Self::Section(name) => match &task.section_id {
                None => false,
                Some(section_id) => cx
                    .store
                    .section(section_id)
                    .is_some_and(|section| section.name.eq_ignore_ascii_case(name)),
            },
            Self::Search(needle) => {
                let needle = needle.to_lowercase();
                task.content.to_lowercase().contains(&needle)
                    || task.description.to_lowercase().contains(&needle)
            }
        }
    }
}

impl DateFilter {
    fn matches(&self, date: NaiveDate, cx: &Context<'_>) -> bool {
        let (spec, compare): (&DateSpec, fn(NaiveDate, NaiveDate) -> bool) = match self {
            Self::On(spec) => (spec, |a, b| a == b),
            Self::Before(spec) => (spec, |a, b| a < b),
            Self::After(spec) => (spec, |a, b| a > b),
        };
        match spec.resolve(cx.today) {
            Some(target) => compare(date, target),
            // An unparseable date matches nothing. The parser rejects these
            // up front; this is the belt to that braces.
            None => false,
        }
    }
}

impl DateSpec {
    fn resolve(&self, today: NaiveDate) -> Option<NaiveDate> {
        match self {
            Self::Phrase(phrase) => parse_date(phrase, today),
        }
    }
}

// --- tokenizer ----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Text(String, usize),
    And(usize),
    Or(usize),
    Not(usize),
    Comma(usize),
    Open(usize),
    Close(usize),
}

impl Token {
    fn at(&self) -> usize {
        match self {
            Self::Text(_, at)
            | Self::And(at)
            | Self::Or(at)
            | Self::Not(at)
            | Self::Comma(at)
            | Self::Open(at)
            | Self::Close(at) => *at,
        }
    }
}

/// Split a query into operators and runs of text.
///
/// Term text may contain spaces — `#My Project`, `due before: next friday` —
/// so a run continues until an operator character. A backslash escapes the
/// character after it, which is how a project genuinely called "R&D" is
/// written: `#R\&D`.
fn tokenize(source: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut text_start = 0;
    let mut characters = source.char_indices().peekable();

    let flush = |text: &mut String, start: usize, tokens: &mut Vec<Token>| {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            tokens.push(Token::Text(trimmed.to_string(), start));
        }
        text.clear();
    };

    while let Some((index, character)) = characters.next() {
        match character {
            '\\' => {
                if text.is_empty() {
                    text_start = index;
                }
                match characters.next() {
                    Some((_, escaped)) => text.push(escaped),
                    None => {
                        return Err(ParseError {
                            message: "the query ends with a stray backslash".into(),
                            at: index,
                        })
                    }
                }
            }
            '&' | '|' | '!' | ',' | '(' | ')' => {
                flush(&mut text, text_start, &mut tokens);
                tokens.push(match character {
                    '&' => Token::And(index),
                    '|' => Token::Or(index),
                    '!' => Token::Not(index),
                    ',' => Token::Comma(index),
                    '(' => Token::Open(index),
                    _ => Token::Close(index),
                });
            }
            _ => {
                if text.is_empty() && !character.is_whitespace() {
                    text_start = index;
                }
                text.push(character);
            }
        }
    }
    flush(&mut text, text_start, &mut tokens);

    Ok(tokens)
}

// --- parser -------------------------------------------------------------

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn end_offset(&self) -> usize {
        self.tokens.last().map_or(0, |token| token.at())
    }

    fn parse_lists(&mut self) -> Result<Vec<Filter>, ParseError> {
        let mut lists = vec![self.parse_or()?];
        while matches!(self.peek(), Some(Token::Comma(_))) {
            self.position += 1;
            lists.push(self.parse_or()?);
        }
        Ok(lists)
    }

    fn parse_or(&mut self) -> Result<Filter, ParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Or(_))) {
            self.position += 1;
            let right = self.parse_and()?;
            left = Filter::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Filter, ParseError> {
        let mut left = self.parse_unary()?;
        while matches!(self.peek(), Some(Token::And(_))) {
            self.position += 1;
            let right = self.parse_unary()?;
            left = Filter::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Filter, ParseError> {
        match self.peek().cloned() {
            Some(Token::Not(_)) => {
                self.position += 1;
                Ok(Filter::Not(Box::new(self.parse_unary()?)))
            }
            Some(Token::Open(at)) => {
                self.position += 1;
                let inner = self.parse_or()?;
                match self.peek() {
                    Some(Token::Close(_)) => {
                        self.position += 1;
                        Ok(inner)
                    }
                    _ => Err(ParseError {
                        message: "this bracket is never closed".into(),
                        at,
                    }),
                }
            }
            Some(Token::Text(text, at)) => {
                self.position += 1;
                parse_term(&text, at).map(Filter::Term)
            }
            Some(other) => Err(ParseError {
                message: "expected a condition here".into(),
                at: other.at(),
            }),
            None => Err(ParseError {
                message: "the query ends where a condition was expected".into(),
                at: self.end_offset(),
            }),
        }
    }

    fn expect_end(&self) -> Result<(), ParseError> {
        match self.peek() {
            None => Ok(()),
            Some(Token::Close(at)) => Err(ParseError {
                message: "this bracket was never opened".into(),
                at: *at,
            }),
            Some(token) => Err(ParseError {
                message: "unexpected text after the end of the query".into(),
                at: token.at(),
            }),
        }
    }
}

/// Turn one run of text into a [`Term`].
fn parse_term(text: &str, at: usize) -> Result<Term, ParseError> {
    let lowered = text.to_ascii_lowercase();
    let collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");

    match collapsed.as_str() {
        "overdue" | "od" => return Ok(Term::Overdue),
        "no date" | "nodate" | "no due date" => return Ok(Term::NoDate),
        "no deadline" => return Ok(Term::NoDeadline),
        "no labels" | "no label" => return Ok(Term::NoLabels),
        "recurring" => return Ok(Term::Recurring),
        "subtask" => return Ok(Term::Subtask),
        "pinned" => return Ok(Term::Pinned),
        "completed" | "done" => return Ok(Term::Completed),
        "all" => {
            return Err(ParseError {
                message: "`all` is not a condition; leave the query empty instead".into(),
                at,
            })
        }
        _ => {}
    }

    if let Some(priority) = Priority::from_token(&collapsed) {
        return Ok(Term::Priority(priority));
    }

    if let Some(name) = text.strip_prefix("##") {
        return named(name, at, "project", |name| Term::Project {
            name,
            include_subprojects: true,
        });
    }
    if let Some(name) = text.strip_prefix('#') {
        return named(name, at, "project", |name| Term::Project {
            name,
            include_subprojects: false,
        });
    }
    if let Some(name) = text.strip_prefix('@') {
        return named(name, at, "label", Term::Label);
    }
    if let Some(name) = text.strip_prefix('/') {
        return named(name, at, "section", Term::Section);
    }

    if let Some(rest) = collapsed.strip_prefix("search:") {
        let needle = text[text.len() - rest.len()..].trim();
        return named(needle, at, "search", Term::Search);
    }

    // `due: ...`, `date: ...`, `deadline: ...`, or a bare date phrase.
    let (rest, is_deadline) = match strip_field(&collapsed) {
        Some((rest, is_deadline)) => (rest, is_deadline),
        None => (collapsed.as_str(), false),
    };

    let filter = parse_date_filter(rest, at)?;
    Ok(if is_deadline {
        Term::Deadline(filter)
    } else {
        Term::Due(filter)
    })
}

/// Build a term from a name, rejecting an empty one.
fn named(
    name: &str,
    at: usize,
    kind: &str,
    build: impl FnOnce(String) -> Term,
) -> Result<Term, ParseError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ParseError {
            message: format!("this needs a {kind} name after it"),
            at,
        });
    }
    Ok(build(name.to_string()))
}

/// Strip a leading `due`/`date`/`deadline`, with or without its colon.
/// Returns the remainder and whether it was the deadline field.
fn strip_field(text: &str) -> Option<(&str, bool)> {
    for (prefix, is_deadline) in [
        ("deadline:", true),
        ("deadline ", true),
        ("due:", false),
        ("due ", false),
        ("date:", false),
        ("date ", false),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return Some((rest.trim(), is_deadline));
        }
    }
    None
}

/// `friday`, `before: friday`, `after friday`.
fn parse_date_filter(text: &str, at: usize) -> Result<DateFilter, ParseError> {
    let text = text.trim();

    for (prefix, build) in [
        ("before:", DateFilter::Before as fn(DateSpec) -> DateFilter),
        ("before ", DateFilter::Before),
        ("after:", DateFilter::After),
        ("after ", DateFilter::After),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return Ok(build(date_spec(rest.trim(), at)?));
        }
    }

    Ok(DateFilter::On(date_spec(text, at)?))
}

/// Validate a date phrase at parse time so a typo is reported as a broken
/// query rather than as a filter that silently matches nothing.
fn date_spec(phrase: &str, at: usize) -> Result<DateSpec, ParseError> {
    let phrase = phrase.trim();
    if phrase.is_empty() {
        return Err(ParseError {
            message: "this needs a date after it".into(),
            at,
        });
    }
    // Any date will do for the check — a phrase either parses on every day or
    // on none, and a real `today` is not available here.
    let probe = NaiveDate::from_ymd_opt(2026, 7, 30).expect("a valid date");
    if parse_date(phrase, probe).is_none() {
        return Err(ParseError {
            message: format!("`{phrase}` is not a date I understand"),
            at,
        });
    }
    Ok(DateSpec::Phrase(phrase.to_string()))
}

impl Store {
    /// Every task matching a query, in no particular order.
    ///
    /// Sorting is the view's business, not the filter's — the same query is
    /// rendered by due date in Today and by priority in a project.
    pub fn query<'a>(&'a self, query: &Query, today: NaiveDate) -> Vec<&'a Task> {
        let context = Context { store: self, today };
        self.tasks()
            .iter()
            .filter(|task| query.matches(task, &context))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::due::Due;
    use crate::id::ProjectId;
    use crate::project::{Project, Section};
    use crate::recurrence::{Recurrence, Unit};
    use chrono::{DateTime, Utc};

    /// Thursday, 30 July 2026.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn now() -> DateTime<Utc> {
        today().and_hms_opt(12, 0, 0).unwrap().and_utc()
    }

    /// A store backed by a path that is never written to. These tests only
    /// query, so the file never has to exist.
    fn store() -> Store {
        let (store, _) = Store::open_at(
            tempfile::TempDir::new()
                .expect("a temporary directory")
                .keep()
                .join("planner.json"),
        );
        store
    }

    fn task(store: &mut Store, content: &str) -> crate::id::TaskId {
        store.add_task(Task::new(ProjectId::inbox(), content, now()))
    }

    fn matching(store: &Store, source: &str) -> Vec<String> {
        let query = Query::parse(source).expect("query parses");
        let mut names: Vec<String> = store
            .query(&query, today())
            .into_iter()
            .map(|task| task.content.clone())
            .collect();
        names.sort();
        names
    }

    // --- parsing --------------------------------------------------------

    #[test]
    fn a_bare_date_phrase_filters_on_the_due_date() {
        let query = Query::parse("today").unwrap();
        assert_eq!(
            query.lists,
            vec![Filter::Term(Term::Due(DateFilter::On(DateSpec::Phrase(
                "today".into()
            ))))]
        );
    }

    #[test]
    fn the_field_can_be_named_with_or_without_a_colon() {
        let with = Query::parse("due: friday").unwrap();
        let without = Query::parse("due friday").unwrap();
        let bare = Query::parse("friday").unwrap();
        assert_eq!(with.lists, without.lists);
        assert_eq!(with.lists, bare.lists);
    }

    #[test]
    fn before_and_after_parse_on_either_field() {
        assert_eq!(
            Query::parse("deadline before: friday").unwrap().lists,
            vec![Filter::Term(Term::Deadline(DateFilter::Before(
                DateSpec::Phrase("friday".into())
            )))]
        );
        assert_eq!(
            Query::parse("due after monday").unwrap().lists,
            vec![Filter::Term(Term::Due(DateFilter::After(
                DateSpec::Phrase("monday".into())
            )))]
        );
    }

    #[test]
    fn operators_bind_and_more_tightly_than_or() {
        // p1 & @work | p2 parses as (p1 & @work) | p2
        let query = Query::parse("p1 & @work | p2").unwrap();
        assert_eq!(
            query.lists,
            vec![Filter::Or(
                Box::new(Filter::And(
                    Box::new(Filter::Term(Term::Priority(Priority::P1))),
                    Box::new(Filter::Term(Term::Label("work".into()))),
                )),
                Box::new(Filter::Term(Term::Priority(Priority::P2))),
            )]
        );
    }

    #[test]
    fn brackets_override_precedence() {
        let query = Query::parse("p1 & (@work | @home)").unwrap();
        assert!(matches!(query.lists[0], Filter::And(_, _)));
    }

    #[test]
    fn a_comma_makes_separate_lists() {
        let query = Query::parse("today, overdue").unwrap();
        assert_eq!(query.lists.len(), 2);
    }

    #[test]
    fn a_name_may_contain_spaces() {
        let query = Query::parse("#My Big Project").unwrap();
        assert_eq!(
            query.lists,
            vec![Filter::Term(Term::Project {
                name: "My Big Project".into(),
                include_subprojects: false
            })]
        );
    }

    #[test]
    fn a_backslash_escapes_an_operator_inside_a_name() {
        let query = Query::parse(r"#R\&D").unwrap();
        assert_eq!(
            query.lists,
            vec![Filter::Term(Term::Project {
                name: "R&D".into(),
                include_subprojects: false
            })]
        );
    }

    #[test]
    fn two_hashes_reach_into_subprojects() {
        let query = Query::parse("##Work").unwrap();
        assert_eq!(
            query.lists,
            vec![Filter::Term(Term::Project {
                name: "Work".into(),
                include_subprojects: true
            })]
        );
    }

    #[test]
    fn a_broken_query_says_where_it_broke() {
        let error = Query::parse("p1 &").unwrap_err();
        assert!(error
            .message
            .contains("ends where a condition was expected"));

        let error = Query::parse("(p1").unwrap_err();
        assert_eq!(error.at, 0);
        assert!(error.message.contains("never closed"));

        let error = Query::parse("p1)").unwrap_err();
        assert!(error.message.contains("never opened"));
    }

    #[test]
    fn an_unparseable_date_is_rejected_rather_than_matching_nothing() {
        let error = Query::parse("due: lunchtime").unwrap_err();
        assert!(error.message.contains("not a date I understand"));
    }

    #[test]
    fn an_empty_name_is_rejected() {
        assert!(Query::parse("@").is_err());
        assert!(Query::parse("#").is_err());
    }

    // --- evaluation -----------------------------------------------------

    #[test]
    fn completed_tasks_are_hidden_unless_the_query_asks_for_them() {
        let mut store = store();
        let open = task(&mut store, "Open");
        let done = task(&mut store, "Done");
        store.task_mut(&open).unwrap().due = Some(Due::on(today()));
        store.task_mut(&done).unwrap().due = Some(Due::on(today()));
        store.complete_task(&done, now(), today());

        assert_eq!(matching(&store, "today"), vec!["Open"]);
        assert_eq!(matching(&store, "completed"), vec!["Done"]);
        assert_eq!(matching(&store, "today & completed"), vec!["Done"]);
    }

    #[test]
    fn the_completed_rule_survives_being_buried_in_the_expression() {
        let mut store = store();
        let done = task(&mut store, "Done");
        store.complete_task(&done, now(), today());

        assert_eq!(matching(&store, "!(p1 & !completed)"), vec!["Done"]);
    }

    #[test]
    fn today_is_due_today_or_overdue() {
        let mut store = store();
        let overdue = task(&mut store, "Overdue");
        let due = task(&mut store, "Due today");
        let later = task(&mut store, "Later");
        store.task_mut(&overdue).unwrap().due = Some(Due::on(date(2026, 7, 1)));
        store.task_mut(&due).unwrap().due = Some(Due::on(today()));
        store.task_mut(&later).unwrap().due = Some(Due::on(date(2026, 8, 30)));

        assert_eq!(
            matching(&store, "due: today | overdue"),
            vec!["Due today", "Overdue"]
        );
    }

    #[test]
    fn a_saved_filter_resolves_today_when_it_runs_not_when_it_was_written() {
        let mut store = store();
        let id = task(&mut store, "Tomorrow's task");
        store.task_mut(&id).unwrap().due = Some(Due::on(date(2026, 7, 31)));

        let query = Query::parse("due: today").unwrap();
        assert!(store.query(&query, today()).is_empty());
        // Run the same query a day later and it matches.
        assert_eq!(store.query(&query, date(2026, 7, 31)).len(), 1);
    }

    #[test]
    fn date_comparisons() {
        let mut store = store();
        let early = task(&mut store, "Early");
        let late = task(&mut store, "Late");
        store.task_mut(&early).unwrap().due = Some(Due::on(date(2026, 7, 20)));
        store.task_mut(&late).unwrap().due = Some(Due::on(date(2026, 8, 20)));

        assert_eq!(matching(&store, "due before: today"), vec!["Early"]);
        assert_eq!(matching(&store, "due after: today"), vec!["Late"]);
    }

    #[test]
    fn deadlines_are_filtered_separately_from_due_dates() {
        let mut store = store();
        let id = task(&mut store, "Report");
        let task_mut = store.task_mut(&id).unwrap();
        task_mut.due = Some(Due::on(date(2026, 8, 4)));
        task_mut.deadline = Some(date(2026, 7, 31));

        assert_eq!(
            matching(&store, "deadline before: 2026-08-01"),
            vec!["Report"]
        );
        assert!(matching(&store, "due before: 2026-08-01").is_empty());
    }

    #[test]
    fn no_date_and_recurring() {
        let mut store = store();
        task(&mut store, "Someday");
        let repeating = task(&mut store, "Weekly");
        store.task_mut(&repeating).unwrap().due =
            Some(Due::on(today()).repeating(Recurrence::every(1, Unit::Week)));

        assert_eq!(matching(&store, "no date"), vec!["Someday"]);
        assert_eq!(matching(&store, "recurring"), vec!["Weekly"]);
    }

    #[test]
    fn labels_match_by_name_regardless_of_case() {
        let mut store = store();
        let label = store.label_for_name("Errand");
        let id = task(&mut store, "Post office");
        task(&mut store, "Something else");
        store.task_mut(&id).unwrap().add_label(label);

        assert_eq!(matching(&store, "@errand"), vec!["Post office"]);
        assert_eq!(matching(&store, "@ERRAND"), vec!["Post office"]);
        assert_eq!(matching(&store, "no labels"), vec!["Something else"]);
    }

    #[test]
    fn one_hash_stays_out_of_subprojects_and_two_reach_in() {
        let mut store = store();
        let parent = store.add_project(Project::new("Work", Color::Blue));
        let mut child_project = Project::new("Admin", Color::Teal);
        child_project.parent_id = Some(parent.clone());
        let child = store.add_project(child_project);

        store.add_task(Task::new(parent.clone(), "In Work", now()));
        store.add_task(Task::new(child, "In Admin", now()));

        assert_eq!(matching(&store, "#Work"), vec!["In Work"]);
        assert_eq!(matching(&store, "##Work"), vec!["In Admin", "In Work"]);
    }

    #[test]
    fn an_unknown_project_matches_nothing_rather_than_everything() {
        let mut store = store();
        task(&mut store, "Anything");
        assert!(matching(&store, "#Nonexistent").is_empty());
    }

    #[test]
    fn sections_match_by_name() {
        let mut store = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let section = store.add_section(Section::new(project.clone(), "Doing"));

        let mut in_section = Task::new(project.clone(), "In progress", now());
        in_section.section_id = Some(section);
        store.add_task(in_section);
        store.add_task(Task::new(project, "Unsorted", now()));

        assert_eq!(matching(&store, "/Doing"), vec!["In progress"]);
    }

    #[test]
    fn search_covers_the_description_as_well_as_the_title() {
        let mut store = store();
        let id = task(&mut store, "Call the plumber");
        task(&mut store, "Unrelated");
        store.task_mut(&id).unwrap().description = "about the leaking TAP".into();

        assert_eq!(
            matching(&store, "search: plumber"),
            vec!["Call the plumber"]
        );
        assert_eq!(matching(&store, "search: tap"), vec!["Call the plumber"]);
    }

    #[test]
    fn negation_and_conjunction_combine() {
        let mut store = store();
        let a = task(&mut store, "Urgent errand");
        let b = task(&mut store, "Urgent desk job");
        let label = store.label_for_name("errand");
        store.task_mut(&a).unwrap().priority = Priority::P1;
        store.task_mut(&a).unwrap().add_label(label);
        store.task_mut(&b).unwrap().priority = Priority::P1;

        assert_eq!(matching(&store, "p1 & !@errand"), vec!["Urgent desk job"]);
    }

    #[test]
    fn a_comma_query_matches_the_union_of_its_lists() {
        let mut store = store();
        let a = task(&mut store, "Pinned");
        let b = task(&mut store, "Urgent");
        task(&mut store, "Neither");
        store.task_mut(&a).unwrap().pinned = true;
        store.task_mut(&b).unwrap().priority = Priority::P1;

        assert_eq!(matching(&store, "pinned, p1"), vec!["Pinned", "Urgent"]);
    }

    #[test]
    fn subtasks_can_be_singled_out_or_excluded() {
        let mut store = store();
        let parent = task(&mut store, "Parent");
        let child = task(&mut store, "Child");
        store.task_mut(&child).unwrap().parent_id = Some(parent);

        assert_eq!(matching(&store, "subtask"), vec!["Child"]);
        assert_eq!(matching(&store, "!subtask"), vec!["Parent"]);
    }

    #[test]
    fn the_built_in_views_are_all_expressible_as_queries() {
        for source in [
            "due: today | overdue",
            "overdue",
            "pinned",
            "completed",
            "no date",
            "recurring",
            "no labels",
            "due after: today",
            "p1 | p2",
            "#Inbox",
        ] {
            assert!(Query::parse(source).is_ok(), "{source} should parse");
        }
    }
}
