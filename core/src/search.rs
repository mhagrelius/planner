//! Finding things by typing part of their name.
//!
//! Substring matching with a score, not fuzzy subsequence matching. Typing
//! `plm` and being offered "Call the plumber" is impressive exactly once; for
//! the rest of the time it means a three-letter query matches most of the
//! list, and the thing you wanted is fourth. A prefix or word-start match is
//! what people actually type, and ranking those above a mid-word hit gets the
//! right answer to the top without the noise.

use super::store::Store;
use super::{LabelId, ProjectId, TaskId};

/// Something Quick Find can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    Task {
        id: TaskId,
        title: String,
        context: String,
    },
    Project {
        id: ProjectId,
        name: String,
    },
    Label {
        id: LabelId,
        name: String,
    },
}

impl Hit {
    /// What the row says.
    pub fn title(&self) -> &str {
        match self {
            Self::Task { title, .. } => title,
            Self::Project { name, .. } | Self::Label { name, .. } => name,
        }
    }

    /// The icon the row takes.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Task { .. } => "object-select-symbolic",
            Self::Project { .. } => "folder-symbolic",
            Self::Label { .. } => "user-bookmarks-symbolic",
        }
    }

    /// The dimmed line under the title.
    pub fn context(&self) -> String {
        match self {
            Self::Task { context, .. } => context.clone(),
            Self::Project { .. } => "Project".to_string(),
            Self::Label { .. } => "Label".to_string(),
        }
    }
}

/// How well `haystack` matches `needle`, higher being better.
///
/// `None` means no match at all. The ordering it produces, best first:
/// an exact match, then a prefix, then a match at the start of a word, then
/// anywhere. Shorter haystacks win ties, because a query that is most of a
/// short name is a better answer than the same query buried in a long one.
pub fn score(haystack: &str, needle: &str) -> Option<u32> {
    if needle.is_empty() {
        return Some(0);
    }
    let haystack_lower = haystack.to_lowercase();
    let needle_lower = needle.to_lowercase();
    let position = haystack_lower.find(&needle_lower)?;

    let base = if haystack_lower == needle_lower {
        1000
    } else if position == 0 {
        800
    } else if starts_a_word(&haystack_lower, position) {
        600
    } else {
        400
    };

    // Up to 100 back for brevity, so the bonus can never outweigh a better
    // class of match.
    let brevity = 100u32.saturating_sub(haystack.chars().count().min(100) as u32);
    Some(base + brevity)
}

/// Whether the character before `position` ends a word.
fn starts_a_word(haystack: &str, position: usize) -> bool {
    haystack[..position]
        .chars()
        .next_back()
        .is_some_and(|character| !character.is_alphanumeric())
}

/// Everything matching `query`, best first.
///
/// Completed tasks are included but rank below open ones: searching is often
/// how you find something you finished last week, and excluding them would
/// make Quick Find lie about what is in the store.
pub fn search(store: &Store, query: &str, limit: usize) -> Vec<Hit> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(u32, Hit)> = Vec::new();

    for project in store.projects() {
        if let Some(score) = score(&project.name, query) {
            scored.push((
                score,
                Hit::Project {
                    id: project.id.clone(),
                    name: project.name.clone(),
                },
            ));
        }
    }

    for label in store.labels() {
        if let Some(score) = score(&label.name, query) {
            scored.push((
                score,
                Hit::Label {
                    id: label.id.clone(),
                    name: label.name.clone(),
                },
            ));
        }
    }

    for task in store.tasks() {
        // The description matches too, but never scores as well as the title:
        // a hit you cannot see in the row is a confusing one.
        let title_score = score(&task.content, query);
        let body_score = score(&task.description, query).map(|score| score / 3);
        let Some(mut best) = title_score.or(body_score) else {
            continue;
        };
        if task.checked {
            best = best.saturating_sub(500);
        }

        let context = store
            .project(&task.project_id)
            .map(|project| project.name.clone())
            .unwrap_or_default();
        scored.push((
            best,
            Hit::Task {
                id: task.id.clone(),
                title: task.content.clone(),
                context: if task.checked {
                    format!("{context} · completed")
                } else {
                    context
                },
            },
        ));
    }

    // Sort by score, then by title, so equal scores come out in a stable
    // order rather than in whatever order the store happens to hold them.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.title().to_lowercase().cmp(&b.1.title().to_lowercase()))
    });
    scored.into_iter().take(limit).map(|(_, hit)| hit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::project::Project;
    use crate::task::Task;
    use chrono::{DateTime, Utc};

    fn now() -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 31)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
    }

    fn store() -> Store {
        let (store, _) = Store::open_at(
            tempfile::TempDir::new()
                .expect("a temporary directory")
                .keep()
                .join("planner.json"),
        );
        store
    }

    fn titles(store: &Store, query: &str) -> Vec<String> {
        search(store, query, 20)
            .into_iter()
            .map(|hit| hit.title().to_string())
            .collect()
    }

    #[test]
    fn a_better_class_of_match_always_outranks_a_worse_one() {
        let exact = score("plumber", "plumber").unwrap();
        let prefix = score("plumbers merchant", "plumb").unwrap();
        let word = score("call the plumber", "plumb").unwrap();
        let middle = score("unplumbed", "plumb").unwrap();

        assert!(exact > prefix, "exact beats prefix");
        assert!(prefix > word, "prefix beats a word start");
        assert!(word > middle, "a word start beats mid-word");
    }

    #[test]
    fn brevity_breaks_ties_but_never_beats_a_better_match() {
        let short = score("plumb", "plumb").unwrap();
        let long = score("plumbing and heating", "plumbing and heating").unwrap();
        assert!(short > long, "the shorter exact match wins");

        // But a long prefix match still loses to a short exact one, and a
        // short mid-word match does not overtake a long prefix match.
        let long_prefix = score("plumbing and heating supplies", "plumb").unwrap();
        let short_middle = score("aplumb", "plumb").unwrap();
        assert!(long_prefix > short_middle);
    }

    #[test]
    fn matching_ignores_case() {
        assert!(score("Call the Plumber", "plumber").is_some());
        assert!(score("call the plumber", "PLUMBER").is_some());
    }

    #[test]
    fn a_query_that_is_not_there_does_not_match() {
        assert_eq!(score("call the plumber", "electrician"), None);
        // And no subsequence matching: "plm" is not in "plumber".
        assert_eq!(score("plumber", "plm"), None);
    }

    #[test]
    fn an_empty_query_finds_nothing_rather_than_everything() {
        let mut store = store();
        store.add_task(Task::new(ProjectId::inbox(), "Anything", now()));
        assert!(titles(&store, "").is_empty());
        assert!(titles(&store, "   ").is_empty());
    }

    #[test]
    fn tasks_projects_and_labels_are_all_searched() {
        let mut store = store();
        store.add_project(Project::new("Plumbing", Color::Blue), now());
        store.label_for_name("plumber", now());
        store.add_task(Task::new(ProjectId::inbox(), "Call the plumber", now()));

        let found = titles(&store, "plumb");
        assert_eq!(found.len(), 3);
        assert!(found.contains(&"Plumbing".to_string()));
        assert!(found.contains(&"plumber".to_string()));
        assert!(found.contains(&"Call the plumber".to_string()));
    }

    #[test]
    fn a_description_match_ranks_below_a_title_match() {
        let mut store = store();
        store.add_task(Task::new(ProjectId::inbox(), "Leaking tap", now()));
        let id = store.add_task(Task::new(ProjectId::inbox(), "Ring someone", now()));
        store.task_mut(&id).unwrap().description = "about the leaking tap".into();

        assert_eq!(
            titles(&store, "leaking"),
            vec!["Leaking tap", "Ring someone"]
        );
    }

    #[test]
    fn completed_tasks_are_findable_but_rank_last() {
        let mut store = store();
        let done = store.add_task(Task::new(ProjectId::inbox(), "Call the plumber", now()));
        store.add_task(Task::new(
            ProjectId::inbox(),
            "Call the plumber again",
            now(),
        ));
        store.complete_task(
            &done,
            now(),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
        );

        let found = titles(&store, "plumber");
        assert_eq!(found.len(), 2, "a finished task is still findable");
        assert_eq!(
            found[0], "Call the plumber again",
            "the open one comes first"
        );
    }

    #[test]
    fn a_completed_task_says_so_in_its_context_line() {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Done thing", now()));
        store.complete_task(
            &id,
            now(),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
        );

        let hits = search(&store, "done", 10);
        assert!(hits[0].context().contains("completed"));
    }

    #[test]
    fn a_task_says_which_project_it_is_in() {
        let mut store = store();
        let work = store.add_project(Project::new("Work", Color::Blue), now());
        store.add_task(Task::new(work, "Draft the report", now()));

        let hits = search(&store, "draft", 10);
        assert_eq!(hits[0].context(), "Work");
    }

    #[test]
    fn the_limit_is_honoured() {
        let mut store = store();
        for index in 0..20 {
            store.add_task(Task::new(
                ProjectId::inbox(),
                format!("Task {index}"),
                now(),
            ));
        }
        assert_eq!(search(&store, "task", 5).len(), 5);
    }

    #[test]
    fn equal_scores_come_out_in_a_stable_order() {
        let mut store = store();
        for name in ["Beta", "Alpha", "Gamma"] {
            store.add_project(Project::new(name, Color::Blue), now());
        }
        // All three are exact-length prefix matches of "a"... only some are.
        let first = titles(&store, "a");
        let second = titles(&store, "a");
        assert_eq!(first, second, "the same query twice gives the same order");
    }
}
