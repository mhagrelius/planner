// The store: every project, label and task, and the file they live in.
//
// One JSON file, held entirely in memory. A personal task list is tens of
// kilobytes and every view is a linear scan over a few thousand records.
// The store owns all the records and hands out pointers that are valid only
// until the next mutation — the same discipline the original borrow checker
// enforced, kept here by convention.
//
// Writes cannot lose the previous file: a save goes through QSaveFile, which
// writes a temporary and renames it into place. A file that will not parse
// is moved aside and the app starts empty. A file from a newer schema
// version is never overwritten.
#pragma once

#include "model.h"
#include "quickadd.h"

#include <QHash>

namespace planner {

constexpr int kSchemaVersion = 1;

struct LoadOutcome {
    enum Kind { Loaded, Fresh, Recovered, ReadOnly } kind = Fresh;
    QString backup;   // Recovered
    QString reason;   // Recovered
    int version = 0;  // ReadOnly
};

struct SaveError {
    enum Kind { Newer, Io } kind = Io;
    QString message;
};

struct RemovedProject {
    QList<Project> projects;
    QList<Task> tasks;
};

struct RemovedSection {
    ProjectId project;
    Section section;
    QList<TaskId> tasks;   // named, not carried: edits made meanwhile survive an undo
};

class Store {
public:
    // $XDG_DATA_HOME/planner/planner.json, falling back to ~/.local/share.
    static QString defaultPath();
    static Store open(LoadOutcome *outcome = nullptr) { return openAt(defaultPath(), outcome); }
    // Never fails: an unreadable file is set aside and an empty store returned.
    static Store openAt(const QString &path, LoadOutcome *outcome = nullptr);
    // An empty store with nowhere to save to; refuses to save rather than
    // writing somewhere arbitrary.
    static Store detached();

    QString path() const { return m_path; }
    bool isReadOnly() const { return m_readOnly; }
    std::optional<SaveError> save() const;
    QJsonObject toJson() const;

    // --- projects
    const QList<Project> &projects() const { return m_projects; }
    const Project *project(const ProjectId &id) const;
    Project *projectMut(const ProjectId &id);
    QList<const Project *> projectsOrdered() const;        // Inbox first, then by order
    QList<const Project *> subprojects(const ProjectId &parent) const;
    QList<ProjectId> projectAndDescendants(const ProjectId &root) const;   // cycle-safe
    ProjectId addProject(Project project);
    Color nextProjectColor() const;
    std::optional<RemovedProject> removeProject(const ProjectId &id);      // the Inbox cannot go
    void restoreProject(const RemovedProject &removed);
    const Project *projectByName(const QString &name) const;

    // --- sections
    std::pair<const Project *, const Section *> section(const SectionId &id) const;
    std::optional<RemovedSection> removeSection(const SectionId &id, const QDateTime &now);
    void restoreSection(const RemovedSection &removed, const QDateTime &now);
    bool renameSection(const SectionId &id, const QString &name);
    bool moveSection(const SectionId &id, int index);

    // --- labels
    const QList<Label> &labels() const { return m_labels; }
    const Label *label(const LabelId &id) const;
    Label *labelMut(const LabelId &id);
    const Label *labelByName(const QString &name) const;
    LabelId labelForName(const QString &name);              // creates it if new
    std::optional<Label> removeLabel(const LabelId &id, const QDateTime &now);
    QHash<LabelId, int> labelCounts() const;                // open tasks per label

    // --- saved filters
    const QList<SavedFilter> &filters() const { return m_filters; }
    const SavedFilter *filter(const FilterId &id) const;
    QList<const SavedFilter *> filtersOrdered() const;
    FilterId putFilter(const SavedFilter &filter);          // add or replace by id
    std::optional<SavedFilter> removeFilter(const FilterId &id);
    Color nextFilterColor() const;

    // --- tasks
    const QList<Task> &tasks() const { return m_tasks; }
    const Task *task(const TaskId &id) const;
    Task *taskMut(const TaskId &id);
    TaskId addTask(Task task);                              // at the end of its list
    Vocabulary vocabulary() const;
    // Names become ids here. An unknown @label is created; an unknown #project
    // is not — the task lands in the default instead.
    TaskId addFromQuickAdd(const QuickAdd &parsed, const ProjectId &defaultProject,
                           const std::optional<SectionId> &defaultSection, const QDateTime &now);
    // Top-level tasks of one project or section, in display order.
    QList<const Task *> tasksIn(const ProjectId &project, const std::optional<SectionId> &section) const;
    // `index` counts positions after the task has been taken out of wherever
    // it was. False if the destination does not exist.
    bool moveTask(const TaskId &id, const ProjectId &project, const std::optional<SectionId> &section, int index, const QDateTime &now);
    QList<const Task *> subtasks(const TaskId &parent) const;
    QList<TaskId> taskAndDescendants(const TaskId &root) const;
    QList<Task> removeTask(const TaskId &id);               // and its subtasks
    void restoreTasks(const QList<Task> &tasks);
    // Completing a parent completes its open subtasks; a recurring parent moves
    // on and leaves them alone.
    std::optional<Completion> completeTask(const TaskId &id, const QDateTime &now, const QDate &today);
    // Reopens the chain of parents above it too.
    void uncompleteTask(const TaskId &id, const QDateTime &now);
    std::pair<int, int> progress(const ProjectId &project) const;   // (done, total), subtasks included

private:
    void ensureInbox();
    void renumber(const QList<TaskId> &ids);

    QString m_path;
    int m_version = kSchemaVersion;
    QList<Project> m_projects;
    QList<Label> m_labels;
    QList<Task> m_tasks;
    QList<SavedFilter> m_filters;
    bool m_readOnly = false;
};

} // namespace planner
