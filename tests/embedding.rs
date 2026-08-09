//! Semantic duplicate retrieval, against the real model.
//!
//! Skipped when no model is installed, which is the ordinary state — see
//! `packaging/fetch-embedding-model.sh`. Running these needs the model on disk
//! and `XDG_DATA_HOME` pointing at it:
//!
//! ```sh
//! packaging/fetch-embedding-model.sh
//! cargo test --test embedding
//! ```
//!
//! Everything here is about *ranking*, never about a score clearing a bar.
//! That is the finding these exist to protect: measured on real task titles,
//! cosine magnitude carries almost no meaning — two unrelated errands score
//! 0.75 because both are short imperative sentences, and "Pay invoice 42"
//! against "Pay invoice 43" scores 0.96 and is not a duplicate. The ordering
//! within one query is what holds up, and that is all these assert.

use planner::model::id::ProjectId;
use planner::model::store::Store;
use planner::model::task::Task;
use planner::ui::embedding::Embedder;

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 8)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
}

fn store_with(titles: &[&str]) -> Store {
    let dir = tempfile::TempDir::new().expect("a temp dir");
    let (mut store, _) = Store::open_at(dir.keep().join("planner.json"));
    for title in titles {
        store.add_task(Task::new(ProjectId::inbox(), *title, now()));
    }
    store
}

/// The model, or `None` if this machine has not installed one.
fn embedder() -> Option<Embedder> {
    match Embedder::load() {
        Ok(embedder) => Some(embedder),
        Err(why) => {
            eprintln!("skipping: no embedding model installed ({why})");
            None
        }
    }
}

/// A realistic personal list, so ranking is tested against competition rather
/// than against one obviously-correct answer.
const LIST: &[&str] = &[
    "Call the plumber about the boiler",
    "Buy milk",
    "Renew the parking permit",
    "Email Sam about the lease",
    "Water the plants",
    "Book the dentist",
    "Pay invoice 42",
    "Submit the expenses claim",
    "Order a new laptop charger",
    "Cancel the gym membership",
    "Send Mum a birthday card",
    "Fix the bike puncture",
    "Read the quarterly report",
    "Clean the oven",
    "Book flights to Berlin",
    "Return the library books",
    "Update the CV",
    "Sort out the leaking tap",
    "Buy bread",
    "Schedule the car service",
];

/// Warm every vector, as the application's startup tick does.
fn warmed(embedder: &Embedder, store: &Store) {
    let mut guard = 0;
    while embedder.warm_one(store) {
        guard += 1;
        assert!(guard < 1000, "warm-up did not terminate");
    }
    assert_eq!(embedder.cached(), store.tasks().len());
}

#[test]
fn a_duplicate_sharing_no_words_is_retrieved() {
    let Some(embedder) = embedder() else { return };
    let store = store_with(LIST);
    warmed(&embedder, &store);

    // The case the word comparison cannot reach: "repair"/"sort out" and
    // "dripping"/"leaking" have nothing in common. This is what the model is
    // installed for.
    let nearest = embedder.nearest(&store, "Repair the dripping tap", None);
    let titles: Vec<&str> = nearest
        .iter()
        .filter_map(|id| store.task(id).map(|task| task.content.as_str()))
        .collect();

    assert_eq!(
        titles.first(),
        Some(&"Sort out the leaking tap"),
        "ranked: {titles:?}"
    );
}

#[test]
fn the_true_duplicate_ranks_first_across_several_phrasings() {
    let Some(embedder) = embedder() else { return };
    let store = store_with(LIST);
    warmed(&embedder, &store);

    for (typed, expected) in [
        (
            "Ring the plumber about the boiler",
            "Call the plumber about the boiler",
        ),
        (
            "Message Sam re the rental agreement",
            "Email Sam about the lease",
        ),
        ("Phone the dentist for an appointment", "Book the dentist"),
        ("Get milk", "Buy milk"),
    ] {
        let nearest = embedder.nearest(&store, typed, None);
        let first = nearest
            .first()
            .and_then(|id| store.task(id))
            .map(|task| task.content.clone());
        assert_eq!(first.as_deref(), Some(expected), "typing {typed:?}");
    }
}

#[test]
fn the_task_being_edited_is_never_its_own_nearest_neighbour() {
    let Some(embedder) = embedder() else { return };
    let store = store_with(LIST);
    warmed(&embedder, &store);

    let id = store
        .tasks()
        .iter()
        .find(|task| task.content == "Buy milk")
        .map(|task| task.id.clone())
        .expect("the seeded task");

    let nearest = embedder.nearest(&store, "Buy milk", Some(&id));
    assert!(!nearest.contains(&id), "a task matched itself");
}

#[test]
fn an_edited_title_is_re_embedded_rather_than_matched_on_what_it_used_to_say() {
    let Some(embedder) = embedder() else { return };
    let mut store = store_with(&["Call the plumber"]);
    warmed(&embedder, &store);

    let id = store.tasks()[0].id.clone();
    store.task_mut(&id).unwrap().content = "Book a holiday to Spain".into();

    // Stale vectors are excluded from ranking outright, so the old title
    // cannot match before the warm-up catches up.
    assert!(embedder
        .nearest(&store, "Ring the plumber", None)
        .is_empty());

    warmed(&embedder, &store);
    let nearest = embedder.nearest(&store, "Trip to Spain", None);
    assert_eq!(nearest.first(), Some(&id), "the new title should match");
}

#[test]
fn a_deleted_task_stops_being_a_candidate_and_its_vector_is_dropped() {
    let Some(embedder) = embedder() else { return };
    let mut store = store_with(&["Buy milk", "Call the plumber"]);
    warmed(&embedder, &store);
    assert_eq!(embedder.cached(), 2);

    let id = store.tasks()[0].id.clone();
    store.remove_task(&id, now());
    warmed(&embedder, &store);

    assert_eq!(embedder.cached(), 1, "the deleted task's vector was kept");
    assert!(!embedder
        .nearest(&store, "buy some milk", None)
        .contains(&id));
}

#[test]
fn embedding_is_deterministic_and_unit_length() {
    let Some(embedder) = embedder() else { return };

    let first = embedder.embed("Call the plumber").expect("a vector");
    let second = embedder.embed("Call the plumber").expect("a vector");
    assert_eq!(first, second, "the same text twice gave different vectors");

    // Unit length is what makes a dot product a cosine.
    let length: f32 = first.iter().map(|value| value * value).sum();
    assert!((length - 1.0).abs() < 1e-4, "length squared was {length}");
    assert!(
        first.iter().all(|value| value.is_finite()),
        "a NaN in the vector poisons every later comparison"
    );
}

#[test]
fn an_empty_or_enormous_title_does_not_panic() {
    let Some(embedder) = embedder() else { return };
    assert!(embedder.embed("").is_ok());
    // Longer than the model's context, to prove the truncation holds.
    assert!(embedder.embed(&"word ".repeat(4000)).is_ok());
}
