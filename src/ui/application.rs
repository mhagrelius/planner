//! The application: owns the store, the window, and the save tick.
//!
//! Everything that mutates a task funnels through here. Windows and rows emit
//! signals describing what the user did; this decides what that means and
//! writes it down. Nothing else calls `save`, so there is exactly one place
//! where a task can be lost.
//!
//! **Saving is coalesced.** A two-second tick flushes the store if anything
//! changed, so typing never blocks on I/O. A hard crash costs at most a couple
//! of seconds of edits, and the write itself is atomic, so it cannot destroy
//! what was already there.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use std::cell::{Cell, RefCell};
use std::time::Duration;

use crate::model::id::TaskId;
use crate::model::schedule::Schedule;
use crate::model::store::{LoadOutcome, Store};
use crate::model::{Due, ViewStyle};
use crate::ui::window::PlannerWindow;

/// How often the store is flushed if it is dirty.
const TICK: Duration = Duration::from_secs(2);

/// A backstop pass, for when nothing else has triggered one.
///
/// Not the mechanism, just the floor. Incoming changes arrive because the
/// server holds a request open until there are some, and outgoing ones go as
/// soon as an edit settles; this only covers the cases neither can see — a
/// machine that has just woken from suspend holding a dead socket, or a
/// notification lost while the database was restarting.
const SYNC_TICK: Duration = Duration::from_secs(180);

/// How long after the last edit before it is pushed.
///
/// Long enough that typing a title is one push rather than thirty, short
/// enough that putting the laptop down and picking up the phone works. The
/// save tick is two seconds and this deliberately sits just behind it, so what
/// gets sent is what got written.
const SYNC_AFTER_EDIT: Duration = Duration::from_secs(3);

/// How many passes must fail before the window says anything.
///
/// One failure is a NAS asleep or a laptop changing networks. A banner for each
/// would teach the user to ignore banners, which costs more than the warning is
/// worth.
const SYNC_FAILURES_BEFORE_SAYING_SO: u32 = 3;

mod imp {
    use super::*;

    pub struct PlannerApplication {
        pub store: RefCell<Store>,
        pub dirty: Cell<bool>,
        /// The last save error, if the file cannot be written. Surfaced as a
        /// banner rather than a toast: losing edits is an ongoing condition,
        /// not an event, and a toast is missed while you are typing.
        pub save_error: RefCell<Option<String>>,
        pub load_outcome: RefCell<Option<LoadOutcome>>,
        pub tick: RefCell<Option<glib::SourceId>>,
        pub schedule: RefCell<Schedule>,
        pub reminder_tick: RefCell<Option<glib::SourceId>>,

        // --- semantic duplicate search ----------------------------------
        /// The embedding model, once loaded. `Rc` because the warm-up timer
        /// and every quick-add dialog share the one cache.
        pub embedder: RefCell<Option<std::rc::Rc<crate::ui::embedding::Embedder>>>,
        /// Loading is attempted once. Without this a missing model would be
        /// re-looked-for on every keystroke.
        pub embedder_tried: Cell<bool>,
        /// The tick embedding existing tasks a few at a time.
        pub warmup: RefCell<Option<glib::SourceId>>,

        // --- sync -------------------------------------------------------
        /// Where to sync to, if the config named somewhere.
        pub sync_target: RefCell<Option<(String, String)>>,
        /// What this machine and the server last agreed on.
        pub sync_base: RefCell<crate::model::sync::Snapshot>,
        pub sync_tick: RefCell<Option<glib::SourceId>>,
        /// Set while a pass is in flight, so a slow server cannot have two
        /// running at once and push the same records twice.
        pub syncing: Cell<bool>,
        /// The last sync failure. A banner rather than a toast, and only once
        /// it has persisted — see [`PlannerApplication::report_sync_failure`].
        pub sync_error: RefCell<Option<String>>,
        /// How many passes in a row have failed.
        pub sync_failures: Cell<u32>,
        /// The cursor the server last handed out, sent back when waiting.
        pub sync_cursor: Cell<Option<chrono::DateTime<chrono::Utc>>>,
        /// A pending push, debounced so a burst of typing is one pass.
        pub sync_soon: RefCell<Option<glib::SourceId>>,
        /// When the last pass finished without an error.
        pub sync_last_pass: Cell<Option<chrono::DateTime<chrono::Utc>>>,
        /// Why the last pass failed, from the first failure rather than the
        /// third. The banner waits for a run of them; the status dialog is
        /// where you go to ask, so it answers straight away.
        pub sync_last_failure: RefCell<Option<String>>,
    }

    impl Default for PlannerApplication {
        fn default() -> Self {
            // A store that refuses to save, replaced by `startup` with the
            // real one. Holding a `Store` rather than an `Option<Store>` means
            // no code path has to cope with "there is no store yet".
            Self {
                store: RefCell::new(Store::detached()),
                dirty: Cell::new(false),
                save_error: RefCell::new(None),
                load_outcome: RefCell::new(None),
                tick: RefCell::new(None),
                schedule: RefCell::new(Schedule::new()),
                reminder_tick: RefCell::new(None),
                embedder: RefCell::new(None),
                embedder_tried: Cell::new(false),
                warmup: RefCell::new(None),
                sync_target: RefCell::new(None),
                sync_base: RefCell::new(Default::default()),
                sync_tick: RefCell::new(None),
                syncing: Cell::new(false),
                sync_error: RefCell::new(None),
                sync_failures: Cell::new(0),
                sync_cursor: Cell::new(None),
                sync_soon: RefCell::new(None),
                sync_last_pass: Cell::new(None),
                sync_last_failure: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PlannerApplication {
        const NAME: &'static str = "PlannerApplication";
        type Type = super::PlannerApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for PlannerApplication {}

    impl ApplicationImpl for PlannerApplication {
        fn startup(&self) {
            // Chain up first: the toolkit initialises in the parent handler,
            // and anything touching GTK before it runs is undefined.
            self.parent_startup();

            let obj = self.obj();
            if let Some(display) = gtk::gdk::Display::default() {
                crate::ui::load_stylesheet(&display);
            }
            obj.load_store();
            obj.install_actions();
            obj.start_tick();
            obj.start_reminders();
            obj.start_sync();
        }

        fn activate(&self) {
            self.parent_activate();
            let obj = self.obj();
            let window = obj
                .active_window()
                .and_downcast::<PlannerWindow>()
                .unwrap_or_else(|| PlannerWindow::new(&obj));
            window.present();
        }

        /// Handle a launch, including a second one that hands its arguments
        /// to the instance already running.
        fn command_line(&self, command_line: &gio::ApplicationCommandLine) -> glib::ExitCode {
            let obj = self.obj();

            // `agent` is a command, not a launch. It is handled here, before
            // anything is presented, precisely so that it *is* the running
            // instance that answers when there is one: the store lives in that
            // process's memory and a second process writing the file behind it
            // would be overwritten by its next save.
            let arguments: Vec<String> = command_line
                .arguments()
                .iter()
                .skip(1)
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            if arguments.first().is_some_and(|word| word == "agent") {
                return obj.run_agent(command_line, &arguments[1..]);
            }

            obj.activate();

            let wants_quick_add = command_line
                .options_dict()
                .lookup_value("new-task", None)
                .is_some();
            if wants_quick_add {
                if let Some(window) = obj.active_window() {
                    // The window has just been presented but not necessarily
                    // mapped; deferring to the next idle means the dialog has
                    // something to attach to.
                    glib::idle_add_local_once(move || {
                        let _ = window.activate_action("win.new-task", None);
                    });
                }
            }
            glib::ExitCode::SUCCESS
        }

        fn shutdown(&self) {
            // The tick will not fire again, so the last edits are written here
            // or not at all.
            self.obj().save_now();
            if let Some(id) = self.tick.take() {
                id.remove();
            }
            if let Some(id) = self.reminder_tick.take() {
                id.remove();
            }
            self.parent_shutdown();
        }
    }

    impl GtkApplicationImpl for PlannerApplication {}
    impl AdwApplicationImpl for PlannerApplication {}
}

glib::wrapper! {
    pub struct PlannerApplication(ObjectSubclass<imp::PlannerApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Default for PlannerApplication {
    fn default() -> Self {
        Self::new()
    }
}

impl PlannerApplication {
    pub fn new() -> Self {
        Self::with_application_id(crate::APP_ID)
    }

    /// Build an application under a given ID.
    ///
    /// Tests use their own: sharing the real one would register this process
    /// as a *remote* for a running Planner, and it would drive the live app
    /// instead of itself.
    pub fn with_application_id(application_id: &str) -> Self {
        let app: Self = glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", gio::ApplicationFlags::HANDLES_COMMAND_LINE)
            .build();
        app.add_main_option(
            "new-task",
            glib::Char::from(b'n'),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            "Open the quick-add dialog",
            None,
        );

        // `--help` is answered by GOption before any of this crate's code
        // runs, so it is the one page a caller is guaranteed to be able to
        // reach by guessing. It therefore has to say where the rest is.
        app.set_option_context_parameter_string(Some("[agent VERB ...]"));
        app.set_option_context_summary(Some(
            "Run with no arguments to open the window.\n\n\
             `planner agent VERB` reads and changes tasks from a script or an \
             assistant, printing JSON.\nIt is answered by the running Planner \
             when there is one, so the window stays in step.",
        ));
        app.set_option_context_description(Some(
            "Start with:\n  \
             planner agent help          every verb, and the two mini-languages\n  \
             planner agent describe      the same thing as JSON\n  \
             planner agent overview      the projects, labels and counts that exist",
        ));
        app
    }

    /// Run an agent command against this application's store.
    ///
    /// Returns what to print and whether it worked. Split from the command
    /// line itself so the part with the rules in it — borrow, save, redraw —
    /// can be tested by driving the real application, which constructing a
    /// `gio::ApplicationCommandLine` by hand does not allow.
    ///
    /// The store is borrowed for exactly as long as the command takes and the
    /// borrow is dropped before anything redraws: a refresh reads the store,
    /// and re-entering a live `borrow_mut` aborts the process.
    pub fn agent_command(&self, arguments: &[String]) -> (String, bool) {
        use crate::model::agent;

        let now = chrono::Utc::now();
        let today = crate::ui::today();

        let result = agent::parse(arguments).and_then(|command| {
            let mut store = self.imp().store.borrow_mut();
            agent::execute(&mut store, command, now, today)
        });

        // Written out now rather than left to the save tick. A caller has been
        // told the change happened, and "it is in the memory of a process you
        // cannot see" is not that.
        if result
            .as_ref()
            .is_ok_and(agent::Response::changed_the_store)
        {
            self.imp().dirty.set(true);
            self.save_now();
            self.refresh();
        }

        (agent::render(&result), result.is_ok())
    }

    /// Answer an `agent` command line.
    fn run_agent(
        &self,
        command_line: &gio::ApplicationCommandLine,
        arguments: &[String],
    ) -> glib::ExitCode {
        let (output, ok) = self.agent_command(arguments);

        // `print_literal` goes back to the process that ran the command, which
        // is what makes this work when the answer came from a different one.
        command_line.print_literal(&format!("{output}\n"));

        if ok {
            glib::ExitCode::SUCCESS
        } else {
            glib::ExitCode::FAILURE
        }
    }

    // --- the store ------------------------------------------------------

    /// Read the store from disk. Called once, from `startup`.
    fn load_store(&self) {
        let (store, outcome) = Store::open();
        self.imp().store.replace(store);
        self.imp().load_outcome.replace(Some(outcome));
    }

    /// Do something with the store, then mark it as needing a save.
    ///
    /// Every mutation goes through here so that no caller can change a task
    /// and forget to flag it. The borrow is dropped before the callback's
    /// result is returned, so a handler that refreshes the window — which
    /// reads the store — cannot re-enter a live `borrow_mut` and abort.
    pub fn mutate<T>(&self, change: impl FnOnce(&mut Store) -> T) -> T {
        let result = {
            let mut store = self.imp().store.borrow_mut();
            change(&mut store)
        };
        self.imp().dirty.set(true);
        // The one place anything changes is the one place worth telling the
        // other machines from. Debounced, and a no-op when sync is off.
        self.sync_after_edit();
        result
    }

    /// Read the store.
    ///
    /// Same rule in the other direction: the borrow lives only as long as the
    /// callback, so nothing can hold a reference across a mutation.
    pub fn with_store<T>(&self, read: impl FnOnce(&Store) -> T) -> T {
        let store = self.imp().store.borrow();
        read(&store)
    }

    /// What happened when the store was opened, taken once.
    ///
    /// Taken rather than read so the window shows a recovery notice at the
    /// moment it opens and never again.
    pub fn take_load_outcome(&self) -> Option<LoadOutcome> {
        self.imp().load_outcome.take()
    }

    /// The current save failure, if the file cannot be written.
    pub fn save_error(&self) -> Option<String> {
        self.imp().save_error.borrow().clone()
    }

    fn start_tick(&self) {
        let id = glib::timeout_add_local(
            TICK,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    app.save_now();
                    glib::ControlFlow::Continue
                }
            ),
        );
        self.imp().tick.replace(Some(id));
    }

    /// The sentence-embedding model, loaded on first use.
    ///
    /// `None` forever if there is no model installed, which is the ordinary
    /// state — loading is attempted once and the outcome remembered, so a
    /// missing file does not cost a directory walk per keystroke.
    pub fn embedder(&self) -> Option<std::rc::Rc<crate::ui::embedding::Embedder>> {
        let imp = self.imp();
        if let Some(embedder) = imp.embedder.borrow().as_ref() {
            return Some(embedder.clone());
        }
        if imp.embedder_tried.get() {
            return None;
        }
        imp.embedder_tried.set(true);

        match crate::ui::embedding::Embedder::load() {
            Ok(embedder) => {
                let embedder = std::rc::Rc::new(embedder);
                imp.embedder.replace(Some(embedder.clone()));
                self.start_embedding_warmup();
                Some(embedder)
            }
            Err(why) => {
                // Not a banner. Someone adding a task cannot act on this, and
                // the word comparison covers for it.
                eprintln!("planner: semantic duplicate search is off ({why})");
                None
            }
        }
    }

    /// Embed the existing tasks a few at a time, off the critical path.
    ///
    /// A title costs about 35ms and the whole list must not be done at once —
    /// that is seconds of frozen window. One per tick keeps the main loop
    /// responsive, and the timer stops itself when everything is covered.
    fn start_embedding_warmup(&self) {
        if self.imp().warmup.borrow().is_some() {
            return;
        }
        let id = glib::timeout_add_local(
            Duration::from_millis(10),
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    let Some(embedder) = app.imp().embedder.borrow().clone() else {
                        return glib::ControlFlow::Break;
                    };
                    if app.with_store(|store| embedder.warm_one(store)) {
                        glib::ControlFlow::Continue
                    } else {
                        app.imp().warmup.replace(None);
                        glib::ControlFlow::Break
                    }
                }
            ),
        );
        self.imp().warmup.replace(Some(id));
    }

    /// Bring the vectors back up to date after tasks arrive or change.
    pub fn rewarm_embeddings(&self) {
        if self.imp().embedder.borrow().is_some() {
            self.start_embedding_warmup();
        }
    }

    /// The Anthropic API key, if the config names one.
    ///
    /// Read on each quick-add rather than cached at startup, so writing the key
    /// into the config file takes effect on the next dialog rather than the
    /// next launch. The file is a few hundred bytes and this happens once per
    /// dialog, which is not a rate worth caching against.
    pub fn anthropic_key(&self) -> Option<String> {
        crate::model::Config::load()
            .anthropic_key()
            .map(str::to_string)
    }

    // --- sync -----------------------------------------------------------

    /// Read the config and, if it names a server, start the sync tick.
    ///
    /// Sync is off until a URL and a token are written on purpose. There is
    /// nothing sensible for the app to guess at here — unlike the document,
    /// whose location it can work out — so a missing config is simply a
    /// planner that does not sync.
    fn start_sync(&self) {
        let config = crate::model::Config::load();
        let Some((url, token)) = config.sync_target() else {
            return;
        };
        self.imp()
            .sync_target
            .replace(Some((url.to_string(), token.to_string())));

        let base_path = crate::model::sync::default_base_path(
            &self.with_store(|store| store.path().to_owned()),
        );
        self.imp()
            .sync_base
            .replace(crate::model::sync::load_base(&base_path));

        // A first pass shortly after startup, then every few minutes. Nothing
        // on screen is waiting for it, and a planner is not a chat window.
        let id = glib::timeout_add_local(
            SYNC_TICK,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    app.sync_now();
                    glib::ControlFlow::Continue
                }
            ),
        );
        self.imp().sync_tick.replace(Some(id));
        self.sync_now();
    }

    /// Run one pass: network on a worker, every local write back here.
    ///
    /// **No code below this opens the planner file.** The worker is handed a
    /// snapshot and the bodies it needs, and gives back records; the merge goes
    /// through `mutate`, which sets the dirty flag and lets the ordinary save
    /// tick do the writing. Two writers over one document is the failure this
    /// shape exists to prevent.
    pub fn sync_now(&self) {
        let Some((url, token)) = self.imp().sync_target.borrow().clone() else {
            return;
        };
        // A server slow enough to overlap two passes would otherwise be sent
        // the same records twice.
        if self.imp().syncing.get() {
            return;
        }

        // Everything the worker needs, taken here while the store is still.
        let base = self.imp().sync_base.borrow().clone();
        let (local, bodies) = self.with_store(|store| {
            let local = crate::model::sync::snapshot_of(store);
            let bodies: std::collections::BTreeMap<_, _> = local
                .keys()
                .filter_map(|key| {
                    store
                        .record_body(key.kind, &key.id)
                        .map(|body| (key.clone(), body))
                })
                .collect();
            (local, bodies)
        });

        self.imp().syncing.set(true);

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = crate::ui::sync::HttpRemote::new(&url, token).and_then(|remote| {
                crate::model::sync::gather(&remote, &base, &local, |key| bodies.get(key).cloned())
            });
            let _ = sender.send(outcome);
        });

        // Poll for the worker's answer on the main loop rather than touching a
        // widget from the thread. `glib::idle_add_local` would spin; a short
        // timer costs nothing and the pass is not urgent.
        glib::timeout_add_local(
            Duration::from_millis(250),
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || match receiver.try_recv() {
                    Ok(outcome) => {
                        app.imp().syncing.set(false);
                        app.finish_sync(outcome);
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    // The worker died without answering. Nothing was applied,
                    // so the next pass simply tries again.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        app.imp().syncing.set(false);
                        glib::ControlFlow::Break
                    }
                }
            ),
        );
    }

    /// Apply what a pass brought back. Main thread, by construction.
    fn finish_sync(
        &self,
        outcome: Result<crate::model::sync::Incoming, crate::model::sync::SyncError>,
    ) {
        let incoming = match outcome {
            Ok(incoming) => incoming,
            Err(error) => return self.report_sync_failure(error.to_string()),
        };

        // A pull landing on the task open in the detail panel would take the
        // text out from under the cursor. Held back, and left out of the base,
        // so the next pass offers it again once the panel has moved on.
        let open = self
            .active_window()
            .and_downcast::<PlannerWindow>()
            .and_then(|window| window.task_in_detail_panel());
        let held = move |key: &crate::model::sync::Key| {
            open.as_ref().is_some_and(|id| {
                key.kind == crate::model::RecordKind::Task && key.id == id.as_str()
            })
        };

        // Held across the merge so `mutate` does not read it as a local edit
        // and push back what just arrived.
        self.imp().syncing.set(true);
        let now = chrono::Utc::now();
        let (report, base) =
            self.mutate(|store| crate::model::sync::apply(store, incoming, &held, now));
        self.imp().syncing.set(false);

        self.imp().sync_base.replace(base.clone());
        let path = crate::model::sync::default_base_path(
            &self.with_store(|store| store.path().to_owned()),
        );
        if let Err(error) = crate::model::sync::save_base(&base, &path) {
            eprintln!("planner: could not record what was synced: {error}");
        }

        self.imp().sync_failures.set(0);
        self.imp().sync_last_pass.set(Some(now));
        self.imp().sync_last_failure.replace(None);
        if self.imp().sync_error.borrow_mut().take().is_some() {
            self.notify_save_state();
        }

        // Nothing at all on a clean pass. Sync is awareness, not applause.
        if !report.is_empty() {
            self.refresh();
        }

        self.wait_for_changes();
    }

    /// Park a worker on the server until another machine writes.
    ///
    /// This is what makes an edit on the Mac show up here in about as long as
    /// the network takes rather than on the next timer. The request costs a
    /// socket and a thread; the server answers it when something changes, or
    /// gives up after fifty seconds and gets asked again.
    fn wait_for_changes(&self) {
        let Some((url, token)) = self.imp().sync_target.borrow().clone() else {
            return;
        };
        if self.imp().syncing.get() {
            return;
        }
        self.imp().syncing.set(true);

        // The server's cursor, not one this machine invented: a change that
        // landed between the pass and this call comes straight back rather
        // than being waited through.
        let since = self
            .imp()
            .sync_cursor
            .get()
            .unwrap_or(chrono::DateTime::UNIX_EPOCH);

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use crate::model::sync::Remote;
            let outcome = crate::ui::sync::HttpRemote::new(&url, token)
                .and_then(|remote| remote.wait_for_change(since));
            let _ = sender.send(outcome);
        });

        glib::timeout_add_local(
            Duration::from_millis(500),
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || match receiver.try_recv() {
                    Ok(Ok((changed, cursor))) => {
                        app.imp().syncing.set(false);
                        app.imp().sync_cursor.set(Some(cursor));
                        // Changed: go and get it. Timed out: ask again, which
                        // a pass does at the end of `finish_sync`.
                        if changed {
                            app.sync_now();
                        } else {
                            app.wait_for_changes();
                        }
                        glib::ControlFlow::Break
                    }
                    // The wait failed. Say nothing and leave it to the
                    // backstop tick — a NAS that went away is exactly what
                    // that tick is for, and a banner per failed wait would be
                    // one a minute.
                    Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        app.imp().syncing.set(false);
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                }
            ),
        );
    }

    /// Something changed here; get it to the other machines.
    ///
    /// Debounced rather than immediate: typing a title fires this on every
    /// keystroke, and thirty passes for one task would be thirty round trips
    /// to say the same thing.
    fn sync_after_edit(&self) {
        if self.imp().sync_target.borrow().is_none() {
            return;
        }
        // Applying a pull goes through `mutate` like everything else. Pushing
        // back what just arrived would be a round trip to tell the server
        // something it told us.
        if self.imp().syncing.get() {
            return;
        }
        // Restart the clock, so a burst of edits is one push at the end of it
        // rather than one at the start.
        if let Some(pending) = self.imp().sync_soon.take() {
            pending.remove();
        }

        let id = glib::timeout_add_local_once(
            SYNC_AFTER_EDIT,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move || {
                    app.imp().sync_soon.replace(None);
                    // The save tick is two seconds, so by here what is being
                    // sent is also what is on disk.
                    app.sync_now();
                }
            ),
        );
        self.imp().sync_soon.replace(Some(id));
    }

    /// A pass failed.
    ///
    /// Not reported the first time. A NAS asleep, a laptop between networks
    /// and a suspended machine all produce one failed pass, and a banner for
    /// each would train the user to ignore the banner. It only becomes an
    /// ongoing condition — the thing a banner is for — once it has kept
    /// failing.
    fn report_sync_failure(&self, message: String) {
        let failures = self.imp().sync_failures.get() + 1;
        self.imp().sync_failures.set(failures);
        self.imp().sync_last_failure.replace(Some(message.clone()));
        if failures < SYNC_FAILURES_BEFORE_SAYING_SO {
            return;
        }
        let already = self.imp().sync_error.borrow().as_deref() == Some(message.as_str());
        if already {
            return;
        }
        self.imp().sync_error.replace(Some(message));
        self.notify_save_state();
    }

    /// Why sync is unhappy, if it has been for long enough to matter.
    pub fn sync_error(&self) -> Option<String> {
        self.imp().sync_error.borrow().clone()
    }

    /// Open the sync status dialog.
    fn show_sync_status(&self) {
        let Some(window) = self.active_window().and_downcast::<PlannerWindow>() else {
            return;
        };
        let (rows, subtitle) = self.sync_status();
        window.show_sync_status(&rows, &subtitle);
    }

    /// What syncing has and has not done, as lines to put on screen.
    ///
    /// A pass says nothing while it is going well, which is right for something
    /// that runs on every edit and wrong for something you cannot check. This
    /// is the place you check, so it answers the question a quiet app leaves
    /// open: is this going anywhere, and did it get there.
    pub fn sync_status(&self) -> (Vec<(String, String)>, String) {
        let imp = self.imp();
        let target = imp.sync_target.borrow().clone();
        let failure = imp.sync_last_failure.borrow().clone();

        let mut rows = vec![(
            "Server".to_string(),
            match &target {
                Some((url, _)) => url.clone(),
                None => "Not set up — this planner stays on this machine".to_string(),
            },
        )];

        let here = live(&self.with_store(crate::model::sync::snapshot_of));
        rows.push((
            "Records here".to_string(),
            match here {
                1 => "1 record".to_string(),
                n => format!("{n} records"),
            },
        ));

        if target.is_some() {
            // The number that answers "did it work": how many records this
            // machine and the server last agreed on. Short of the local count
            // means there is work left rather than something broken.
            let agreed = live(&imp.sync_base.borrow());
            rows.push((
                "Synced".to_string(),
                match (agreed, here) {
                    (0, 0) => "Nothing to sync yet".to_string(),
                    (0, _) => "Not yet — the first pass has not finished".to_string(),
                    (agreed, here) if agreed >= here => format!("All {here}"),
                    (agreed, here) => format!("{agreed} of {here}, the rest on the next pass"),
                },
            ));

            rows.push((
                "Last pass".to_string(),
                match (&failure, imp.sync_last_pass.get()) {
                    (Some(error), _) => format!("Failed — {error}"),
                    (None, Some(when)) => ago(when),
                    (None, None) => "Not since Planner was opened".to_string(),
                },
            ));
        }

        rows.push((
            "File".to_string(),
            self.with_store(|store| store.path().display().to_string()),
        ));

        let subtitle = match (&target, &failure) {
            (None, _) => format!(
                "Syncing is off. Set sync_url and sync_token in {} to share this planner \
                 between machines.",
                crate::model::Config::default_path().display()
            ),
            (Some(_), Some(_)) => "Nothing here is at risk — the copy on this machine is the \
                                   one that counts, and the next pass will try again."
                .to_string(),
            (Some(_), None) => "A pass runs when an edit settles, and the server holds a \
                                request open so a change made elsewhere arrives as it happens."
                .to_string(),
        };

        (rows, subtitle)
    }

    // --- reminders ------------------------------------------------------

    /// Start watching for reminders coming due.
    ///
    /// A minute tick rather than a timer armed for the next reminder. The
    /// precision is the same — a reminder is a notification, not an alarm
    /// clock — and it needs no re-arming when a task is edited, when the
    /// clocks change, or when the machine wakes from suspend having missed
    /// whatever timer was pending.
    fn start_reminders(&self) {
        // Anything already overdue at startup is marked as seen rather than
        // shown: being told about yesterday's meeting on opening the app is
        // noise, and it would arrive again every launch until the task was
        // dealt with.
        let now = chrono::Utc::now();
        let zone = *chrono::Local::now().offset();
        self.with_store(|store| {
            self.imp().schedule.borrow_mut().catch_up(store, now, &zone);
        });

        let id = glib::timeout_add_seconds_local(
            30,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    app.fire_due_reminders();
                    glib::ControlFlow::Continue
                }
            ),
        );
        self.imp().reminder_tick.replace(Some(id));
    }

    fn fire_due_reminders(&self) {
        let now = chrono::Utc::now();
        let zone = *chrono::Local::now().offset();

        let due =
            self.with_store(|store| self.imp().schedule.borrow_mut().take_due(store, now, &zone));

        for firing in due {
            let notification = gio::Notification::new(&firing.title);
            notification.set_body(Some("Task due"));
            notification.set_priority(gio::NotificationPriority::Normal);
            // Identified by task, so a second reminder for the same task
            // replaces the first rather than stacking up.
            self.send_notification(Some(&format!("task-{}", firing.task)), &notification);
        }
    }

    /// Let a task's reminders fire again, after it repeated.
    fn rearm_reminders(&self, id: &TaskId) {
        self.imp().schedule.borrow_mut().forget(id);
    }

    /// Write the store out if anything has changed.
    pub fn save_now(&self) {
        if !self.imp().dirty.get() {
            return;
        }

        let result = self.imp().store.borrow().save();
        match result {
            Ok(()) => {
                self.imp().dirty.set(false);
                if self.imp().save_error.take().is_some() {
                    self.notify_save_state();
                }
            }
            Err(error) => {
                // Stay dirty: the next tick tries again, so a transient
                // failure heals itself without the user doing anything.
                let message = error.to_string();
                let previous = self.imp().save_error.replace(Some(message.clone()));
                if previous.as_deref() != Some(message.as_str()) {
                    self.notify_save_state();
                }
            }
        }
    }

    /// Put whichever condition matters most on the window's one banner.
    ///
    /// **A save failure outranks a sync failure**, because it is data not
    /// being written right now, where a sync failure is data that has not
    /// reached the other machine yet. Three conditions on one surface needs a
    /// priority rule, and this is it.
    fn notify_save_state(&self) {
        if let Some(window) = self.active_window().and_downcast::<PlannerWindow>() {
            let condition = self.save_error().or_else(|| {
                self.sync_error()
                    .map(|reason| format!("Not syncing: {reason}"))
            });
            window.set_save_error(condition);
        }
    }

    // --- what the user did ----------------------------------------------

    /// Tick a task off, or move a recurring one on.
    pub fn complete_task(&self, id: &TaskId) {
        let today = crate::ui::today();
        let now = chrono::Utc::now();
        let outcome = self.mutate(|store| store.complete_task(id, now, today));

        // A repeating task keeps its reminders, so the next occurrence has to
        // be allowed to fire them again.
        if matches!(outcome, Some(crate::model::Completion::Rescheduled { .. })) {
            self.rearm_reminders(id);
        }
        // And a completed task's notification is stale the moment it is done.
        self.withdraw_notification(&format!("task-{id}"));
        self.refresh();
    }

    /// File a quick-add line.
    ///
    /// Parsing happens here rather than in the dialog so that the names are
    /// resolved against the store as it is at the moment of adding, not as it
    /// was when the dialog opened.
    pub fn add_quick_add(
        &self,
        line: &str,
        default_project: &crate::model::ProjectId,
        default_section: Option<&crate::model::SectionId>,
    ) -> TaskId {
        let today = crate::ui::today();
        let now = chrono::Utc::now();

        let id = self.mutate(|store| {
            let parsed = crate::model::parse::parse_quick_add(line, today, &store.vocabulary());
            store.add_from_quick_add(&parsed, default_project, default_section, now)
        });
        self.refresh();
        id
    }

    /// Apply one edit from the detail panel.
    ///
    /// Every variant lands here rather than in the panel so that the "one
    /// place that can lose a task" rule survives the panel existing. It also
    /// keeps `touch` in one place: an edit that changed something bumps
    /// `updated_at`, and one that did not — retyping the same title, which
    /// `changed` fires for constantly — does not.
    pub fn apply_edit(&self, id: &TaskId, edit: &crate::ui::detail_panel::Edit) {
        use crate::ui::detail_panel::Edit;

        let now = chrono::Utc::now();
        let today = crate::ui::today();

        match edit {
            Edit::Delete | Edit::Open(_) => return, // the window handles these
            Edit::AddSubtask(line) => {
                self.add_subtask(id, line);
                return;
            }
            Edit::ToggleSubtask(child, checked) => {
                if *checked {
                    self.complete_task(child);
                } else {
                    self.uncomplete_task(child);
                }
                return;
            }
            Edit::Label { name, on } => {
                self.mutate(|store| {
                    let label = store.label_for_name(name, now);
                    if let Some(task) = store.task_mut(id) {
                        if *on {
                            task.add_label(label);
                        } else {
                            task.remove_label(&label);
                        }
                        task.touch(now);
                    }
                });
                self.refresh();
                return;
            }
            _ => {}
        }

        let changed = self.mutate(|store| {
            let Some(task) = store.task_mut(id) else {
                return false;
            };
            let changed = match edit {
                Edit::Title(title) if task.content != *title => {
                    task.content = title.clone();
                    true
                }
                Edit::Description(text) if task.description != *text => {
                    task.description = text.clone();
                    true
                }
                Edit::Priority(priority) if task.priority != *priority => {
                    task.priority = *priority;
                    true
                }
                Edit::Due(due) if task.due != *due => {
                    task.due = due.clone();
                    true
                }
                Edit::Deadline(deadline) if task.deadline != *deadline => {
                    task.deadline = *deadline;
                    true
                }
                Edit::Pinned(pinned) if task.pinned != *pinned => {
                    task.pinned = *pinned;
                    true
                }
                _ => false,
            };
            if changed {
                task.touch(now);
            }
            changed
        });

        let _ = today;
        if changed {
            self.refresh();
        }
    }

    /// Add a subtask from a quick-add line.
    ///
    /// A subtask shares its parent's project, whatever the line says: a child
    /// filed somewhere else would appear under a parent it cannot be reached
    /// from, and in a project whose count it inflates.
    pub fn add_subtask(&self, parent: &TaskId, line: &str) -> Option<TaskId> {
        let today = crate::ui::today();
        let now = chrono::Utc::now();

        let id = self.mutate(|store| {
            let (project, section) = {
                let parent = store.task(parent)?;
                (parent.project_id.clone(), parent.section_id.clone())
            };
            let parsed = crate::model::parse::parse_quick_add(line, today, &store.vocabulary());
            let id = store.add_from_quick_add(&parsed, &project, section.as_ref(), now);
            if let Some(task) = store.task_mut(&id) {
                task.parent_id = Some(parent.clone());
                task.project_id = project;
            }
            Some(id)
        });
        self.refresh();
        id
    }

    /// Move a task into a section of a project, at a position.
    pub fn move_task(
        &self,
        id: &TaskId,
        project: &crate::model::ProjectId,
        section: Option<&crate::model::SectionId>,
        index: u32,
    ) -> bool {
        let now = chrono::Utc::now();
        let moved = self.mutate(|store| store.move_task(id, project, section, index as usize, now));
        if moved {
            self.refresh();
        }
        moved
    }

    /// Create a project, optionally nested under another.
    pub fn add_project(
        &self,
        name: &str,
        parent: Option<&crate::model::ProjectId>,
    ) -> crate::model::ProjectId {
        let now = chrono::Utc::now();
        let id = self.mutate(|store| {
            let colour = store.next_project_color();
            let mut project = crate::model::Project::new(name, colour);
            project.parent_id = parent.cloned();
            store.add_project(project, now)
        });
        self.refresh();
        id
    }

    /// Rename a project.
    pub fn rename_project(&self, id: &crate::model::ProjectId, name: &str) {
        let now = chrono::Utc::now();
        self.mutate(|store| {
            if let Some(project) = store.project_mut(id) {
                project.name = name.to_string();
                project.touch(now);
            }
        });
        self.refresh();
    }

    /// Delete a project, its subprojects and their tasks.
    pub fn remove_project(
        &self,
        id: &crate::model::ProjectId,
    ) -> Option<crate::model::store::RemovedProject> {
        let now = chrono::Utc::now();
        let removed = self.mutate(|store| store.remove_project(id, now));
        self.refresh();
        removed
    }

    /// Put a deleted project back.
    pub fn restore_project(&self, removed: crate::model::store::RemovedProject) {
        self.mutate(|store| store.restore_project(removed));
        self.refresh();
    }

    /// Add a section to a project.
    pub fn add_section(&self, project: &crate::model::ProjectId, name: &str) {
        let now = chrono::Utc::now();
        self.mutate(|store| {
            if store.project(project).is_some() {
                store.add_section(crate::model::Section::new(project.clone(), name), now);
            }
        });
        self.refresh();
    }

    pub fn rename_section(&self, id: &crate::model::SectionId, name: &str) {
        let now = chrono::Utc::now();
        self.mutate(|store| store.rename_section(id, name, now));
        self.refresh();
    }

    /// Remove a section. Its tasks stay in the project, unsectioned.
    pub fn remove_section(
        &self,
        id: &crate::model::SectionId,
    ) -> Option<crate::model::store::RemovedSection> {
        let now = chrono::Utc::now();
        let removed = self.mutate(|store| store.remove_section(id, now));
        self.refresh();
        removed
    }

    /// Put a deleted section back, with the tasks it held.
    pub fn restore_section(&self, removed: crate::model::store::RemovedSection) {
        let now = chrono::Utc::now();
        self.mutate(|store| store.restore_section(removed, now));
        self.refresh();
    }

    /// Switch a project between list and board.
    pub fn set_view_style(&self, project: &crate::model::ProjectId, style: ViewStyle) {
        let now = chrono::Utc::now();
        let changed = self.mutate(|store| match store.project_mut(project) {
            Some(project) if project.view_style != style => {
                project.view_style = style;
                project.touch(now);
                true
            }
            _ => false,
        });
        if changed {
            self.refresh();
        }
    }

    /// Save a filter, new or edited.
    pub fn put_filter(&self, filter: crate::model::SavedFilter) {
        let now = chrono::Utc::now();
        self.mutate(|store| store.put_filter(filter, now));
        self.refresh();
    }

    /// Delete a filter.
    pub fn remove_filter(&self, id: &crate::model::FilterId) {
        let now = chrono::Utc::now();
        self.mutate(|store| store.remove_filter(id, now));
        self.refresh();
    }

    /// Complete several tasks at once.
    ///
    /// Returns the ones that were actually completed, so the undo can reopen
    /// exactly those: a task that was already done must not be reopened by
    /// undoing a bulk complete it took no part in.
    pub fn complete_all(&self, ids: &[TaskId]) -> Vec<TaskId> {
        let today = crate::ui::today();
        let now = chrono::Utc::now();

        let completed = self.mutate(|store| {
            let mut completed = Vec::new();
            for id in ids {
                if store.task(id).is_some_and(|task| task.checked) {
                    continue;
                }
                // A repeating task moves on rather than finishing, so undoing
                // the batch must not reopen it — it was never closed.
                if matches!(
                    store.complete_task(id, now, today),
                    Some(crate::model::Completion::Done)
                ) {
                    completed.push(id.clone());
                }
            }
            completed
        });
        self.refresh();
        completed
    }

    /// Reopen several tasks.
    pub fn uncomplete_all(&self, ids: &[TaskId]) {
        let now = chrono::Utc::now();
        self.mutate(|store| {
            for id in ids {
                store.uncomplete_task(id, now);
            }
        });
        self.refresh();
    }

    /// Delete several tasks, returning them all so it can be undone.
    pub fn delete_all(&self, ids: &[TaskId]) -> Vec<crate::model::Task> {
        let now = chrono::Utc::now();
        let removed = self.mutate(|store| {
            ids.iter()
                .flat_map(|id| store.remove_task(id, now))
                .collect::<Vec<_>>()
        });
        self.refresh();
        removed
    }

    /// Set the priority on several tasks.
    pub fn set_priority_all(&self, ids: &[TaskId], priority: crate::model::Priority) {
        let now = chrono::Utc::now();
        self.mutate(|store| {
            for id in ids {
                if let Some(task) = store.task_mut(id) {
                    if task.priority != priority {
                        task.priority = priority;
                        task.touch(now);
                    }
                }
            }
        });
        self.refresh();
    }

    /// Set the due date on several tasks, keeping each one's own repeat rule.
    pub fn set_due_all(&self, ids: &[TaskId], date: Option<chrono::NaiveDate>) {
        let now = chrono::Utc::now();
        self.mutate(|store| {
            for id in ids {
                let Some(task) = store.task_mut(id) else {
                    continue;
                };
                let updated = match (date, task.due.clone()) {
                    (None, _) => None,
                    // Keep the time and the rule this task already had: a bulk
                    // "do these on Friday" is about the day, and silently
                    // stopping three repeating tasks would not be.
                    (Some(date), Some(existing)) => Some(Due { date, ..existing }),
                    (Some(date), None) => Some(Due::on(date)),
                };
                if task.due != updated {
                    task.due = updated;
                    task.touch(now);
                }
            }
        });
        self.refresh();
    }

    /// Delete a task and its subtasks, returning them so it can be undone.
    pub fn delete_task(&self, id: &TaskId) -> Vec<crate::model::Task> {
        let now = chrono::Utc::now();
        let removed = self.mutate(|store| store.remove_task(id, now));
        self.refresh();
        removed
    }

    /// Put deleted tasks back.
    pub fn restore_tasks(&self, tasks: Vec<crate::model::Task>) {
        self.mutate(|store| store.restore_tasks(tasks));
        self.refresh();
    }

    /// Reopen a completed task.
    pub fn uncomplete_task(&self, id: &TaskId) {
        let now = chrono::Utc::now();
        self.mutate(|store| store.uncomplete_task(id, now));
        self.refresh();
    }

    /// Rebuild whatever the window is showing.
    pub fn refresh(&self) {
        if let Some(window) = self.active_window().and_downcast::<PlannerWindow>() {
            window.refresh();
        }
        // A task added, edited or pulled in by a sync needs a vector before it
        // can be found by meaning. No-op unless a model is already loaded, so
        // this costs nothing for the installs that never turn it on.
        self.rewarm_embeddings();
    }

    fn install_actions(&self) {
        let entries = [
            gio::ActionEntry::builder("quit")
                .activate(|app: &Self, _, _| {
                    app.save_now();
                    app.quit();
                })
                .build(),
            gio::ActionEntry::builder("about")
                .activate(|app: &Self, _, _| app.show_about())
                .build(),
            gio::ActionEntry::builder("sync-status")
                .activate(|app: &Self, _, _| app.show_sync_status())
                .build(),
        ];
        self.add_action_entries(entries);

        // Modifiers only. Planify binds bare letters (`a` for a new task) and
        // it is genuinely quicker, but a bare accelerator that fires while a
        // text entry has focus is a bug the user cannot work around, and
        // getting that right needs a shortcut controller scoped to the list
        // rather than an application accel. Ctrl-based ones are safe now; the
        // single-letter set can come with the shortcuts dialog.
        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("win.new-task", &["<Control>n"]);
        self.set_accels_for_action("win.toggle-sidebar", &["<Control>b"]);
        self.set_accels_for_action("win.find", &["<Control>f"]);
    }

    fn show_about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name("Planner")
            .application_icon(crate::APP_ID)
            .developer_name("Matthew Hagrelius")
            .version(env!("CARGO_PKG_VERSION"))
            .license_type(gtk::License::Gpl30)
            .build();
        dialog.present(self.active_window().as_ref());
    }
}

/// How many records a snapshot holds that have not been deleted.
///
/// Tombstones are in there so a pass can tell "deleted" from "never seen", but
/// a count of records that includes them answers no question a user is asking.
fn live(snapshot: &crate::model::sync::Snapshot) -> usize {
    snapshot
        .values()
        .filter(|version| matches!(version, crate::model::sync::Version::Live(_)))
        .count()
}

/// How long ago something happened, in the roundest terms that are still true.
///
/// Seconds would be a number that changes while you read it, and a pass that
/// ran twenty seconds ago and one that ran a minute ago mean the same thing.
fn ago(when: chrono::DateTime<chrono::Utc>) -> String {
    let seconds = (chrono::Utc::now() - when).num_seconds();
    match seconds {
        // The clock went backwards, or the pass is younger than the phrasing.
        i64::MIN..=90 => "just now".to_string(),
        91..=5400 => format!("{} minutes ago", (seconds + 30) / 60),
        _ => match (seconds + 1800) / 3600 {
            1 => "an hour ago".to_string(),
            hours => format!("{hours} hours ago"),
        },
    }
}
