//! Turning task titles into vectors, so "Repair the dripping tap" can find
//! "Sort out the leaking tap".
//!
//! The word comparison in [`crate::model::similar`] cannot retrieve a pair that
//! shares no words, and no amount of tuning will make it. This closes that hole
//! by ranking every task by sentence-embedding distance and handing the top few
//! to [`crate::model::duplicate`] alongside the lexical hits.
//!
//! # Rank, never threshold
//!
//! Measured on real task titles, the cosine *ordering* is reliable and the
//! *magnitude* is close to meaningless. Over a twenty-task list, the true
//! duplicate came first for every query tried, by a clear margin. But across
//! queries, "Pay invoice 42"/"Pay invoice 43" scores 0.96 and is not a
//! duplicate, "Call the plumber"/"Ring the plumber" scores 0.88 and is, and two
//! wholly unrelated errands score 0.75 because both are short imperative
//! sentences. Nothing here compares a cosine against a constant, and nothing
//! should start.
//!
//! # It is not in `planner-core`
//!
//! `planner-server` links that crate, and a container serving JSON has no
//! business carrying a neural network. The pure half — cosine, ranking — lives
//! there; the model lives here.
//!
//! # The model is a file the user supplies
//!
//! Off until installed on purpose, which is how sync and the API key already
//! work. Bundling it would put 87MB into every `.deb` and Flatpak for a feature
//! most people will not turn on, and half precision is not an option — f16
//! attention overflows to NaN on CPU, and is slower besides, CPU having no
//! native f16 arithmetic. Absent, everything degrades to the word comparison,
//! which is the whole feature for anyone who never reads this far.
//!
//! `packaging/fetch-embedding-model.sh` puts the three files where this looks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::Tokenizer;

use crate::model::similar::{rank_by_vector, SEMANTIC_TOP_K};
use crate::model::store::Store;
use crate::model::TaskId;

/// A title longer than this is truncated before embedding. The model's own
/// limit is 512 tokens; a task title that long is a description in disguise,
/// and the first few hundred characters decide the match anyway.
const MAX_CHARS: usize = 512;

/// Where the three files live: `$XDG_DATA_HOME/planner/model/`.
///
/// Beside the document rather than in the binary's prefix, so installing the
/// model does not need root and survives reinstalling the app.
pub fn model_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("planner")
        .join("model")
}

/// A loaded sentence-embedding model, and the vectors it has produced so far.
pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    /// Task id to (content it was embedded from, vector).
    ///
    /// Keyed on the content as well as the id so an edited title re-embeds
    /// rather than matching on what it used to say. Held in memory only: at
    /// 35ms per title a whole list costs a few seconds once per run, and a
    /// cache on disk is a second copy of the store to keep honest and to
    /// invalidate when the model changes.
    cache: RefCell<HashMap<TaskId, (String, Vec<f32>)>>,
}

impl Embedder {
    /// Load the model, or report why not.
    ///
    /// The error is for a log line, not a dialog. Someone adding a task cannot
    /// act on a missing tensor file.
    pub fn load() -> Result<Self, String> {
        let dir = model_dir();
        let weights = dir.join("model.safetensors");
        let config = dir.join("config.json");
        let vocabulary = dir.join("tokenizer.json");

        for path in [&weights, &config, &vocabulary] {
            if !path.exists() {
                return Err(format!("{} is not there", path.display()));
            }
        }

        let device = Device::Cpu;
        let config: Config = std::fs::read_to_string(&config)
            .map_err(|error| error.to_string())
            .and_then(|raw| serde_json::from_str(&raw).map_err(|error| error.to_string()))?;

        // Safety: mapping a file the user installed. A corrupt one is caught by
        // safetensors' own header validation and returns an error here.
        let builder = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, &device)
                .map_err(|error| error.to_string())?
        };
        let model = BertModel::load(builder, &config).map_err(|error| error.to_string())?;
        let tokenizer = Tokenizer::from_file(&vocabulary).map_err(|error| error.to_string())?;

        Ok(Self {
            model,
            tokenizer,
            device,
            cache: RefCell::new(HashMap::new()),
        })
    }

    /// Embed one string: mean-pool the token states, then scale to unit length
    /// so that a dot product is the cosine.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let text: String = text.chars().take(MAX_CHARS).collect();
        let encoded = self
            .tokenizer
            .encode(text.as_str(), true)
            .map_err(|error| error.to_string())?;

        let build = |values: &[u32]| {
            Tensor::new(values, &self.device)
                .and_then(|tensor| tensor.unsqueeze(0))
                .map_err(|error| error.to_string())
        };
        let ids = build(encoded.get_ids())?;
        let types = build(encoded.get_type_ids())?;
        let mask = build(encoded.get_attention_mask())?;

        let hidden = self
            .model
            .forward(&ids, &types, Some(&mask))
            .map_err(|error| error.to_string())?;

        let pooled = (|| {
            let (_batch, tokens, _width) = hidden.dims3()?;
            let pooled = (hidden.sum(1)? / tokens as f64)?.squeeze(0)?;
            let length = pooled.sqr()?.sum_all()?.sqrt()?.to_scalar::<f32>()?;
            // An all-zero vector would divide to NaN and poison every later
            // comparison. It should not happen, and costs one branch to rule out.
            if length <= f32::EPSILON {
                return Ok(vec![0.0; pooled.dims1()?]);
            }
            (pooled / length as f64)?.to_vec1::<f32>()
        })()
        .map_err(|error: candle_core::Error| error.to_string())?;

        Ok(pooled)
    }

    /// The vector for a task, computing and remembering it if need be.
    fn vector_for(&self, id: &TaskId, content: &str) -> Option<Vec<f32>> {
        if let Some((embedded, vector)) = self.cache.borrow().get(id) {
            if embedded == content {
                return Some(vector.clone());
            }
        }
        let vector = self.embed(content).ok()?;
        self.cache
            .borrow_mut()
            .insert(id.clone(), (content.to_string(), vector.clone()));
        Some(vector)
    }

    /// Embed one task that has no current vector. Returns whether any remain.
    ///
    /// One task per call, because this runs on the main loop and a title costs
    /// about 35ms — a whole list in one go would freeze the window for seconds.
    /// Driven by a timer at startup and after a sync, so by the time anyone
    /// opens quick-add the vectors are already there.
    pub fn warm_one(&self, store: &Store) -> bool {
        let stale = store.tasks().iter().find(|task| {
            self.cache
                .borrow()
                .get(&task.id)
                .map_or(true, |(embedded, _)| embedded != &task.content)
        });

        match stale {
            Some(task) => {
                self.vector_for(&task.id, &task.content);
                true
            }
            None => {
                // Nothing left to do: drop vectors for tasks that have since
                // been deleted, so a long session does not accumulate them.
                let live: std::collections::HashSet<&TaskId> =
                    store.tasks().iter().map(|task| &task.id).collect();
                self.cache.borrow_mut().retain(|id, _| live.contains(id));
                false
            }
        }
    }

    /// The tasks whose titles sit nearest `title`, nearest first.
    ///
    /// Ranks over vectors already computed and never embeds a task here — only
    /// the query, which is one 35ms call behind a 600ms debounce. A task whose
    /// vector has not been warmed yet is simply not a candidate this time; the
    /// word comparison still sees it, and the next keystroke usually will too.
    pub fn nearest(&self, store: &Store, title: &str, exclude: Option<&TaskId>) -> Vec<TaskId> {
        let Ok(query) = self.embed(title) else {
            return Vec::new();
        };

        let cache = self.cache.borrow();
        let tasks: Vec<&crate::model::Task> = store
            .tasks()
            .iter()
            .filter(|task| exclude != Some(&task.id))
            .collect();

        let vectors: Vec<(usize, Vec<f32>)> = tasks
            .iter()
            .enumerate()
            .filter_map(|(index, task)| match cache.get(&task.id) {
                // A vector computed from text the task no longer has is about
                // the old title, and would match on words since edited away.
                Some((embedded, vector)) if embedded == &task.content => {
                    Some((index, vector.clone()))
                }
                _ => None,
            })
            .collect();

        rank_by_vector(&query, &vectors, SEMANTIC_TOP_K)
            .into_iter()
            .map(|index| tasks[index].id.clone())
            .collect()
    }

    /// How many vectors are held, for tests.
    pub fn cached(&self) -> usize {
        self.cache.borrow().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_lives_beside_the_document_not_in_the_prefix() {
        // Installing it must not need root, and must survive reinstalling.
        let dir = model_dir();
        assert!(dir.ends_with("planner/model"), "{}", dir.display());
    }

    #[test]
    fn a_missing_model_is_a_message_rather_than_a_panic() {
        // The ordinary state for anyone who has not installed one. Everything
        // has to keep working, so this must never be a hard failure.
        let previous = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", "/nowhere/at/all");
        let outcome = Embedder::load();
        match previous {
            Some(value) => std::env::set_var("XDG_DATA_HOME", value),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }

        let error = outcome.err().expect("no model in /nowhere/at/all");
        assert!(error.contains("is not there"), "{error}");
    }
}
