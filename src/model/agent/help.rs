//! What the surface says about itself.
//!
//! One table of verbs feeds three things: the prose help, the machine-readable
//! `describe` output, and the list of verbs the parser will accept. They cannot
//! drift apart, because there is nothing to drift — a verb that is not in this
//! table cannot be parsed, and one that is has documentation by construction.
//!
//! The help is written for a model rather than for a person at a terminal.
//! That mostly means the same things good help always meant — say what the
//! arguments are, show a real example — with one addition: every verb states
//! what it returns, because a caller deciding whether to run `show` after
//! `add` needs to know that `add` already gave it the task back.

use serde::Serialize;

/// One argument of one verb.
#[derive(Debug, Clone, Serialize)]
pub struct Argument {
    pub name: &'static str,
    pub required: bool,
    pub description: &'static str,
}

/// One verb, as both prose and schema.
#[derive(Debug, Clone, Serialize)]
pub struct Verb {
    pub name: &'static str,
    #[serde(skip_serializing_if = "<[&str]>::is_empty")]
    pub aliases: &'static [&'static str],
    pub usage: &'static str,
    pub summary: &'static str,
    /// Whether running it changes anything. A caller that gates writes behind
    /// an approval prompt reads this rather than keeping its own list.
    pub mutates: bool,
    pub arguments: &'static [Argument],
    /// What comes back, so a caller knows whether it needs a second call.
    pub returns: &'static str,
    #[serde(skip_serializing_if = "<[&str]>::is_empty")]
    pub examples: &'static [&'static str],
}

const TASK_REFERENCE: Argument = Argument {
    name: "task",
    required: true,
    description: "A task, named by its id or by any part of its title. A title \
                  that matches more than one open task comes back as an \
                  `ambiguous` error listing the candidates and their ids.",
};

/// Every verb, in the order the help lists them: read before write.
pub const VERBS: &[Verb] = &[
    Verb {
        // No `--help` alias: `GOption` intercepts that before this code runs,
        // and documenting a spelling that never reaches us would be a lie. The
        // launcher's own `--help` points back here instead.
        name: "help",
        aliases: &[],
        usage: "planner agent help [verb]",
        summary: "This text, or everything about one verb.",
        mutates: false,
        arguments: &[Argument {
            name: "verb",
            required: false,
            description: "The verb to explain. Omit it for the whole surface.",
        }],
        returns: "Plain text, not JSON.",
        examples: &["planner agent help update"],
    },
    Verb {
        name: "describe",
        aliases: &[],
        usage: "planner agent describe",
        summary: "Every verb as JSON, for generating tool definitions.",
        mutates: false,
        arguments: &[],
        returns: "`{verbs: [{name, usage, summary, mutates, arguments, returns}]}`.",
        examples: &["planner agent describe"],
    },
    Verb {
        name: "overview",
        aliases: &["projects"],
        usage: "planner agent overview",
        summary: "Projects, sections, labels, filters, and what is outstanding.",
        mutates: false,
        arguments: &[],
        returns: "`{projects, labels, filters, counts}`. Start here — it is the \
                  only call that shows what project and label names exist, which \
                  the other verbs expect you to use.",
        examples: &["planner agent overview"],
    },
    Verb {
        name: "list",
        aliases: &["tasks"],
        usage: "planner agent list [query] [limit=N]",
        summary: "The tasks matching a filter query.",
        mutates: false,
        arguments: &[
            Argument {
                name: "query",
                required: false,
                description: "A filter query (see FILTER QUERIES). Omitted, it \
                              lists every open task.",
            },
            Argument {
                name: "limit",
                required: false,
                description: "How many to return. Defaults to 50. The response \
                              always says how many matched, so a truncated list \
                              is visible rather than silent.",
            },
        ],
        returns: "`{tasks, count, matched, truncated}`. Completed tasks are \
                  excluded unless the query says `completed`.",
        examples: &[
            "planner agent list due: today | overdue",
            "planner agent list '#Work & p1'",
            "planner agent list 'no date' limit=10",
        ],
    },
    Verb {
        name: "show",
        aliases: &["task"],
        usage: "planner agent show <task>",
        summary: "One task in full, with its description and subtasks.",
        mutates: false,
        arguments: &[TASK_REFERENCE],
        returns: "`{task}`, including `description`, `reminders` and nested \
                  `subtasks` — none of which `list` returns.",
        examples: &["planner agent show 'Email Sam'"],
    },
    Verb {
        name: "search",
        aliases: &["find"],
        usage: "planner agent search <text> [limit=N]",
        summary: "Tasks, projects and labels whose names contain some text.",
        mutates: false,
        arguments: &[
            Argument {
                name: "text",
                required: true,
                description: "What to look for. Substring, case-insensitive.",
            },
            Argument {
                name: "limit",
                required: false,
                description: "How many to return. Defaults to 50.",
            },
        ],
        returns: "`{hits}`, each with a `kind` of task, project or label, and an \
                  id. Use this to turn a vague reference from the user into an \
                  id before calling a verb that changes something.",
        examples: &["planner agent search lease"],
    },
    Verb {
        name: "add",
        aliases: &["new"],
        usage: "planner agent add <line>",
        summary: "Create a task from a natural-language line.",
        mutates: true,
        arguments: &[Argument {
            name: "line",
            required: true,
            description: "A quick-add line (see QUICK-ADD LINES). Everything — \
                          project, labels, priority, date, repeat — goes in the \
                          one line rather than in separate fields.",
        }],
        returns: "`{task}`, the task as it was actually understood. Check it \
                  rather than assuming: the date and project came from parsing \
                  prose, so this is where a misread shows up.",
        examples: &[
            "planner agent add Email Sam about the lease #Work @email p2 friday 9am",
            "planner agent add Water the plants every other monday",
        ],
    },
    Verb {
        name: "subtask",
        aliases: &[],
        usage: "planner agent subtask <parent> <line>",
        summary: "Create a task underneath another one.",
        mutates: true,
        arguments: &[
            Argument {
                name: "parent",
                required: true,
                description: "The task to nest under, as ONE argument. Quote it \
                              if it has spaces — unlike the other verbs, there \
                              is a second argument after it.",
            },
            Argument {
                name: "line",
                required: true,
                description: "A quick-add line for the subtask.",
            },
        ],
        returns: "`{task}`. A subtask always shares its parent's project, \
                  whatever the line says.",
        examples: &["planner agent subtask 'Move house' Pack the books p1"],
    },
    Verb {
        name: "complete",
        aliases: &["done", "check"],
        usage: "planner agent complete <task>",
        summary: "Tick a task off.",
        mutates: true,
        arguments: &[TASK_REFERENCE],
        returns: "`{task, outcome}`. The outcome is `done`, `already-done`, or \
                  `completed-and-repeats` with a `next_due` date. The last one \
                  means the completion worked and the task repeats, so it is \
                  open again later — report it as done *and* say when it comes \
                  back, not as a reschedule. Completing a task completes its \
                  subtasks.",
        examples: &["planner agent complete 'Email Sam'"],
    },
    Verb {
        name: "reopen",
        aliases: &["uncomplete", "uncheck"],
        usage: "planner agent reopen <task>",
        summary: "Put a completed task back.",
        mutates: true,
        arguments: &[TASK_REFERENCE],
        returns: "`{task, reopened}`. `reopened` is false when the task was not \
                  ticked off to begin with, so a no-op does not read as an undo. \
                  Reopening a subtask reopens the parents above it, since an \
                  open task cannot sit inside a finished one.",
        examples: &["planner agent reopen 'Email Sam'"],
    },
    Verb {
        name: "delete",
        aliases: &["remove"],
        usage: "planner agent delete <task>",
        summary: "Delete a task and its subtasks.",
        mutates: true,
        arguments: &[TASK_REFERENCE],
        returns: "`{removed, count}`, every task that went, so you can report \
                  the subtasks that went with it. There is no undo from here — \
                  prefer `complete` when the user meant they had finished it.",
        examples: &["planner agent delete 'Email Sam'"],
    },
    Verb {
        name: "update",
        aliases: &["edit", "set"],
        usage: "planner agent update <task> <field=value> [field=value ...]",
        summary: "Change fields of an existing task.",
        mutates: true,
        arguments: &[
            TASK_REFERENCE,
            Argument {
                name: "title",
                required: false,
                description: "The title, as plain text. Quick-add tokens are not \
                              parsed here — set the date with `due=`.",
            },
            Argument {
                name: "description",
                required: false,
                description: "The long text under the title. Markdown.",
            },
            Argument {
                name: "due",
                required: false,
                description: "A date phrase: `friday`, `next monday 9am`, \
                              `in 3 days`, `every other week`. `due=none` clears \
                              it, which also stops it repeating.",
            },
            Argument {
                name: "deadline",
                required: false,
                description: "The hard date, as distinct from the day to work on \
                              it. A date phrase, or `none`.",
            },
            Argument {
                name: "priority",
                required: false,
                description: "`p1` to `p4`. p1 is most urgent; p4 means unset.",
            },
            Argument {
                name: "project",
                required: false,
                description: "Move it to another project, by name or id. Its \
                              subtasks go with it.",
            },
            Argument {
                name: "section",
                required: false,
                description: "File it under a section of its project, by name. \
                              `section=none` takes it out of one. Sections are \
                              created in the app, not from here.",
            },
            Argument {
                name: "pinned",
                required: false,
                description: "`true` or `false`.",
            },
            Argument {
                name: "add-label",
                required: false,
                description: "Put a label on it, creating the label if it is new.",
            },
            Argument {
                name: "remove-label",
                required: false,
                description: "Take a label off it. The label itself stays.",
            },
        ],
        returns: "`{task, applied}`. `applied` lists what actually changed, so a \
                  value that was already set is visibly a no-op rather than a \
                  silent one.",
        examples: &[
            "planner agent update 'Email Sam' due=next friday priority=p1",
            "planner agent update 'Email Sam' project=Work add-label=urgent",
        ],
    },
    Verb {
        name: "add-project",
        aliases: &["new-project"],
        usage: "planner agent add-project <name> [parent=<project>]",
        summary: "Create a project.",
        mutates: true,
        arguments: &[
            Argument {
                name: "name",
                required: true,
                description: "What to call it.",
            },
            Argument {
                name: "parent",
                required: false,
                description: "An existing project to nest it under.",
            },
        ],
        returns: "`{project}`, with the colour it was given.",
        examples: &["planner agent add-project Loft conversion parent=Home"],
    },
    Verb {
        name: "rename-project",
        aliases: &[],
        usage: "planner agent rename-project <project> <name>",
        summary: "Rename a project.",
        mutates: true,
        arguments: &[
            Argument {
                name: "project",
                required: true,
                description: "The project to rename, as ONE argument. Quote it \
                              if it has spaces.",
            },
            Argument {
                name: "name",
                required: true,
                description: "The new name.",
            },
        ],
        returns: "`{project}`.",
        examples: &["planner agent rename-project Work 'Work — 2026'"],
    },
    Verb {
        name: "remove-project",
        aliases: &["delete-project"],
        usage: "planner agent remove-project <project>",
        summary: "Delete a project, its subprojects, and every task in them.",
        mutates: true,
        arguments: &[Argument {
            name: "project",
            required: true,
            description: "The project to delete. The Inbox cannot be deleted.",
        }],
        returns: "`{name, projects, tasks}` — how much went. This deletes tasks \
                  the user may not have had in mind; confirm before calling it.",
        examples: &["planner agent remove-project 'Loft conversion'"],
    },
];

/// The canonical name of a verb, given a name or one of its aliases.
pub fn canonical_verb(word: &str) -> Option<&'static str> {
    VERBS
        .iter()
        .find(|verb| verb.name == word || verb.aliases.contains(&word))
        .map(|verb| verb.name)
}

/// Every verb's canonical name.
pub fn verb_names() -> Vec<&'static str> {
    VERBS.iter().map(|verb| verb.name).collect()
}

/// One verb by name or alias.
pub fn verb(word: &str) -> Option<&'static Verb> {
    VERBS
        .iter()
        .find(|verb| verb.name == word || verb.aliases.contains(&word))
}

/// The two mini-languages, written out once.
///
/// Both are the app's own: the same parser reads a quick-add line typed into
/// the dialog, and the same evaluator runs a query behind every view in the
/// sidebar. Documenting them here rather than inventing a separate set of
/// fields for the assistant is what keeps the two ways of using the planner
/// from being two different products.
const QUICK_ADD_HELP: &str = "\
QUICK-ADD LINES
  `add` and `subtask` take one line of prose. Tokens are recognised anywhere in
  it and removed from the title, so what is left is the title.

    #Project    file it in a project, by name. A #project that does not exist
                is NOT created — the task lands in the Inbox instead, where it
                is visible, rather than a misspelling becoming a new project.
    /Section    file it under a section of that project
    @label      put a label on it. A @label that does not exist IS created.
    p1 p2 p3 p4 priority. p1 is most urgent, p4 is none
    !30m        remind that long before it is due
    dates       today, tomorrow, fri, next friday, 27th, in 3 days,
                end of month, 9am, friday 9am
    repeats     every day, every other monday, every 3 weeks, every month,
                every weekday, every year until 1 Jan 2027, every day x5
                `every!` repeats from the day you complete it, not from the
                due date — use it for \"water the plants every 3 days\".

  Example: Email Sam about the lease #Work /Admin @email p2 friday 9am !30m
           becomes the task \"Email Sam about the lease\", in Work's Admin
           section, labelled email, priority 2, due 09:00 on Friday, with a
           reminder half an hour before.";

const QUERY_HELP: &str = "\
FILTER QUERIES
  `list` takes the same query language the app's own views are built from.

    due: today            deadline: friday      overdue
    due before: friday    due after: monday     no date
    p1                    @label                no labels
    #Project              ##Project (with its subprojects)     /Section
    pinned                recurring             subtask
    completed             search: some text     no deadline

  Combine with `&` (and), `|` (or), `!` (not) and parentheses.

    #Work & p1                      due: today | overdue
    ##Home & !p4 & @errand          (p1 | p2) & no date

  Completed tasks are left out unless the query says `completed`.";

/// The whole surface.
pub fn overview() -> String {
    let mut text = String::from(
        "\
planner agent — read and change tasks from a script or an assistant.

USAGE
  planner agent <verb> [arguments]

  Every verb prints one JSON object on stdout and exits 0, or prints
  {\"ok\": false, \"error\": ...} and exits 1. `help` prints text instead.

  Arguments are positional words and `key=value` pairs. There are no `--flags`:
  the launcher parses those before this code runs and would reject them.

  If Planner is running, the command is handed to it, so the window updates as
  you go and there is no second copy of the file to fall out of step. If it is
  not, the command reads and writes the file itself.

VERBS
",
    );

    let width = VERBS.iter().map(|verb| verb.name.len()).max().unwrap_or(0);
    for verb in VERBS {
        let mark = if verb.mutates { "*" } else { " " };
        text.push_str(&format!(
            "  {mark} {:width$}  {}\n",
            verb.name,
            first_sentence(verb.summary),
            width = width
        ));
    }
    text.push_str(
        "\n  * changes something. Everything else only reads.\n\n\
         Run `planner agent help <verb>` for arguments and examples, or\n\
         `planner agent describe` for the same thing as JSON.\n\n",
    );

    text.push_str(QUICK_ADD_HELP);
    text.push_str("\n\n");
    text.push_str(QUERY_HELP);
    text.push_str(
        "\n\nREFERRING TO A TASK\n  \
         By id, or by any part of its title. A title matching several open\n  \
         tasks is an `ambiguous` error listing them with their ids, rather\n  \
         than a guess — use `search` first when the user was vague, and pass\n  \
         the id when you already have it.\n",
    );
    text
}

/// One verb, at length.
pub fn for_verb(name: &str) -> Option<String> {
    let verb = verb(name)?;

    let mut text = format!(
        "{}\n\n{}\n\nUSAGE\n  {}\n",
        verb.name,
        wrap(verb.summary, "  "),
        verb.usage
    );

    if !verb.aliases.is_empty() {
        text.push_str(&format!("\nALSO CALLED\n  {}\n", verb.aliases.join(", ")));
    }

    if !verb.arguments.is_empty() {
        text.push_str("\nARGUMENTS\n");
        for argument in verb.arguments {
            let required = if argument.required {
                "required"
            } else {
                "optional"
            };
            text.push_str(&format!(
                "  {} ({})\n{}\n",
                argument.name,
                required,
                wrap(argument.description, "      ")
            ));
        }
    }

    text.push_str(&format!("\nRETURNS\n{}\n", wrap(verb.returns, "  ")));

    if verb.mutates {
        text.push_str("\n  This changes the store.\n");
    }

    if !verb.examples.is_empty() {
        text.push_str("\nEXAMPLES\n");
        for example in verb.examples {
            text.push_str(&format!("  {example}\n"));
        }
    }

    if matches!(verb.name, "add" | "subtask") {
        text.push('\n');
        text.push_str(QUICK_ADD_HELP);
        text.push('\n');
    }
    if verb.name == "list" {
        text.push('\n');
        text.push_str(QUERY_HELP);
        text.push('\n');
    }
    Some(text)
}

/// Take the first sentence of a summary, for the one-line verb table.
fn first_sentence(summary: &str) -> String {
    let collapsed = collapse(summary);
    match collapsed.find(". ") {
        Some(end) => collapsed[..=end].to_string(),
        None => collapsed,
    }
}

/// Squash the line breaks and indentation a `&'static str` in source carries.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rewrap prose to fit a terminal, under a fixed indent.
fn wrap(text: &str, indent: &str) -> String {
    const WIDTH: usize = 78;

    let mut lines = Vec::new();
    let mut line = String::from(indent);
    for word in collapse(text).split(' ') {
        if line.len() > indent.len() && line.len() + 1 + word.len() > WIDTH {
            lines.push(std::mem::replace(&mut line, String::from(indent)));
        } else if line.len() > indent.len() {
            line.push(' ');
        }
        line.push_str(word);
    }
    lines.push(line);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verb_is_documented_well_enough_to_be_used() {
        for verb in VERBS {
            assert!(!verb.summary.is_empty(), "{} has no summary", verb.name);
            assert!(
                !verb.returns.is_empty(),
                "{} says nothing about what it returns",
                verb.name
            );
            assert!(
                verb.usage.starts_with("planner agent "),
                "{} shows a usage line that cannot be run: {}",
                verb.name,
                verb.usage
            );
            // A verb with required arguments and no example is the one people
            // get wrong, so the example is not optional.
            if verb.arguments.iter().any(|argument| argument.required) {
                assert!(!verb.examples.is_empty(), "{} has no example", verb.name);
            }
        }
    }

    #[test]
    fn no_two_verbs_answer_to_the_same_word() {
        let mut seen = std::collections::HashSet::new();
        for verb in VERBS {
            for word in std::iter::once(&verb.name).chain(verb.aliases) {
                assert!(seen.insert(*word), "`{word}` names more than one verb");
            }
        }
    }

    #[test]
    fn an_alias_finds_the_verb_it_stands_for() {
        assert_eq!(canonical_verb("done"), Some("complete"));
        assert_eq!(canonical_verb("complete"), Some("complete"));
        assert_eq!(canonical_verb("nonsense"), None);
    }

    #[test]
    fn the_overview_lists_every_verb_and_both_languages() {
        let text = overview();
        for verb in VERBS {
            assert!(
                text.contains(verb.name),
                "{} is missing from the help",
                verb.name
            );
        }
        assert!(text.contains("QUICK-ADD LINES"));
        assert!(text.contains("FILTER QUERIES"));
    }

    #[test]
    fn a_verbs_own_help_carries_the_language_it_needs() {
        let add = for_verb("add").expect("add is documented");
        assert!(
            add.contains("QUICK-ADD LINES"),
            "add needs the quick-add syntax"
        );
        let list = for_verb("list").expect("list is documented");
        assert!(
            list.contains("FILTER QUERIES"),
            "list needs the query syntax"
        );
        // And a verb that needs neither is not padded with both.
        let overview = for_verb("overview").expect("overview is documented");
        assert!(!overview.contains("QUICK-ADD LINES"));
    }

    #[test]
    fn help_for_something_that_is_not_a_verb_is_absent_rather_than_empty() {
        assert!(for_verb("frobnicate").is_none());
    }

    #[test]
    fn the_verb_table_fits_in_a_terminal() {
        // The table is the first thing anyone reads. A summary long enough to
        // wrap turns the column of verbs into a wall.
        for line in overview()
            .lines()
            .take_while(|line| !line.contains("QUICK-ADD"))
        {
            assert!(line.len() <= 88, "{} wide: {line:?}", line.len());
        }
    }

    #[test]
    fn wrapped_prose_stays_inside_a_terminal() {
        let long = "a ".repeat(200);
        for line in wrap(&long, "    ").lines() {
            assert!(line.len() <= 78, "{line:?} is {} wide", line.len());
        }
    }
}
