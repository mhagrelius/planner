//! Reading an argument list.
//!
//! The surface is positional words and `key=value` pairs, never `--flags`.
//! That is not a style preference: `GApplication` parses the command line with
//! `GOption` before the application ever sees it and refuses any option it was
//! not told about in advance, while an unrecognised *word* is passed straight
//! through to the handler. A `--flag` invented for one verb would therefore be
//! rejected by the launcher rather than reaching the code that understands it.
//! Positional syntax is the only form that can carry a verb's own arguments
//! through that gate, so it is the form the whole surface uses.
//!
//! Parsing is separate from executing because the two fail for unrelated
//! reasons. "I do not know that verb" is answerable with no store at all, and
//! answering it here means the error can list what the verbs actually are.

use super::help;
use super::{AgentError, ErrorKind};
use crate::model::priority::Priority;

/// How many results a list or search returns when the caller does not say.
///
/// Fifty is about as much as is worth putting in front of a model in one go.
/// Nothing is silently dropped: a truncated response says how many matched.
pub const DEFAULT_LIMIT: usize = 50;

/// One thing the assistant asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// The whole surface, or one verb of it.
    Help {
        verb: Option<String>,
    },
    /// Every verb as JSON, for a caller generating tool definitions.
    Describe,
    Overview,
    List {
        query: Option<String>,
        limit: usize,
    },
    Show {
        task: String,
    },
    Search {
        text: String,
        limit: usize,
    },
    Add {
        line: String,
    },
    Subtask {
        parent: String,
        line: String,
    },
    Complete {
        task: String,
    },
    Reopen {
        task: String,
    },
    Delete {
        task: String,
    },
    Update {
        task: String,
        changes: Vec<Change>,
    },
    AddProject {
        name: String,
        parent: Option<String>,
    },
    RenameProject {
        project: String,
        name: String,
    },
    RemoveProject {
        project: String,
    },
}

/// One field of one task, set to one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Title(String),
    Description(String),
    /// `None` clears the date. A cleared due date takes any repeat with it.
    Due(Option<String>),
    Deadline(Option<String>),
    Priority(Priority),
    Project(String),
    Section(Option<String>),
    Pinned(bool),
    AddLabel(String),
    RemoveLabel(String),
}

impl Command {
    /// Whether running this would change the store.
    ///
    /// Read by the caller before it touches a store loaded from a file it must
    /// not write, and by the application to decide whether to save and redraw.
    pub fn changes_the_store(&self) -> bool {
        matches!(
            self,
            Self::Add { .. }
                | Self::Subtask { .. }
                | Self::Complete { .. }
                | Self::Reopen { .. }
                | Self::Delete { .. }
                | Self::Update { .. }
                | Self::AddProject { .. }
                | Self::RenameProject { .. }
                | Self::RemoveProject { .. }
        )
    }
}

/// Read the arguments that followed `agent`.
///
/// No arguments at all is the help, not an error: a caller that runs the verb
/// with nothing after it is asking what it can do.
pub fn parse(args: &[String]) -> Result<Command, AgentError> {
    let Some((verb, rest)) = args.split_first() else {
        return Ok(Command::Help { verb: None });
    };

    let verb = verb.to_ascii_lowercase();
    // `help` after a verb reads the same way round as before it. An assistant
    // that has just been told a verb exists will try both.
    if rest.first().is_some_and(|word| word == "help") && verb != "help" {
        return Ok(Command::Help { verb: Some(verb) });
    }

    match help::canonical_verb(&verb) {
        Some("help") => Ok(Command::Help {
            verb: rest.first().map(|verb| verb.to_ascii_lowercase()),
        }),
        Some("describe") => Ok(Command::Describe),
        Some("overview") => Ok(Command::Overview),
        Some("list") => {
            let (words, pairs) = split_pairs(rest);
            let limit = take_limit(&pairs, "list")?;
            let query = join(&words);
            Ok(Command::List {
                query: (!query.is_empty()).then_some(query),
                limit,
            })
        }
        Some("search") => {
            let (words, pairs) = split_pairs(rest);
            let limit = take_limit(&pairs, "search")?;
            let text = join(&words);
            if text.is_empty() {
                return Err(missing("search", "some text to look for"));
            }
            Ok(Command::Search { text, limit })
        }
        Some("show") => Ok(Command::Show {
            task: reference(rest, "show")?,
        }),
        Some("complete") => Ok(Command::Complete {
            task: reference(rest, "complete")?,
        }),
        Some("reopen") => Ok(Command::Reopen {
            task: reference(rest, "reopen")?,
        }),
        Some("delete") => Ok(Command::Delete {
            task: reference(rest, "delete")?,
        }),
        Some("add") => {
            let line = join(rest);
            if line.is_empty() {
                return Err(missing("add", "a quick-add line"));
            }
            Ok(Command::Add { line })
        }
        Some("subtask") => {
            let (parent, rest) = first_then_rest(rest, "subtask", "a parent task")?;
            let line = join(rest);
            if line.is_empty() {
                return Err(missing("subtask", "a quick-add line for the subtask"));
            }
            Ok(Command::Subtask { parent, line })
        }
        Some("update") => {
            let (words, pairs) = split_pairs(rest);
            let task = join(&words);
            if task.is_empty() {
                return Err(missing("update", "a task to change"));
            }
            if pairs.is_empty() {
                return Err(missing("update", "at least one `field=value`"));
            }
            let changes = pairs
                .iter()
                .map(|(key, value)| change(key, value))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Command::Update { task, changes })
        }
        Some("add-project") => {
            let (words, pairs) = split_pairs(rest);
            let name = join(&words);
            if name.is_empty() {
                return Err(missing("add-project", "a name"));
            }
            let mut parent = None;
            for (key, value) in &pairs {
                match key.as_str() {
                    "parent" => parent = Some(value.clone()),
                    _ => return Err(unknown_field(key, "add-project", &["parent"])),
                }
            }
            Ok(Command::AddProject { name, parent })
        }
        Some("rename-project") => {
            let (project, rest) = first_then_rest(rest, "rename-project", "a project")?;
            let name = join(rest);
            if name.is_empty() {
                return Err(missing("rename-project", "a new name"));
            }
            Ok(Command::RenameProject { project, name })
        }
        Some("remove-project") => Ok(Command::RemoveProject {
            project: reference(rest, "remove-project")?,
        }),
        _ => Err(AgentError {
            kind: ErrorKind::UnknownVerb,
            message: format!(
                "`{verb}` is not a verb. The verbs are: {}.",
                help::verb_names().join(", ")
            ),
            candidates: Vec::new(),
            hint: Some("Run `planner agent help` for what each one does.".into()),
        }),
    }
}

/// A reference made of every remaining word.
///
/// Joining rather than demanding one quoted argument, because these verbs take
/// nothing else — there is no second argument for a loose word to be mistaken
/// for, so `complete Email Sam` can only mean one thing and refusing it would
/// be pedantry.
fn reference(args: &[String], verb: &str) -> Result<String, AgentError> {
    let reference = join(args);
    if reference.is_empty() {
        return Err(missing(verb, "a task"));
    }
    Ok(reference)
}

/// The first argument, and everything after it.
///
/// Used where a reference is followed by free text. Here the reference *must*
/// be a single argument, because "where does the reference end and the text
/// begin" has no other answer. Callers spawn an argument list rather than a
/// shell command, so quoting it is not a burden.
fn first_then_rest<'a>(
    args: &'a [String],
    verb: &str,
    wanted: &str,
) -> Result<(String, &'a [String]), AgentError> {
    match args.split_first() {
        Some((first, rest)) => Ok((first.clone(), rest)),
        None => Err(missing(verb, wanted)),
    }
}

fn join(words: &[String]) -> String {
    words.join(" ").trim().to_string()
}

/// Split arguments into the leading words and the `key=value` pairs.
///
/// A token shaped like `key=` opens a pair, and the words after it belong to
/// its value until the next such token. That is what makes
/// `title=Email Sam about the lease` mean one thing whether or not the caller
/// remembered to quote it.
///
/// The key must be lower-case ASCII, so an `=` inside prose — a title, a
/// search term — does not turn the rest of the line into a field.
fn split_pairs(args: &[String]) -> (Vec<String>, Vec<(String, String)>) {
    let mut leading = Vec::new();
    let mut pairs: Vec<(String, String)> = Vec::new();

    for argument in args {
        match key_of(argument) {
            Some((key, first)) => pairs.push((key, first.to_string())),
            None => match pairs.last_mut() {
                Some((_, value)) => {
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(argument);
                }
                None => leading.push(argument.clone()),
            },
        }
    }
    (leading, pairs)
}

/// The key a `key=value` token opens, and the part of the value it carried.
fn key_of(argument: &str) -> Option<(String, &str)> {
    let (key, value) = argument.split_once('=')?;
    let shaped = !key.is_empty()
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    shaped.then(|| (key.to_string(), value))
}

fn take_limit(pairs: &[(String, String)], verb: &str) -> Result<usize, AgentError> {
    let mut limit = DEFAULT_LIMIT;
    for (key, value) in pairs {
        match key.as_str() {
            "limit" => {
                limit = value.trim().parse().map_err(|_| AgentError {
                    kind: ErrorKind::BadValue,
                    message: format!("`limit={value}` is not a whole number."),
                    candidates: Vec::new(),
                    hint: None,
                })?
            }
            _ => return Err(unknown_field(key, verb, &["limit"])),
        }
    }
    Ok(limit)
}

/// Read one `field=value` of an update.
fn change(key: &str, value: &str) -> Result<Change, AgentError> {
    let value = value.trim();
    // A field that can be unset is cleared by naming nothing, or by the word
    // itself. `due=` from a template that had no date to fill in means the
    // same as `due=none`, and either is better than a task due "none".
    let cleared = value.is_empty() || value.eq_ignore_ascii_case("none");

    match key {
        "title" | "content" => Ok(Change::Title(value.to_string())),
        "description" | "notes" => Ok(Change::Description(value.to_string())),
        "due" | "date" => Ok(Change::Due((!cleared).then(|| value.to_string()))),
        "deadline" => Ok(Change::Deadline((!cleared).then(|| value.to_string()))),
        "priority" => match Priority::from_token(value) {
            Some(priority) => Ok(Change::Priority(priority)),
            None if cleared => Ok(Change::Priority(Priority::P4)),
            None => Err(AgentError {
                kind: ErrorKind::BadValue,
                message: format!("`priority={value}` is not a priority. Use p1, p2, p3 or p4."),
                candidates: Vec::new(),
                hint: Some("p1 is the most urgent; p4 means no priority.".into()),
            }),
        },
        "project" => {
            if cleared {
                return Err(AgentError {
                    kind: ErrorKind::BadValue,
                    message: "A task is always in a project. Move it to Inbox rather than \
                              clearing it."
                        .into(),
                    candidates: Vec::new(),
                    hint: Some("project=Inbox".into()),
                });
            }
            Ok(Change::Project(value.to_string()))
        }
        "section" => Ok(Change::Section((!cleared).then(|| value.to_string()))),
        "pinned" | "pin" => match boolean(value) {
            Some(pinned) => Ok(Change::Pinned(pinned)),
            None => Err(AgentError {
                kind: ErrorKind::BadValue,
                message: format!("`pinned={value}` is not true or false."),
                candidates: Vec::new(),
                hint: None,
            }),
        },
        "add-label" | "label" => Ok(Change::AddLabel(value.to_string())),
        "remove-label" | "unlabel" => Ok(Change::RemoveLabel(value.to_string())),
        _ => Err(unknown_field(
            key,
            "update",
            &[
                "title",
                "description",
                "due",
                "deadline",
                "priority",
                "project",
                "section",
                "pinned",
                "add-label",
                "remove-label",
            ],
        )),
    }
}

fn boolean(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn missing(verb: &str, wanted: &str) -> AgentError {
    AgentError {
        kind: ErrorKind::MissingArgument,
        message: format!("`{verb}` needs {wanted}."),
        candidates: Vec::new(),
        hint: Some(format!(
            "Run `planner agent help {verb}` for the arguments."
        )),
    }
}

fn unknown_field(key: &str, verb: &str, allowed: &[&str]) -> AgentError {
    AgentError {
        kind: ErrorKind::UnknownField,
        message: format!(
            "`{verb}` has no `{key}` field. It takes: {}.",
            allowed.join(", ")
        ),
        candidates: Vec::new(),
        hint: Some(format!("Run `planner agent help {verb}`.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    fn parsed(line: &str) -> Command {
        parse(&args(line)).expect("a command")
    }

    #[test]
    fn no_arguments_at_all_is_a_request_for_help() {
        assert_eq!(parse(&[]).unwrap(), Command::Help { verb: None });
    }

    #[test]
    fn help_reads_the_same_before_or_after_a_verb() {
        let expected = Command::Help {
            verb: Some("update".into()),
        };
        assert_eq!(parsed("help update"), expected);
        assert_eq!(parsed("update help"), expected);
    }

    #[test]
    fn an_unknown_verb_is_told_what_the_verbs_are() {
        let error = parse(&args("frobnicate")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownVerb);
        assert!(error.message.contains("complete"), "{}", error.message);
    }

    #[test]
    fn a_query_keeps_its_spaces_and_operators() {
        assert_eq!(
            parsed("list due: today | overdue"),
            Command::List {
                query: Some("due: today | overdue".into()),
                limit: DEFAULT_LIMIT,
            }
        );
    }

    #[test]
    fn a_limit_is_taken_out_of_the_query_rather_than_searched_for() {
        assert_eq!(
            parsed("list #Work limit=5"),
            Command::List {
                query: Some("#Work".into()),
                limit: 5,
            }
        );
    }

    #[test]
    fn a_limit_that_is_not_a_number_says_so() {
        let error = parse(&args("list limit=lots")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::BadValue);
    }

    #[test]
    fn an_unquoted_field_value_keeps_its_words() {
        let Command::Update { task, changes } = parsed("update Email title=Ring Sam instead")
        else {
            panic!("an update");
        };
        assert_eq!(task, "Email");
        assert_eq!(changes, vec![Change::Title("Ring Sam instead".into())]);
    }

    #[test]
    fn one_field_ends_where_the_next_begins() {
        let Command::Update { task, changes } =
            parsed("update Email title=Ring Sam due=next friday priority=p1")
        else {
            panic!("an update");
        };
        assert_eq!(task, "Email");
        assert_eq!(
            changes,
            vec![
                Change::Title("Ring Sam".into()),
                Change::Due(Some("next friday".into())),
                Change::Priority(Priority::P1),
            ]
        );
    }

    #[test]
    fn a_task_reference_may_be_several_loose_words() {
        assert_eq!(
            parsed("complete Email Sam about the lease"),
            Command::Complete {
                task: "Email Sam about the lease".into(),
            }
        );
    }

    #[test]
    fn an_equals_sign_in_prose_is_not_a_field() {
        // The title is free text, so `2+2=4` must survive being an argument.
        assert_eq!(
            parsed("add Prove that 2+2=4"),
            Command::Add {
                line: "Prove that 2+2=4".into(),
            }
        );
        assert_eq!(
            parsed("search E=mc2"),
            Command::Search {
                text: "E=mc2".into(),
                limit: DEFAULT_LIMIT,
            }
        );
    }

    #[test]
    fn a_quick_add_line_is_handed_over_untouched() {
        assert_eq!(
            parsed("add Email Sam #Work @email p2 friday 9am"),
            Command::Add {
                line: "Email Sam #Work @email p2 friday 9am".into(),
            }
        );
    }

    #[test]
    fn a_subtask_takes_its_parent_as_one_argument_and_the_rest_as_the_line() {
        let command = parse(&[
            "subtask".into(),
            "Move house".into(),
            "Pack".into(),
            "the".into(),
            "books".into(),
        ])
        .unwrap();
        assert_eq!(
            command,
            Command::Subtask {
                parent: "Move house".into(),
                line: "Pack the books".into(),
            }
        );
    }

    #[test]
    fn a_field_left_empty_clears_the_date_rather_than_setting_it_to_nothing() {
        let Command::Update { changes, .. } = parsed("update Email due=") else {
            panic!("an update");
        };
        assert_eq!(changes, vec![Change::Due(None)]);

        let Command::Update { changes, .. } = parsed("update Email due=none") else {
            panic!("an update");
        };
        assert_eq!(changes, vec![Change::Due(None)]);
    }

    #[test]
    fn an_unknown_field_lists_the_ones_that_exist() {
        let error = parse(&args("update Email colour=blue")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownField);
        assert!(error.message.contains("priority"), "{}", error.message);
    }

    #[test]
    fn an_update_with_nothing_to_change_is_refused() {
        let error = parse(&args("update Email")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::MissingArgument);
    }

    #[test]
    fn clearing_a_project_is_refused_with_the_thing_to_do_instead() {
        let error = parse(&args("update Email project=none")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::BadValue);
        assert_eq!(error.hint.as_deref(), Some("project=Inbox"));
    }

    #[test]
    fn every_verb_the_help_advertises_can_be_parsed() {
        // The help table and the parser are two lists of verbs. This is what
        // stops one growing an entry the other has never heard of.
        for verb in help::verb_names() {
            let command = parse(&[verb.to_string(), "help".to_string()]);
            assert!(command.is_ok(), "`{verb}` is documented but not parsed");
        }
    }

    #[test]
    fn only_the_verbs_that_write_say_they_change_the_store() {
        assert!(parsed("add Something").changes_the_store());
        assert!(parsed("complete Something").changes_the_store());
        assert!(parsed("delete Something").changes_the_store());
        assert!(!parsed("list").changes_the_store());
        assert!(!parsed("overview").changes_the_store());
        assert!(!parsed("show Something").changes_the_store());
    }
}
