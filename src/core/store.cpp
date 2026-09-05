#include "store.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonParseError>
#include <QSaveFile>
#include <QStandardPaths>

#include <algorithm>

namespace planner {

QString Store::defaultPath() {
    const QByteArray xdg = qgetenv("XDG_DATA_HOME");
    QString base;
    if (!xdg.isEmpty()) base = QString::fromLocal8Bit(xdg);
    else if (!qgetenv("HOME").isEmpty()) base = QString::fromLocal8Bit(qgetenv("HOME")) + QStringLiteral("/.local/share");
    else base = QStringLiteral(".");
    return base + QStringLiteral("/planner/planner.json");
}

Store Store::detached() {
    Store store;
    store.m_projects.append(Project::inbox());
    store.m_readOnly = true;
    return store;
}

// Move an unreadable file aside so the app can start. The timestamp in the
// name means repeated failures cannot overwrite the first, most useful, copy.
static LoadOutcome quarantine(const QString &path, const QString &reason) {
    const QString backup = path + QStringLiteral(".corrupt-%1").arg(QDateTime::currentSecsSinceEpoch());
    QFile::rename(path, backup);
    LoadOutcome outcome;
    outcome.kind = LoadOutcome::Recovered;
    outcome.backup = backup;
    outcome.reason = reason;
    return outcome;
}

Store Store::openAt(const QString &path, LoadOutcome *outcome) {
    Store store;
    store.m_path = path;
    store.m_projects.append(Project::inbox());
    LoadOutcome result;

    QFile file(path);
    if (!file.exists()) {
        result.kind = LoadOutcome::Fresh;
    } else if (!file.open(QIODevice::ReadOnly)) {
        result = quarantine(path, file.errorString());
    } else {
        QJsonParseError error;
        const QJsonDocument document = QJsonDocument::fromJson(file.readAll(), &error);
        if (error.error != QJsonParseError::NoError || !document.isObject()) {
            result = quarantine(path, error.error == QJsonParseError::NoError ? QStringLiteral("not a JSON object") : error.errorString());
        } else {
            const QJsonObject root = document.object();
            store.m_version = root.value(QStringLiteral("version")).toInt(kSchemaVersion);
            store.m_projects.clear();
            for (const QJsonValue &value : root.value(QStringLiteral("projects")).toArray()) store.m_projects.append(Project::fromJson(value.toObject()));
            for (const QJsonValue &value : root.value(QStringLiteral("labels")).toArray()) store.m_labels.append(Label::fromJson(value.toObject()));
            for (const QJsonValue &value : root.value(QStringLiteral("tasks")).toArray()) store.m_tasks.append(Task::fromJson(value.toObject()));
            for (const QJsonValue &value : root.value(QStringLiteral("filters")).toArray()) store.m_filters.append(SavedFilter::fromJson(value.toObject()));
            store.ensureInbox();
            if (store.m_version > kSchemaVersion) {
                store.m_readOnly = true;
                result.kind = LoadOutcome::ReadOnly;
                result.version = store.m_version;
            } else {
                result.kind = LoadOutcome::Loaded;
            }
        }
    }
    if (outcome) *outcome = result;
    return store;
}

// A hand-edited file that has lost its Inbox would strand every task that
// points at it.
void Store::ensureInbox() {
    for (const Project &project : m_projects)
        if (project.isInbox()) return;
    m_projects.prepend(Project::inbox());
}

QJsonObject Store::toJson() const {
    QJsonArray projects, labels, tasks, filters;
    for (const Project &project : m_projects) projects.append(project.toJson());
    for (const Label &label : m_labels) labels.append(label.toJson());
    for (const Task &task : m_tasks) tasks.append(task.toJson());
    for (const SavedFilter &filter : m_filters) filters.append(filter.toJson());
    return QJsonObject{{QStringLiteral("version"), m_version}, {QStringLiteral("projects"), projects},
                       {QStringLiteral("labels"), labels}, {QStringLiteral("tasks"), tasks}, {QStringLiteral("filters"), filters}};
}

std::optional<SaveError> Store::save() const {
    if (m_readOnly) {
        return SaveError{SaveError::Newer,
                         QStringLiteral("the planner file is from a newer version (v%1, this build understands v%2) and will not be overwritten")
                             .arg(m_version).arg(kSchemaVersion)};
    }
    QDir().mkpath(QFileInfo(m_path).absolutePath());
    // QSaveFile writes a temporary beside the target, flushes it, and renames
    // it over the original on commit, so an interrupted write leaves the old
    // file untouched.
    QSaveFile file(m_path);
    if (!file.open(QIODevice::WriteOnly))
        return SaveError{SaveError::Io, QStringLiteral("could not write the planner file: %1").arg(file.errorString())};
    file.write(QJsonDocument(toJson()).toJson(QJsonDocument::Indented));
    if (!file.commit())
        return SaveError{SaveError::Io, QStringLiteral("could not write the planner file: %1").arg(file.errorString())};
    return std::nullopt;
}

// --- projects ----------------------------------------------------------------

const Project *Store::project(const ProjectId &id) const {
    for (const Project &project : m_projects)
        if (project.id == id) return &project;
    return nullptr;
}

Project *Store::projectMut(const ProjectId &id) {
    for (Project &project : m_projects)
        if (project.id == id) return &project;
    return nullptr;
}

QList<const Project *> Store::projectsOrdered() const {
    QList<const Project *> ordered;
    for (const Project &project : m_projects)
        if (!project.isArchived) ordered.append(&project);
    std::stable_sort(ordered.begin(), ordered.end(), [](const Project *a, const Project *b) {
        if (a->order != b->order) return a->order < b->order;
        return a->name < b->name;
    });
    return ordered;
}

QList<const Project *> Store::subprojects(const ProjectId &parent) const {
    QList<const Project *> children;
    for (const Project *project : projectsOrdered())
        if (project->parentId && *project->parentId == parent) children.append(project);
    return children;
}

QList<ProjectId> Store::projectAndDescendants(const ProjectId &root) const {
    QList<ProjectId> found{root};
    for (int index = 0; index < found.size(); ++index) {
        for (const Project *child : subprojects(found.at(index)))
            if (!found.contains(child->id)) found.append(child->id);
    }
    return found;
}

ProjectId Store::addProject(Project project) {
    int max = -1;
    bool any = false;
    for (const Project &existing : m_projects) { max = any ? std::max(max, existing.order) : existing.order; any = true; }
    project.order = any ? max + 1 : 0;
    m_projects.append(project);
    return project.id;
}

Color Store::nextProjectColor() const {
    QList<Color> used;
    for (const Project &project : m_projects) used.append(project.color);
    return leastUsedColor(used);
}

template <typename T, typename F>
static QList<T> extract(QList<T> &items, F doomed) {
    QList<T> taken;
    for (int i = 0; i < items.size();) {
        if (doomed(items.at(i))) taken.append(items.takeAt(i));
        else ++i;
    }
    return taken;
}

std::optional<RemovedProject> Store::removeProject(const ProjectId &id) {
    if (id == inboxId() || !project(id)) return std::nullopt;
    const QList<ProjectId> doomed = projectAndDescendants(id);
    RemovedProject removed;
    removed.projects = extract(m_projects, [&](const Project &p) { return doomed.contains(p.id); });
    removed.tasks = extract(m_tasks, [&](const Task &t) { return doomed.contains(t.projectId); });
    return removed;
}

void Store::restoreProject(const RemovedProject &removed) {
    m_projects.append(removed.projects);
    m_tasks.append(removed.tasks);
}

const Project *Store::projectByName(const QString &name) const {
    for (const Project &project : m_projects)
        if (project.name.compare(name, Qt::CaseInsensitive) == 0) return &project;
    return nullptr;
}

// --- sections ----------------------------------------------------------------

std::pair<const Project *, const Section *> Store::section(const SectionId &id) const {
    for (const Project &project : m_projects)
        if (const Section *section = project.section(id)) return {&project, section};
    return {nullptr, nullptr};
}

// Tasks go back to the project's own list rather than being deleted with it.
std::optional<RemovedSection> Store::removeSection(const SectionId &id, const QDateTime &now) {
    const auto found = section(id);
    if (!found.first) return std::nullopt;
    const ProjectId projectId = found.first->id;
    const auto removedSection = projectMut(projectId)->removeSection(id);
    if (!removedSection) return std::nullopt;
    RemovedSection removed;
    removed.project = projectId;
    removed.section = *removedSection;
    for (Task &task : m_tasks) {
        if (task.sectionId && *task.sectionId == id) {
            task.sectionId.reset();
            task.touch(now);
            removed.tasks.append(task.id);
        }
    }
    return removed;
}

void Store::restoreSection(const RemovedSection &removed, const QDateTime &now) {
    Project *owner = projectMut(removed.project);
    if (!owner) return;
    owner->restoreSection(removed.section);
    for (Task &task : m_tasks) {
        // A task moved to another project in the meantime keeps its new home.
        if (task.projectId == removed.project && removed.tasks.contains(task.id)) {
            task.sectionId = removed.section.id;
            task.touch(now);
        }
    }
}

bool Store::renameSection(const SectionId &id, const QString &name) {
    const auto found = section(id);
    if (!found.first) return false;
    Section *target = projectMut(found.first->id)->sectionMut(id);
    if (!target) return false;
    target->name = name;
    return true;
}

bool Store::moveSection(const SectionId &id, int index) {
    const auto found = section(id);
    if (!found.first) return false;
    Project *owner = projectMut(found.first->id);
    QList<SectionId> order;
    for (const Section *s : owner->sectionsOrdered())
        if (s->id != id) order.append(s->id);
    index = std::clamp(index, 0, static_cast<int>(order.size()));
    order.insert(index, id);
    for (int position = 0; position < order.size(); ++position)
        if (Section *s = owner->sectionMut(order.at(position))) s->order = position;
    return true;
}

// --- labels ------------------------------------------------------------------

const Label *Store::label(const LabelId &id) const {
    for (const Label &label : m_labels)
        if (label.id == id) return &label;
    return nullptr;
}

Label *Store::labelMut(const LabelId &id) {
    for (Label &label : m_labels)
        if (label.id == id) return &label;
    return nullptr;
}

const Label *Store::labelByName(const QString &name) const {
    for (const Label &label : m_labels)
        if (label.matchesName(name)) return &label;
    return nullptr;
}

LabelId Store::labelForName(const QString &name) {
    if (const Label *existing = labelByName(name)) return existing->id;
    QList<Color> used;
    for (const Label &label : m_labels) used.append(label.color);
    Label label = Label::create(name, leastUsedColor(used));
    label.order = static_cast<int>(m_labels.size());
    m_labels.append(label);
    return label.id;
}

std::optional<Label> Store::removeLabel(const LabelId &id, const QDateTime &now) {
    for (int i = 0; i < m_labels.size(); ++i) {
        if (m_labels.at(i).id != id) continue;
        const Label removed = m_labels.takeAt(i);
        for (Task &task : m_tasks) {
            if (task.hasLabel(id)) {
                task.removeLabel(id);
                task.touch(now);
            }
        }
        return removed;
    }
    return std::nullopt;
}

QHash<LabelId, int> Store::labelCounts() const {
    QHash<LabelId, int> counts;
    for (const Task &task : m_tasks) {
        if (task.checked) continue;
        for (const LabelId &label : task.labels) counts[label] += 1;
    }
    return counts;
}

// --- saved filters -----------------------------------------------------------

const SavedFilter *Store::filter(const FilterId &id) const {
    for (const SavedFilter &filter : m_filters)
        if (filter.id == id) return &filter;
    return nullptr;
}

QList<const SavedFilter *> Store::filtersOrdered() const {
    QList<const SavedFilter *> ordered;
    for (const SavedFilter &filter : m_filters) ordered.append(&filter);
    std::stable_sort(ordered.begin(), ordered.end(), [](const SavedFilter *a, const SavedFilter *b) {
        if (a->order != b->order) return a->order < b->order;
        return a->name < b->name;
    });
    return ordered;
}

FilterId Store::putFilter(const SavedFilter &filter) {
    for (SavedFilter &existing : m_filters) {
        if (existing.id == filter.id) {
            existing = filter;
            return filter.id;
        }
    }
    SavedFilter added = filter;
    int max = -1;
    bool any = false;
    for (const SavedFilter &existing : m_filters) { max = any ? std::max(max, existing.order) : existing.order; any = true; }
    added.order = any ? max + 1 : 0;
    m_filters.append(added);
    return added.id;
}

std::optional<SavedFilter> Store::removeFilter(const FilterId &id) {
    for (int i = 0; i < m_filters.size(); ++i)
        if (m_filters.at(i).id == id) return m_filters.takeAt(i);
    return std::nullopt;
}

Color Store::nextFilterColor() const {
    QList<Color> used;
    for (const SavedFilter &filter : m_filters) used.append(filter.color);
    return leastUsedColor(used);
}

// --- tasks -------------------------------------------------------------------

const Task *Store::task(const TaskId &id) const {
    for (const Task &task : m_tasks)
        if (task.id == id) return &task;
    return nullptr;
}

Task *Store::taskMut(const TaskId &id) {
    for (Task &task : m_tasks)
        if (task.id == id) return &task;
    return nullptr;
}

TaskId Store::addTask(Task task) {
    int max = -1;
    bool any = false;
    for (const Task &existing : m_tasks) {
        if (existing.projectId == task.projectId && existing.sectionId == task.sectionId) {
            max = any ? std::max(max, existing.order) : existing.order;
            any = true;
        }
    }
    task.order = any ? max + 1 : 0;
    m_tasks.append(task);
    return task.id;
}

Vocabulary Store::vocabulary() const {
    Vocabulary vocabulary;
    for (const Project &project : m_projects) {
        vocabulary.projects << project.name;
        for (const Section &section : project.sections) vocabulary.sections << section.name;
    }
    for (const Label &label : m_labels) vocabulary.labels << label.name;
    return vocabulary;
}

TaskId Store::addFromQuickAdd(const QuickAdd &parsed, const ProjectId &defaultProject,
                              const std::optional<SectionId> &defaultSection, const QDateTime &now) {
    ProjectId projectId;
    if (parsed.project) {
        if (const Project *named = projectByName(*parsed.project)) projectId = named->id;
    }
    if (projectId.isEmpty()) {
        // A default pointing at a project that has since been deleted must not
        // strand the task somewhere invisible.
        projectId = project(defaultProject) ? defaultProject : inboxId();
    }

    std::optional<SectionId> sectionId;
    const Project *owner = project(projectId);
    if (parsed.section) {
        if (owner) {
            for (const Section &section : owner->sections)
                if (section.name.compare(*parsed.section, Qt::CaseInsensitive) == 0) sectionId = section.id;
        }
    } else if (defaultSection && !parsed.project) {
        // A default section only applies in its own project.
        if (owner && owner->section(*defaultSection)) sectionId = defaultSection;
    }

    QList<LabelId> labels;
    for (const QString &name : parsed.labels) labels.append(labelForName(name));

    Task task = Task::create(projectId, parsed.title.trimmed(), now);
    task.sectionId = sectionId;
    task.labels = labels;
    task.due = parsed.due;
    if (parsed.priority) task.priority = *parsed.priority;
    for (qint64 minutes : parsed.reminders) task.reminders.append(Reminder::beforeDue(minutes));
    return addTask(task);
}

QList<const Task *> Store::tasksIn(const ProjectId &project, const std::optional<SectionId> &section) const {
    QList<const Task *> tasks;
    for (const Task &task : m_tasks)
        if (!task.parentId && task.projectId == project && task.sectionId == section) tasks.append(&task);
    std::stable_sort(tasks.begin(), tasks.end(), [](const Task *a, const Task *b) {
        if (a->order != b->order) return a->order < b->order;
        return a->addedAt < b->addedAt;
    });
    return tasks;
}

// Renumbering is bookkeeping, not an edit, so `updated_at` is left alone.
void Store::renumber(const QList<TaskId> &ids) {
    for (int position = 0; position < ids.size(); ++position)
        if (Task *task = taskMut(ids.at(position))) task->order = position;
}

bool Store::moveTask(const TaskId &id, const ProjectId &projectId, const std::optional<SectionId> &sectionId, int index, const QDateTime &now) {
    const Task *moving = task(id);
    if (!moving || !project(projectId)) return false;
    if (sectionId && !project(projectId)->section(*sectionId)) return false;

    const ProjectId previousProject = moving->projectId;
    const std::optional<SectionId> previousSection = moving->sectionId;
    // A dragged task takes its subtasks with it.
    const QList<TaskId> family = taskAndDescendants(id);

    QList<TaskId> order;
    for (const Task *other : tasksIn(projectId, sectionId))
        if (other->id != id) order.append(other->id);
    index = std::clamp(index, 0, static_cast<int>(order.size()));
    order.insert(index, id);

    for (const TaskId &member : family) {
        if (Task *t = taskMut(member)) {
            t->projectId = projectId;
            if (member == id) t->sectionId = sectionId;
            t->touch(now);
        }
    }
    renumber(order);
    if (previousProject != projectId || previousSection != sectionId) {
        QList<TaskId> vacated;
        for (const Task *other : tasksIn(previousProject, previousSection)) vacated.append(other->id);
        renumber(vacated);
    }
    return true;
}

QList<const Task *> Store::subtasks(const TaskId &parent) const {
    QList<const Task *> children;
    for (const Task &task : m_tasks)
        if (task.parentId && *task.parentId == parent) children.append(&task);
    std::stable_sort(children.begin(), children.end(), [](const Task *a, const Task *b) { return a->order < b->order; });
    return children;
}

QList<TaskId> Store::taskAndDescendants(const TaskId &root) const {
    QList<TaskId> found{root};
    for (int index = 0; index < found.size(); ++index)
        for (const Task *child : subtasks(found.at(index)))
            if (!found.contains(child->id)) found.append(child->id);
    return found;
}

QList<Task> Store::removeTask(const TaskId &id) {
    const QList<TaskId> doomed = taskAndDescendants(id);
    return extract(m_tasks, [&](const Task &t) { return doomed.contains(t.id); });
}

void Store::restoreTasks(const QList<Task> &tasks) { m_tasks.append(tasks); }

std::optional<Completion> Store::completeTask(const TaskId &id, const QDateTime &now, const QDate &today) {
    const QList<TaskId> affected = taskAndDescendants(id);
    Task *target = taskMut(id);
    if (!target) return std::nullopt;
    const Completion outcome = target->complete(now, today);
    if (outcome.kind == Completion::Done) {
        for (int i = 1; i < affected.size(); ++i)
            if (Task *child = taskMut(affected.at(i))) child->complete(now, today);
    }
    return outcome;
}

void Store::uncompleteTask(const TaskId &id, const QDateTime &now) {
    std::optional<TaskId> current = id;
    while (current) {
        Task *task = taskMut(*current);
        if (!task) break;
        task->uncomplete(now);
        current = task->parentId;
    }
}

std::pair<int, int> Store::progress(const ProjectId &project) const {
    int total = 0, done = 0;
    for (const Task &task : m_tasks) {
        if (task.projectId != project) continue;
        ++total;
        if (task.checked) ++done;
    }
    return {done, total};
}

} // namespace planner
