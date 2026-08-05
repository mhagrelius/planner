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

    fn notify_save_state(&self) {
        if let Some(window) = self.active_window().and_downcast::<PlannerWindow>() {
            window.set_save_error(self.save_error());
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
                    let label = store.label_for_name(name);
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
        let id = self.mutate(|store| {
            let colour = store.next_project_color();
            let mut project = crate::model::Project::new(name, colour);
            project.parent_id = parent.cloned();
            store.add_project(project)
        });
        self.refresh();
        id
    }

    /// Rename a project.
    pub fn rename_project(&self, id: &crate::model::ProjectId, name: &str) {
        self.mutate(|store| {
            if let Some(project) = store.project_mut(id) {
                project.name = name.to_string();
            }
        });
        self.refresh();
    }

    /// Delete a project, its subprojects and their tasks.
    pub fn remove_project(
        &self,
        id: &crate::model::ProjectId,
    ) -> Option<crate::model::store::RemovedProject> {
        let removed = self.mutate(|store| store.remove_project(id));
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
        self.mutate(|store| {
            if store.project(project).is_some() {
                store.add_section(crate::model::Section::new(project.clone(), name));
            }
        });
        self.refresh();
    }

    pub fn rename_section(&self, id: &crate::model::SectionId, name: &str) {
        self.mutate(|store| store.rename_section(id, name));
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
        let changed = self.mutate(|store| match store.project_mut(project) {
            Some(project) if project.view_style != style => {
                project.view_style = style;
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
        self.mutate(|store| store.put_filter(filter));
        self.refresh();
    }

    /// Delete a filter.
    pub fn remove_filter(&self, id: &crate::model::FilterId) {
        self.mutate(|store| store.remove_filter(id));
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
        let removed = self.mutate(|store| {
            ids.iter()
                .flat_map(|id| store.remove_task(id))
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
        let removed = self.mutate(|store| store.remove_task(id));
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
