//! The interface an assistant drives the planner through.
//!
//! A second way into the same data is a second way to get it wrong, so this is
//! built to be as little of one as possible. It adds no vocabulary: a task is
//! created from the same quick-add line the dialog parses, and a list is the
//! same filter query the sidebar runs. What is genuinely new here is only the
//! shape of the answers — names where the records hold ids — and the rules for
//! turning a thing the user *said* into the record they meant.
//!
//! **It refuses rather than guesses.** "Complete the email one" with two open
//! tasks matching is an error listing both with their ids, not a coin flip.
//! An assistant that picks wrong here ticks off the wrong piece of the user's
//! life, and a clarifying question costs a sentence.
//!
//! **It never claims more than happened.** Completing a repeating task moves
//! it on rather than finishing it, and the response says `completed-and-repeats`
//! with the date it comes back on. An update reports which fields actually
//! changed, so a value that was already set reads as a no-op instead of a
//! success.
//!
//! **Nothing here reads the clock or the filesystem.** `now` and `today` are
//! arguments and the store is borrowed, which is what lets the whole surface —
//! including every error message an assistant will ever see — be tested
//! without a display, a bus, or a real planner file.
//!
//! Where this runs matters and is decided by the caller — the GTK shell's
//! `ui::application`, which lives in the `planner` crate above this one: when
//! Planner is open, the command is handed to the running instance, because it
//! holds the document in memory and would otherwise overwrite anything written
//! behind its back.

pub mod command;
pub mod help;
pub mod view;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use crate::id::{ProjectId, SectionId, TaskId};
use crate::parse::{parse_date, parse_quick_add, Vocabulary};
use crate::project::Project;
use crate::query::Query;
use crate::search::search;
use crate::store::Store;
use crate::task::Task;
use crate::Due;

pub use command::{parse, Change, Command};
pub use view::Response;

/// The kind of a failure, as a stable string a caller can branch on.
///
/// Separate from the message because the two have different jobs: the kind is
/// for code deciding what to do next, the message is for the model deciding
/// what to say. Neither reads well doing the other's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    UnknownVerb,
    MissingArgument,
    UnknownField,
    BadValue,
    BadQuery,
    BadDate,
    NotFound,
    /// The reference fits more than one thing. `candidates` says which.
    Ambiguous,
    /// The file is from a newer version of Planner and will not be written.
    ReadOnly,
    /// Understood, and not allowed.
    Refused,
}

/// One of the things a reference could have meant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    /// Where it lives, so two tasks with the same title can be told apart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Why a command did not run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentError {
    #[serde(rename = "error")]
    pub kind: ErrorKind,
    /// A whole sentence. This is the part a model reads, so it says what was
    /// wrong rather than naming the rule that was broken.
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    /// What to do about it, when there is a specific answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl AgentError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            candidates: Vec::new(),
            hint: None,
        }
    }

    fn hinted(kind: ErrorKind, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            hint: Some(hint.into()),
            ..Self::new(kind, message)
        }
    }
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AgentError {}

/// Run one command against a store.
pub fn execute(
    store: &mut Store,
    command: Command,
    now: DateTime<Utc>,
    today: NaiveDate,
) -> Result<Response, AgentError> {
    // Checked once, here, rather than at each write: a store that will not be
    // saved must not be edited in memory either, or the caller reports a
    // success that quietly evaporates.
    if command.changes_the_store() && store.is_read_only() {
        return Err(AgentError::hinted(
            ErrorKind::ReadOnly,
            "This planner file was written by a newer version of Planner, so it will not be \
             changed.",
            "Reading still works. Upgrade Planner to write to it.",
        ));
    }

    match command {
        Command::Help { verb } => {
            let text = match verb {
                None => help::overview(),
                Some(verb) => help::for_verb(&verb).ok_or_else(|| {
                    AgentError::hinted(
                        ErrorKind::UnknownVerb,
                        format!(
                            "`{verb}` is not a verb. The verbs are: {}.",
                            help::verb_names().join(", ")
                        ),
                        "Run `planner agent help` for what each one does.",
                    )
                })?,
            };
            Ok(Response::Help { text })
        }

        Command::Describe => Ok(Response::Describe { verbs: help::VERBS }),

        Command::Overview => Ok(overview(store, today)),

        Command::List { query, limit } => list(store, query, limit, today),

        Command::Show { task } => {
            let id = resolve_task(store, &task)?;
            let task = store.task(&id).expect("just resolved");
            Ok(Response::Show {
                task: view::TaskView::detailed(store, task, today),
            })
        }

        Command::Search { text, limit } => {
            let hits = search(store, &text, limit);
            Ok(Response::Search {
                count: hits.len(),
                hits: hits.iter().map(view::HitView::of).collect(),
            })
        }

        Command::Add { line } => {
            let parsed = parse_quick_add(&line, today, &store.vocabulary());
            if parsed.title.trim().is_empty() {
                return Err(AgentError::hinted(
                    ErrorKind::BadValue,
                    format!(
                        "`{line}` is all tokens and no title, so there is nothing to call \
                             the task."
                    ),
                    "Put the title first: `Email Sam #Work friday`.",
                ));
            }
            let id = store.add_from_quick_add(&parsed, &ProjectId::inbox(), None, now);
            Ok(Response::Added {
                task: view::TaskView::of(store, store.task(&id).expect("just added"), today),
            })
        }

        Command::Subtask { parent, line } => subtask(store, &parent, &line, now, today),

        Command::Complete { task } => {
            let id = resolve_task(store, &task)?;
            let outcome = store
                .complete_task(&id, now, today)
                .expect("just resolved")
                .into();
            Ok(Response::Completed {
                task: view::TaskView::of(store, store.task(&id).expect("still there"), today),
                outcome,
            })
        }

        Command::Reopen { task } => {
            let id = resolve_task(store, &task)?;
            // Whether it was ticked off at all. Reported rather than assumed,
            // so "reopen X" on something already open reads as the no-op it is
            // instead of as an undo that happened.
            let reopened = store.task(&id).is_some_and(|task| task.checked);
            store.uncomplete_task(&id, now);
            Ok(Response::Reopened {
                task: view::TaskView::of(store, store.task(&id).expect("still there"), today),
                reopened,
            })
        }

        Command::Delete { task } => {
            let id = resolve_task(store, &task)?;
            let removed = store.remove_task(&id);
            Ok(Response::Deleted {
                count: removed.len(),
                removed: removed.iter().map(view::RemovedView::of).collect(),
            })
        }

        Command::Update { task, changes } => update(store, &task, &changes, now, today),

        Command::AddProject { name, parent } => {
            let parent = parent
                .as_deref()
                .map(|reference| resolve_project(store, reference))
                .transpose()?;
            let colour = store.next_project_color();
            let mut project = Project::new(name.trim(), colour);
            project.parent_id = parent;
            let id = store.add_project(project);
            Ok(Response::ProjectAdded {
                project: view::ProjectView::of(store, store.project(&id).expect("just added")),
            })
        }

        Command::RenameProject { project, name } => {
            let id = resolve_project(store, &project)?;
            if id.is_inbox() {
                return Err(AgentError::new(
                    ErrorKind::Refused,
                    "The Inbox cannot be renamed. It is where anything without a project goes, \
                     and every other part of the app names it.",
                ));
            }
            store.project_mut(&id).expect("just resolved").name = name.trim().to_string();
            Ok(Response::ProjectRenamed {
                project: view::ProjectView::of(store, store.project(&id).expect("still there")),
            })
        }

        Command::RemoveProject { project } => {
            let id = resolve_project(store, &project)?;
            let name = store
                .project(&id)
                .map(|project| project.name.clone())
                .unwrap_or_default();
            match store.remove_project(&id) {
                Some(removed) => Ok(Response::ProjectRemoved {
                    name,
                    projects: removed.projects.len(),
                    tasks: removed.tasks.len(),
                }),
                None => Err(AgentError::hinted(
                    ErrorKind::Refused,
                    "The Inbox cannot be deleted. It is where a task with no project of its own \
                     lives.",
                    "Delete the tasks in it instead, or move them somewhere else.",
                )),
            }
        }
    }
}

/// Render a result the way the command line prints it.
///
/// Help is text; everything else is one JSON object carrying `ok`, so a caller
/// that only reads stdout can tell success from failure without the exit code
/// — and one that only reads the exit code does not have to parse anything.
pub fn render(result: &Result<Response, AgentError>) -> String {
    #[derive(Serialize)]
    struct Envelope<'a, T: Serialize> {
        ok: bool,
        #[serde(flatten)]
        body: &'a T,
    }

    let rendered = match result {
        Ok(Response::Help { text }) => return text.clone(),
        Ok(response) => serde_json::to_string_pretty(&Envelope {
            ok: true,
            body: response,
        }),
        Err(error) => serde_json::to_string_pretty(&Envelope {
            ok: false,
            body: error,
        }),
    };

    // Serialising these cannot fail — they are plain data with no maps keyed
    // by anything but strings — but a panic in a CLI that has already changed
    // the store would be the worst possible way to say so.
    rendered.unwrap_or_else(|error| {
        format!(r#"{{"ok": false, "error": "internal", "message": "{error}"}}"#)
    })
}

// --- the verbs ----------------------------------------------------------

fn overview(store: &Store, today: NaiveDate) -> Response {
    let counts = store.label_counts();
    let open = store.tasks().iter().filter(|task| !task.checked);

    Response::Overview {
        projects: store
            .projects_ordered()
            .into_iter()
            .map(|project| view::ProjectView::of(store, project))
            .collect(),
        labels: store
            .labels()
            .iter()
            .map(|label| view::LabelView::of(label, counts.get(&label.id).copied().unwrap_or(0)))
            .collect(),
        filters: store
            .filters_ordered()
            .into_iter()
            .map(view::FilterView::of)
            .collect(),
        counts: view::Counts {
            open: open.clone().count(),
            completed: store.tasks().iter().filter(|task| task.checked).count(),
            due_today: open
                .clone()
                .filter(|task| task.due.as_ref().is_some_and(|due| due.date == today))
                .count(),
            overdue: open.clone().filter(|task| task.is_overdue(today)).count(),
            inbox: open.filter(|task| task.project_id.is_inbox()).count(),
        },
    }
}

fn list(
    store: &Store,
    query: Option<String>,
    limit: usize,
    today: NaiveDate,
) -> Result<Response, AgentError> {
    let parsed = match query.as_deref() {
        None => Query::all(),
        Some(source) => Query::parse(source).map_err(|error| {
            AgentError::hinted(
                ErrorKind::BadQuery,
                format!("`{source}` is not a filter query: {error}"),
                "Run `planner agent help list` for the syntax.",
            )
        })?,
    };

    let mut matches = store.query(&parsed, today);
    sort_for_reading(&mut matches);

    let matched = matches.len();
    let tasks: Vec<view::TaskView> = matches
        .iter()
        .take(limit)
        .map(|task| view::TaskView::of(store, task, today))
        .collect();

    Ok(Response::List {
        query,
        count: tasks.len(),
        matched,
        truncated: matched > tasks.len(),
        tasks,
    })
}

/// Soonest first, then most urgent, then by title.
///
/// The app sorts each view its own way; a list going to a reader with no
/// screen wants the order it would be worked in, and wants it to be the same
/// every time so two calls can be compared.
fn sort_for_reading(tasks: &mut [&Task]) {
    tasks.sort_by(|a, b| {
        let date = |task: &Task| task.due.as_ref().map(|due| due.date);
        // An undated task sorts after every dated one rather than before, so
        // `None` cannot masquerade as the distant past.
        match (date(a), date(b)) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| a.priority.rank().cmp(&b.priority.rank()))
        .then_with(|| a.content.cmp(&b.content))
    });
}

fn subtask(
    store: &mut Store,
    parent: &str,
    line: &str,
    now: DateTime<Utc>,
    today: NaiveDate,
) -> Result<Response, AgentError> {
    let parent_id = resolve_task(store, parent)?;
    let parsed = parse_quick_add(line, today, &store.vocabulary());
    if parsed.title.trim().is_empty() {
        return Err(AgentError::new(
            ErrorKind::BadValue,
            format!(
                "`{line}` is all tokens and no title, so there is nothing to call the subtask."
            ),
        ));
    }

    let (project, section) = {
        let parent = store.task(&parent_id).expect("just resolved");
        (parent.project_id.clone(), parent.section_id.clone())
    };

    // A subtask shares its parent's project whatever the line said. One filed
    // elsewhere would be unreachable from the parent it hangs under, and would
    // inflate the count of a project it is not shown in.
    let id = store.add_from_quick_add(&parsed, &project, section.as_ref(), now);
    if let Some(task) = store.task_mut(&id) {
        task.parent_id = Some(parent_id);
        task.project_id = project;
    }

    Ok(Response::Added {
        task: view::TaskView::of(store, store.task(&id).expect("just added"), today),
    })
}

fn update(
    store: &mut Store,
    reference: &str,
    changes: &[Change],
    now: DateTime<Utc>,
    today: NaiveDate,
) -> Result<Response, AgentError> {
    let id = resolve_task(store, reference)?;
    let mut applied: Vec<String> = Vec::new();

    // Where it ends up. Resolved before anything is written, so a bad project
    // name fails with the task untouched rather than half-edited.
    let mut destination: Option<ProjectId> = None;
    let mut section: Option<Option<String>> = None;

    for change in changes {
        match change {
            Change::Project(name) => destination = Some(resolve_project(store, name)?),
            Change::Section(name) => section = Some(name.clone()),
            Change::AddLabel(name) => {
                let label = store.label_for_name(name);
                let task = store.task_mut(&id).expect("just resolved");
                if !task.has_label(&label) {
                    task.add_label(label);
                    task.touch(now);
                    applied.push(format!("label + {name}"));
                }
            }
            Change::RemoveLabel(name) => {
                let label = store.label_by_name(name).map(|label| label.id.clone());
                let Some(label) = label else { continue };
                let task = store.task_mut(&id).expect("just resolved");
                if task.has_label(&label) {
                    task.remove_label(&label);
                    task.touch(now);
                    applied.push(format!("label − {name}"));
                }
            }
            Change::Due(phrase) => {
                let due = phrase
                    .as_deref()
                    .map(|phrase| parse_due(phrase, today))
                    .transpose()?;
                let task = store.task_mut(&id).expect("just resolved");
                if task.due != due {
                    let said = due.as_ref().map_or("none".to_string(), view::due_phrase);
                    task.due = due;
                    task.touch(now);
                    applied.push(format!("due → {said}"));
                }
            }
            Change::Deadline(phrase) => {
                let deadline = phrase
                    .as_deref()
                    .map(|phrase| parse_day(phrase, today))
                    .transpose()?;
                let task = store.task_mut(&id).expect("just resolved");
                if task.deadline != deadline {
                    task.deadline = deadline;
                    task.touch(now);
                    applied.push(format!(
                        "deadline → {}",
                        deadline.map_or("none".to_string(), |date| date.to_string())
                    ));
                }
            }
            Change::Title(title) => {
                let task = store.task_mut(&id).expect("just resolved");
                if task.content != *title {
                    task.content = title.clone();
                    task.touch(now);
                    applied.push(format!("title → {title}"));
                }
            }
            Change::Description(text) => {
                let task = store.task_mut(&id).expect("just resolved");
                if task.description != *text {
                    task.description = text.clone();
                    task.touch(now);
                    applied.push("description changed".to_string());
                }
            }
            Change::Priority(priority) => {
                let task = store.task_mut(&id).expect("just resolved");
                if task.priority != *priority {
                    task.priority = *priority;
                    task.touch(now);
                    applied.push(format!("priority → {}", priority.token()));
                }
            }
            Change::Pinned(pinned) => {
                let task = store.task_mut(&id).expect("just resolved");
                if task.pinned != *pinned {
                    task.pinned = *pinned;
                    task.touch(now);
                    applied.push(format!("pinned → {pinned}"));
                }
            }
        }
    }

    apply_move(store, &id, destination, section, now, &mut applied)?;

    Ok(Response::Updated {
        task: view::TaskView::of(store, store.task(&id).expect("still there"), today),
        applied,
    })
}

/// Move a task between projects and sections, if either was named.
///
/// Done in one step at the end rather than as the fields arrive, because a
/// section only means anything inside a project: `project=Home section=Doing`
/// has to look for Doing in Home, not in wherever the task used to be.
fn apply_move(
    store: &mut Store,
    id: &TaskId,
    destination: Option<ProjectId>,
    section: Option<Option<String>>,
    now: DateTime<Utc>,
    applied: &mut Vec<String>,
) -> Result<(), AgentError> {
    if destination.is_none() && section.is_none() {
        return Ok(());
    }

    let task = store.task(id).expect("just resolved");
    let current_project = task.project_id.clone();
    let current_section = task.section_id.clone();
    let project = destination
        .clone()
        .unwrap_or_else(|| current_project.clone());

    let section = match section {
        // Not mentioned. Only reachable when a project *was*, and a section
        // belongs to one project — so the old one cannot come along.
        None | Some(None) => None,
        Some(Some(name)) => Some(resolve_section(store, &project, &name)?),
    };

    if project == current_project && section == current_section {
        return Ok(());
    }

    // Past the end, so the task lands at the bottom of its new list. There is
    // no position in an assistant's vocabulary and the end is where a person
    // adding something would put it.
    if !store.move_task(id, &project, section.as_ref(), usize::MAX, now) {
        return Err(AgentError::new(
            ErrorKind::Refused,
            "That task could not be moved there.",
        ));
    }

    if let Some(destination) = destination {
        let name = store
            .project(&destination)
            .map(|project| project.name.clone())
            .unwrap_or_default();
        applied.push(format!("project → {name}"));
    }
    let named = section
        .as_ref()
        .and_then(|id| store.section(id))
        .map(|(_, section)| section.name.clone());
    match named {
        Some(name) => applied.push(format!("section → {name}")),
        None if current_section.is_some() => applied.push("section → none".to_string()),
        None => {}
    }
    Ok(())
}

// --- turning what was said into what was meant --------------------------

/// A date phrase, understood exactly as quick-add understands it.
///
/// Run through the whole quick-add parser rather than through [`parse_date`]
/// alone, so `due=` accepts everything the entry box does: a time, a repeat,
/// or both. Reimplementing a subset here would mean `every other monday` works
/// when typed and fails when set, which is the kind of difference nobody can
/// remember the shape of.
fn parse_due(phrase: &str, today: NaiveDate) -> Result<Due, AgentError> {
    let parsed = parse_quick_add(phrase, today, &Vocabulary::default());

    match parsed.due {
        // Words the date parser did not account for mean it read something
        // other than what was intended. Silently keeping the part it liked is
        // how a task ends up due on a day nobody chose.
        Some(_) if !parsed.title.trim().is_empty() => Err(AgentError::hinted(
            ErrorKind::BadDate,
            format!(
                "I did not understand `{}` in the date `{phrase}`.",
                parsed.title.trim()
            ),
            "Dates look like: friday, next monday 9am, 27th, in 3 days, every other week.",
        )),
        Some(due) => Ok(due),
        None => Err(AgentError::hinted(
            ErrorKind::BadDate,
            format!("`{phrase}` is not a date I understand."),
            "Dates look like: friday, next monday 9am, 27th, in 3 days, every other week.",
        )),
    }
}

/// A plain date, for a deadline. A deadline has no time and does not repeat.
fn parse_day(phrase: &str, today: NaiveDate) -> Result<NaiveDate, AgentError> {
    parse_date(phrase, today).ok_or_else(|| {
        AgentError::hinted(
            ErrorKind::BadDate,
            format!("`{phrase}` is not a date I understand."),
            "A deadline is a plain day: friday, 27th, next monday, in 3 days.",
        )
    })
}

/// Find the task someone meant.
///
/// Exact id, then exact title, then any title containing the text. Completed
/// tasks are considered throughout — "reopen the one I finished by mistake"
/// has to be able to name it — but lose to an open task when both fit, because
/// an instruction to change something is almost always about live work.
pub fn resolve_task(store: &Store, reference: &str) -> Result<TaskId, AgentError> {
    let wanted = reference.trim();

    if let Some(task) = store.tasks().iter().find(|task| task.id.as_str() == wanted) {
        return Ok(task.id.clone());
    }

    let exact: Vec<&Task> = store
        .tasks()
        .iter()
        .filter(|task| task.content.trim().eq_ignore_ascii_case(wanted))
        .collect();
    if !exact.is_empty() {
        return pick_task(store, wanted, &exact);
    }

    let lowered = wanted.to_lowercase();
    let partial: Vec<&Task> = store
        .tasks()
        .iter()
        .filter(|task| task.content.to_lowercase().contains(&lowered))
        .collect();
    if !partial.is_empty() {
        return pick_task(store, wanted, &partial);
    }

    Err(AgentError::hinted(
        ErrorKind::NotFound,
        format!("No task matches `{wanted}`."),
        format!("`planner agent search {wanted}` looks across projects and labels too."),
    ))
}

fn pick_task(store: &Store, wanted: &str, matches: &[&Task]) -> Result<TaskId, AgentError> {
    if let [only] = matches {
        return Ok(only.id.clone());
    }

    let open: Vec<&&Task> = matches.iter().filter(|task| !task.checked).collect();
    if let [only] = open.as_slice() {
        return Ok(only.id.clone());
    }

    Err(AgentError {
        kind: ErrorKind::Ambiguous,
        message: format!(
            "`{wanted}` matches {} tasks. Name one by its id.",
            matches.len()
        ),
        candidates: matches
            .iter()
            .map(|task| Candidate {
                id: task.id.to_string(),
                name: task.content.clone(),
                context: Some(task_context(store, task)),
            })
            .collect(),
        hint: None,
    })
}

/// Enough about a task to tell it from another of the same name.
fn task_context(store: &Store, task: &Task) -> String {
    let project = store
        .project(&task.project_id)
        .map(|project| project.name.as_str())
        .unwrap_or("unknown project");
    let state = if task.checked { ", completed" } else { "" };
    match task.due.as_ref() {
        Some(due) => format!("{project}, due {}{state}", view::due_phrase(due)),
        None => format!("{project}{state}"),
    }
}

/// Find the project someone meant. Id, then exact name, then a name containing
/// the text.
pub fn resolve_project(store: &Store, reference: &str) -> Result<ProjectId, AgentError> {
    let wanted = reference.trim();

    if let Some(project) = store
        .projects()
        .iter()
        .find(|project| project.id.as_str() == wanted)
    {
        return Ok(project.id.clone());
    }

    if let Some(project) = store.project_by_name(wanted) {
        return Ok(project.id.clone());
    }

    let lowered = wanted.to_lowercase();
    let partial: Vec<&Project> = store
        .projects()
        .iter()
        .filter(|project| project.name.to_lowercase().contains(&lowered))
        .collect();

    match partial.as_slice() {
        [only] => Ok(only.id.clone()),
        [] => Err(AgentError::hinted(
            ErrorKind::NotFound,
            format!("There is no project called `{wanted}`."),
            "`planner agent overview` lists the projects that exist. A project is not created \
             by naming one that does not.",
        )),
        many => Err(AgentError {
            kind: ErrorKind::Ambiguous,
            message: format!(
                "`{wanted}` matches {} projects. Name one exactly, or by its id.",
                many.len()
            ),
            candidates: many
                .iter()
                .map(|project| Candidate {
                    id: project.id.to_string(),
                    name: project.name.clone(),
                    context: None,
                })
                .collect(),
            hint: None,
        }),
    }
}

/// Find a section within one project.
fn resolve_section(
    store: &Store,
    project: &ProjectId,
    reference: &str,
) -> Result<SectionId, AgentError> {
    let wanted = reference.trim();
    let Some(owner) = store.project(project) else {
        return Err(AgentError::new(
            ErrorKind::NotFound,
            format!("There is no project `{project}` to look for `{wanted}` in."),
        ));
    };

    let sections = owner.sections_ordered();
    let found = sections
        .iter()
        .find(|section| section.name.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            let lowered = wanted.to_lowercase();
            sections
                .iter()
                .find(|section| section.name.to_lowercase().contains(&lowered))
        });

    match found {
        Some(section) => Ok(section.id.clone()),
        None if sections.is_empty() => Err(AgentError::hinted(
            ErrorKind::NotFound,
            format!("`{}` has no sections.", owner.name),
            "Sections are created in the app, not from here.",
        )),
        None => Err(AgentError::hinted(
            ErrorKind::NotFound,
            format!("`{}` has no section called `{wanted}`.", owner.name),
            format!(
                "Its sections are: {}.",
                sections
                    .iter()
                    .map(|section| section.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

#[cfg(test)]
mod tests;
