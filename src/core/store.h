// The store: every project, section, label, task and saved filter, the
// markers for what was deleted, and the file they live in.
//
// One JSON file, held entirely in memory. A personal task list is tens of
// kilobytes and every view is a linear scan over a few thousand records.
// The store owns all the records and hands out pointers that are valid only
// until the next mutation.
//
// Schema v2: sections are records of their own, hand-sorted lists use order
// keys, and a deletion leaves a tombstone so another machine can tell "gone"
// from "never seen". A v1 file reads in and is written back as v2.
//
// Writes cannot lose the previous file: a save goes through QSaveFile. A file
// that will not parse is moved aside and the app starts empty. A file from a
// newer schema version is never overwritten.
#pragma once

#include "model.h"
#include "quickadd.h"

#include <QHash>
#include <QJsonValue>

namespace planner {

constexpr int kSchemaVersion = 2;

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
    QList<Section> sections;
    QList<Task> tasks;
};

struct RemovedSection {
    ProjectId project;
    Section section;
    QList<TaskId> tasks;   // named, not carried: edits made meanwhile survive an undo
};

class Store {
public:
    static QString defaultPath();
    static Store open(LoadOutcome *outcome = nullptr) { return openAt(defaultPath(), outcome); }
    // Never fails: an unreadable file is set aside and an empty store returned.
    // `now` stamps the quarantine name and prunes expired tombstones.
    static Store openAt(const QString &path, LoadOutcome *outcome = nullptr, const QDateTime &now = QDateTime::currentDateTimeUtc());
    static Store detached();

    QString path() const { return m_path; }
    bool isReadOnly() const { return m_readOnly; }
    std::optional<SaveError> save() const;
    QJsonObject toJson() const;

    // --- projects
    const QList<Project> &projects() const { return m_projects; }
    const Project *project(const ProjectId &id) const;
    Project *projectMut(const ProjectId &id);
    QList<const Project *> projectsOrdered() const;
    QList<const Project *> subprojects(const ProjectId &parent) const;
    QList<ProjectId> projectAndDescendants(const ProjectId &root) const;
    ProjectId addProject(Project project, const QDateTime &now);
    Color nextProjectColor() const;
    // Takes subprojects, their sections and their tasks; the Inbox cannot go.
    std::optional<RemovedProject> removeProject(const ProjectId &id, const QDateTime &now);
    void restoreProject(const RemovedProject &removed);
    const Project *projectByName(const QString &name) const;

    // --- sections
    const QList<Section> &sections() const { return m_sections; }
    const Section *section(const SectionId &id) const;
    Section *sectionMut(const SectionId &id);
    QList<const Section *> sectionsIn(const ProjectId &project) const;   // display order
    SectionId addSection(Section section, const QDateTime &now);        // at the end of its project
    std::optional<RemovedSection> removeSection(const SectionId &id, const QDateTime &now);
    void restoreSection(const RemovedSection &removed, const QDateTime &now);
    bool renameSection(const SectionId &id, const QString &name, const QDateTime &now);
    bool moveSection(const SectionId &id, int index, const QDateTime &now);

    // --- labels
    const QList<Label> &labels() const { return m_labels; }
    const Label *label(const LabelId &id) const;
    Label *labelMut(const LabelId &id);
    const Label *labelByName(const QString &name) const;
    LabelId labelForName(const QString &name, const QDateTime &now);   // creates it if new
    std::optional<Label> removeLabel(const LabelId &id, const QDateTime &now);
    QHash<LabelId, int> labelCounts() const;

    // --- saved filters
    const QList<SavedFilter> &filters() const { return m_filters; }
    const SavedFilter *filter(const FilterId &id) const;
    QList<const SavedFilter *> filtersOrdered() const;
    FilterId putFilter(SavedFilter filter, const QDateTime &now);
    std::optional<SavedFilter> removeFilter(const FilterId &id, const QDateTime &now);
    Color nextFilterColor() const;

    // --- tasks
    const QList<Task> &tasks() const { return m_tasks; }
    const Task *task(const TaskId &id) const;
    Task *taskMut(const TaskId &id);
    TaskId addTask(Task task);
    Vocabulary vocabulary() const;
    TaskId addFromQuickAdd(const QuickAdd &parsed, const ProjectId &defaultProject,
                           const std::optional<SectionId> &defaultSection, const QDateTime &now);
    QList<const Task *> tasksIn(const ProjectId &project, const std::optional<SectionId> &section) const;
    // One record changes: the moved task takes the key between its new neighbours.
    bool moveTask(const TaskId &id, const ProjectId &project, const std::optional<SectionId> &section, int index, const QDateTime &now);
    QList<const Task *> subtasks(const TaskId &parent) const;
    QList<TaskId> taskAndDescendants(const TaskId &root) const;
    QList<Task> removeTask(const TaskId &id, const QDateTime &now);
    void restoreTasks(const QList<Task> &tasks);
    std::optional<Completion> completeTask(const TaskId &id, const QDateTime &now, const QDate &today);
    // A dated note on a task; false if the task is not there or the text is blank.
    bool addNote(const TaskId &id, const QString &text, const QDateTime &now);
    bool removeNote(const TaskId &id, const QDateTime &at, const QDateTime &now);
    void uncompleteTask(const TaskId &id, const QDateTime &now);
    std::pair<int, int> progress(const ProjectId &project) const;

    // --- tombstones and sync
    const QList<Tombstone> &tombstones() const { return m_tombstones; }
    bool isDeleted(RecordKind kind, const QString &id) const;
    // Put a record from another machine in, keeping its own updated_at: this
    // is the same edit arriving here, not a new one. False if it will not read.
    bool mergeRecord(RecordKind kind, const QJsonObject &body);
    // Apply a deletion another machine made. The Inbox is exempt.
    void applyDeletion(RecordKind kind, const QString &id, const QDateTime &at);
    // The record as it goes on the wire; nothing if it is not there.
    std::optional<QJsonObject> recordBody(RecordKind kind, const QString &id) const;

private:
    void ensureInbox();
    void liftLegacySections();
    void purgeTombstones(const QDateTime &now);
    void markDeleted(RecordKind kind, const QString &id, const QDateTime &now);
    void unmark(RecordKind kind, const QString &id);

    QString m_path;
    int m_version = kSchemaVersion;
    QList<Project> m_projects;
    QList<Section> m_sections;
    QList<Label> m_labels;
    QList<Task> m_tasks;
    QList<SavedFilter> m_filters;
    QList<Tombstone> m_tombstones;
    bool m_readOnly = false;
};

} // namespace planner
