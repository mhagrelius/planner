#include "store.h"

#include "order.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonParseError>
#include <QSaveFile>

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

static LoadOutcome quarantine(const QString &path, const QString &reason, const QDateTime &now) {
    const QString backup = path + QStringLiteral(".corrupt-%1").arg(now.toSecsSinceEpoch());
    QFile::rename(path, backup);
    LoadOutcome outcome;
    outcome.kind = LoadOutcome::Recovered;
    outcome.backup = backup;
    outcome.reason = reason;
    return outcome;
}

Store Store::openAt(const QString &path, LoadOutcome *outcome, const QDateTime &now) {
    Store store;
    store.m_path = path;
    store.m_projects.append(Project::inbox());
    LoadOutcome result;

    QFile file(path);
    if (!file.exists()) {
        result.kind = LoadOutcome::Fresh;
    } else if (!file.open(QIODevice::ReadOnly)) {
        result = quarantine(path, file.errorString(), now);
    } else {
        QJsonParseError error;
        const QJsonDocument document = QJsonDocument::fromJson(file.readAll(), &error);
        if (error.error != QJsonParseError::NoError || !document.isObject()) {
            result = quarantine(path, error.error == QJsonParseError::NoError ? QStringLiteral("not a JSON object") : error.errorString(), now);
        } else {
            const QJsonObject root = document.object();
            store.m_version = root.value(QStringLiteral("version")).toInt(kSchemaVersion);
            store.m_projects.clear();
            for (const QJsonValue &value : root.value(QStringLiteral("projects")).toArray()) store.m_projects.append(Project::fromJson(value.toObject()));
            for (const QJsonValue &value : root.value(QStringLiteral("sections")).toArray()) store.m_sections.append(Section::fromJson(value.toObject()));
            for (const QJsonValue &value : root.value(QStringLiteral("labels")).toArray()) store.m_labels.append(Label::fromJson(value.toObject()));
            for (const QJsonValue &value : root.value(QStringLiteral("tasks")).toArray()) store.m_tasks.append(Task::fromJson(value.toObject()));
            for (const QJsonValue &value : root.value(QStringLiteral("filters")).toArray()) store.m_filters.append(SavedFilter::fromJson(value.toObject()));
            for (const QJsonValue &value : root.value(QStringLiteral("tombstones")).toArray())
                if (const auto tombstone = Tombstone::fromJson(value.toObject())) store.m_tombstones.append(*tombstone);
            store.ensureInbox();
            store.liftLegacySections();
            store.purgeTombstones(now);
            if (store.m_version > kSchemaVersion) {
                store.m_readOnly = true;
                result.kind = LoadOutcome::ReadOnly;
                result.version = store.m_version;
            } else {
                // Written back in the shape this build writes.
                store.m_version = kSchemaVersion;
                result.kind = LoadOutcome::Loaded;
            }
        }
    }
    if (outcome) *outcome = result;
    return store;
}

void Store::ensureInbox() {
    for (const Project &project : m_projects)
        if (project.isInbox()) return;
    m_projects.prepend(Project::inbox());
}

// Runs on every open rather than only when the version says v1: a merged or
// hand-edited file can hold both shapes, and a section left nested is a board
// column that silently stops existing.
void Store::liftLegacySections() {
    for (Project &project : m_projects) {
        for (const Section &legacy : project.legacySections) {
            bool already = false;
            for (const Section &existing : m_sections)
                if (existing.id == legacy.id) already = true;
            if (!already) m_sections.append(legacy);
        }
        project.legacySections.clear();
    }
}

void Store::purgeTombstones(const QDateTime &now) {
    const QDateTime cutoff = now.addDays(-kTombstoneRetentionDays);
    m_tombstones.erase(std::remove_if(m_tombstones.begin(), m_tombstones.end(), [&](const Tombstone &t) { return t.deletedAt <= cutoff; }), m_tombstones.end());
}

QJsonObject Store::toJson() const {
    QJsonArray projects, sections, labels, tasks, filters, tombstones;
    for (const Project &project : m_projects) projects.append(project.toJson());
    for (const Section &section : m_sections) sections.append(section.toJson());
    for (const Label &label : m_labels) labels.append(label.toJson());
    for (const Task &task : m_tasks) tasks.append(task.toJson());
    for (const SavedFilter &filter : m_filters) filters.append(filter.toJson());
    for (const Tombstone &tombstone : m_tombstones) tombstones.append(tombstone.toJson());
    return QJsonObject{{QStringLiteral("version"), m_version}, {QStringLiteral("projects"), projects}, {QStringLiteral("sections"), sections},
                       {QStringLiteral("labels"), labels}, {QStringLiteral("tasks"), tasks}, {QStringLiteral("filters"), filters},
                       {QStringLiteral("tombstones"), tombstones}};
}

std::optional<SaveError> Store::save() const {
    if (m_readOnly) {
        return SaveError{SaveError::Newer,
                         QStringLiteral("the planner file is from a newer version (v%1, this build understands v%2) and will not be overwritten")
                             .arg(m_version).arg(kSchemaVersion)};
    }
    QDir().mkpath(QFileInfo(m_path).absolutePath());
    QSaveFile file(m_path);
    if (!file.open(QIODevice::WriteOnly))
        return SaveError{SaveError::Io, QStringLiteral("could not write the planner file: %1").arg(file.errorString())};
    file.write(QJsonDocument(toJson()).toJson(QJsonDocument::Indented));
    if (!file.commit())
        return SaveError{SaveError::Io, QStringLiteral("could not write the planner file: %1").arg(file.errorString())};
    return std::nullopt;
}

// --- tombstones ------------------------------------------------------------------

void Store::markDeleted(RecordKind kind, const QString &id, const QDateTime &now) {
    unmark(kind, id);
    m_tombstones.append(Tombstone{kind, id, now.toUTC()});
}

// An undo that left the marker would delete the record again on the next sync.
void Store::unmark(RecordKind kind, const QString &id) {
    m_tombstones.erase(std::remove_if(m_tombstones.begin(), m_tombstones.end(), [&](const Tombstone &t) { return t.kind == kind && t.id == id; }), m_tombstones.end());
}

bool Store::isDeleted(RecordKind kind, const QString &id) const {
    for (const Tombstone &t : m_tombstones)
        if (t.kind == kind && t.id == id) return true;
    return false;
}

template <typename T>
static void replaceById(QList<T> &items, const T &incoming) {
    for (T &existing : items) {
        if (existing.id == incoming.id) {
            existing = incoming;
            return;
        }
    }
    items.append(incoming);
}

bool Store::mergeRecord(RecordKind kind, const QJsonObject &body) {
    if (body.value(QStringLiteral("id")).toString().isEmpty()) return false;
    switch (kind) {
    case RecordKind::Task: {
        const Task task = Task::fromJson(body);
        if (task.projectId.isEmpty() || !body.contains(QStringLiteral("added_at"))) return false;
        unmark(kind, task.id);
        replaceById(m_tasks, task);
        return true;
    }
    case RecordKind::Project: {
        const Project project = Project::fromJson(body);
        unmark(kind, project.id);
        replaceById(m_projects, project);
        return true;
    }
    case RecordKind::Section: {
        const Section section = Section::fromJson(body);
        if (section.projectId.isEmpty()) return false;
        unmark(kind, section.id);
        replaceById(m_sections, section);
        return true;
    }
    case RecordKind::Label: {
        const Label label = Label::fromJson(body);
        unmark(kind, label.id);
        replaceById(m_labels, label);
        return true;
    }
    case RecordKind::Filter: {
        const SavedFilter filter = SavedFilter::fromJson(body);
        unmark(kind, filter.id);
        replaceById(m_filters, filter);
        return true;
    }
    }
    return false;
}

template <typename T>
static void eraseById(QList<T> &items, const QString &id) {
    items.erase(std::remove_if(items.begin(), items.end(), [&](const T &item) { return item.id == id; }), items.end());
}

void Store::applyDeletion(RecordKind kind, const QString &id, const QDateTime &at) {
    switch (kind) {
    case RecordKind::Task: eraseById(m_tasks, id); break;
    case RecordKind::Project:
        if (id == inboxId()) return;
        eraseById(m_projects, id);
        break;
    case RecordKind::Section: eraseById(m_sections, id); break;
    case RecordKind::Label: eraseById(m_labels, id); break;
    case RecordKind::Filter: eraseById(m_filters, id); break;
    }
    markDeleted(kind, id, at);
}

std::optional<QJsonObject> Store::recordBody(RecordKind kind, const QString &id) const {
    switch (kind) {
    case RecordKind::Task: if (const Task *t = task(id)) return t->toJson(); break;
    case RecordKind::Project: if (const Project *p = project(id)) return p->toJson(); break;
    case RecordKind::Section: if (const Section *s = section(id)) return s->toJson(); break;
    case RecordKind::Label: if (const Label *l = label(id)) return l->toJson(); break;
    case RecordKind::Filter: if (const SavedFilter *f = filter(id)) return f->toJson(); break;
    }
    return std::nullopt;
}

// --- projects --------------------------------------------------------------------

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
    for (int index = 0; index < found.size(); ++index)
        for (const Project *child : subprojects(found.at(index)))
            if (!found.contains(child->id)) found.append(child->id);
    return found;
}

ProjectId Store::addProject(Project project, const QDateTime &now) {
    project.touch(now);
    int max = -1;
    bool any = false;
    for (const Project &existing : m_projects) { max = any ? std::max(max, existing.order) : existing.order; any = true; }
    project.order = any ? max + 1 : 0;
    unmark(RecordKind::Project, project.id);
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

std::optional<RemovedProject> Store::removeProject(const ProjectId &id, const QDateTime &now) {
    if (id == inboxId() || !project(id)) return std::nullopt;
    const QList<ProjectId> doomed = projectAndDescendants(id);
    RemovedProject removed;
    removed.projects = extract(m_projects, [&](const Project &p) { return doomed.contains(p.id); });
    removed.sections = extract(m_sections, [&](const Section &s) { return doomed.contains(s.projectId); });
    removed.tasks = extract(m_tasks, [&](const Task &t) { return doomed.contains(t.projectId); });
    // Each record individually: a machine replaying this needs the same set.
    for (const Project &p : removed.projects) markDeleted(RecordKind::Project, p.id, now);
    for (const Section &s : removed.sections) markDeleted(RecordKind::Section, s.id, now);
    for (const Task &t : removed.tasks) markDeleted(RecordKind::Task, t.id, now);
    return removed;
}

void Store::restoreProject(const RemovedProject &removed) {
    for (const Project &p : removed.projects) unmark(RecordKind::Project, p.id);
    for (const Section &s : removed.sections) unmark(RecordKind::Section, s.id);
    for (const Task &t : removed.tasks) unmark(RecordKind::Task, t.id);
    m_projects.append(removed.projects);
    m_sections.append(removed.sections);
    m_tasks.append(removed.tasks);
}

const Project *Store::projectByName(const QString &name) const {
    for (const Project &project : m_projects)
        if (project.name.compare(name, Qt::CaseInsensitive) == 0) return &project;
    return nullptr;
}

// --- sections --------------------------------------------------------------------

const Section *Store::section(const SectionId &id) const {
    for (const Section &section : m_sections)
        if (section.id == id) return &section;
    return nullptr;
}

Section *Store::sectionMut(const SectionId &id) {
    for (Section &section : m_sections)
        if (section.id == id) return &section;
    return nullptr;
}

QList<const Section *> Store::sectionsIn(const ProjectId &project) const {
    QList<const Section *> ordered;
    for (const Section &section : m_sections)
        if (section.projectId == project) ordered.append(&section);
    std::stable_sort(ordered.begin(), ordered.end(), [](const Section *a, const Section *b) {
        const int c = order::compare(a->order, b->order);
        return c != 0 ? c < 0 : a->id < b->id;
    });
    return ordered;
}

SectionId Store::addSection(Section section, const QDateTime &now) {
    section.touch(now);
    const auto siblings = sectionsIn(section.projectId);
    section.order = order::between(siblings.isEmpty() ? QString() : siblings.last()->order, QString());
    unmark(RecordKind::Section, section.id);
    m_sections.append(section);
    return section.id;
}

std::optional<RemovedSection> Store::removeSection(const SectionId &id, const QDateTime &now) {
    for (int i = 0; i < m_sections.size(); ++i) {
        if (m_sections.at(i).id != id) continue;
        RemovedSection removed;
        removed.section = m_sections.takeAt(i);
        removed.project = removed.section.projectId;
        markDeleted(RecordKind::Section, id, now);
        for (Task &task : m_tasks) {
            if (task.sectionId && *task.sectionId == id) {
                task.sectionId.reset();
                task.touch(now);
                removed.tasks.append(task.id);
            }
        }
        return removed;
    }
    return std::nullopt;
}

void Store::restoreSection(const RemovedSection &removed, const QDateTime &now) {
    if (!project(removed.project)) return;
    // Keeps its own key, so an undone deletion lands back where it was.
    unmark(RecordKind::Section, removed.section.id);
    m_sections.append(removed.section);
    for (Task &task : m_tasks) {
        if (task.projectId == removed.project && removed.tasks.contains(task.id)) {
            task.sectionId = removed.section.id;
            task.touch(now);
        }
    }
}

bool Store::renameSection(const SectionId &id, const QString &name, const QDateTime &now) {
    Section *target = sectionMut(id);
    if (!target) return false;
    target->name = name;
    target->touch(now);
    return true;
}

bool Store::moveSection(const SectionId &id, int index, const QDateTime &now) {
    const Section *moving = section(id);
    if (!moving) return false;
    QStringList neighbours;
    for (const Section *s : sectionsIn(moving->projectId))
        if (s->id != id) neighbours << s->order;
    index = std::clamp(index, 0, static_cast<int>(neighbours.size()));
    const QString before = index > 0 ? neighbours.at(index - 1) : QString();
    const QString after = index < neighbours.size() ? neighbours.at(index) : QString();
    Section *target = sectionMut(id);
    target->order = order::between(before, after);
    target->touch(now);
    return true;
}

// --- labels ----------------------------------------------------------------------

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

LabelId Store::labelForName(const QString &name, const QDateTime &now) {
    if (const Label *existing = labelByName(name)) return existing->id;
    QList<Color> used;
    for (const Label &label : m_labels) used.append(label.color);
    Label label = Label::create(name, leastUsedColor(used));
    label.order = static_cast<int>(m_labels.size());
    label.touch(now);
    unmark(RecordKind::Label, label.id);
    m_labels.append(label);
    return label.id;
}

std::optional<Label> Store::removeLabel(const LabelId &id, const QDateTime &now) {
    for (int i = 0; i < m_labels.size(); ++i) {
        if (m_labels.at(i).id != id) continue;
        const Label removed = m_labels.takeAt(i);
        markDeleted(RecordKind::Label, id, now);
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

// --- saved filters ---------------------------------------------------------------

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

FilterId Store::putFilter(SavedFilter filter, const QDateTime &now) {
    filter.touch(now);
    // The undo path as well as the create path, so a deleted filter comes back here.
    unmark(RecordKind::Filter, filter.id);
    for (SavedFilter &existing : m_filters) {
        if (existing.id == filter.id) {
            existing = filter;
            return filter.id;
        }
    }
    int max = -1;
    bool any = false;
    for (const SavedFilter &existing : m_filters) { max = any ? std::max(max, existing.order) : existing.order; any = true; }
    filter.order = any ? max + 1 : 0;
    m_filters.append(filter);
    return filter.id;
}

std::optional<SavedFilter> Store::removeFilter(const FilterId &id, const QDateTime &now) {
    for (int i = 0; i < m_filters.size(); ++i) {
        if (m_filters.at(i).id != id) continue;
        const SavedFilter removed = m_filters.takeAt(i);
        markDeleted(RecordKind::Filter, id, now);
        return removed;
    }
    return std::nullopt;
}

Color Store::nextFilterColor() const {
    QList<Color> used;
    for (const SavedFilter &filter : m_filters) used.append(filter.color);
    return leastUsedColor(used);
}

// --- tasks -----------------------------------------------------------------------

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
    QString last;
    for (const Task &existing : m_tasks)
        if (existing.projectId == task.projectId && existing.sectionId == task.sectionId && (last.isEmpty() || order::compare(existing.order, last) > 0))
            last = existing.order;
    task.order = order::between(last, QString());
    unmark(RecordKind::Task, task.id);
    m_tasks.append(task);
    return task.id;
}

Vocabulary Store::vocabulary() const {
    Vocabulary vocabulary;
    for (const Project &project : m_projects) vocabulary.projects << project.name;
    for (const Section &section : m_sections) vocabulary.sections << section.name;
    for (const Label &label : m_labels) vocabulary.labels << label.name;
    return vocabulary;
}

TaskId Store::addFromQuickAdd(const QuickAdd &parsed, const ProjectId &defaultProject,
                              const std::optional<SectionId> &defaultSection, const QDateTime &now) {
    ProjectId projectId;
    if (parsed.project) {
        if (const Project *named = projectByName(*parsed.project)) projectId = named->id;
    }
    if (projectId.isEmpty()) projectId = project(defaultProject) ? defaultProject : inboxId();

    std::optional<SectionId> sectionId;
    if (parsed.section) {
        for (const Section *section : sectionsIn(projectId))
            if (section->name.compare(*parsed.section, Qt::CaseInsensitive) == 0) sectionId = section->id;
    } else if (defaultSection && !parsed.project) {
        // A default section only applies in its own project.
        if (const Section *s = section(*defaultSection); s && s->projectId == projectId) sectionId = defaultSection;
    }

    QList<LabelId> labels;
    for (const QString &name : parsed.labels) labels.append(labelForName(name, now));

    Task task = Task::create(projectId, parsed.title.trimmed(), now);
    task.sectionId = sectionId;
    task.labels = labels;
    task.due = parsed.due;
    if (parsed.priority) task.priority = *parsed.priority;
    for (qint64 minutes : parsed.reminders) task.reminders.append(Reminder::beforeDue(minutes));
    return addTask(task);
}

static bool byOrder(const Task *a, const Task *b) {
    const int c = order::compare(a->order, b->order);
    if (c != 0) return c < 0;
    if (a->addedAt != b->addedAt) return a->addedAt < b->addedAt;
    return a->id < b->id;
}

QList<const Task *> Store::tasksIn(const ProjectId &project, const std::optional<SectionId> &section) const {
    QList<const Task *> tasks;
    for (const Task &task : m_tasks)
        if (!task.parentId && task.projectId == project && task.sectionId == section) tasks.append(&task);
    std::stable_sort(tasks.begin(), tasks.end(), byOrder);
    return tasks;
}

bool Store::moveTask(const TaskId &id, const ProjectId &projectId, const std::optional<SectionId> &sectionId, int index, const QDateTime &now) {
    if (!task(id) || !project(projectId)) return false;
    if (sectionId) {
        const Section *s = section(*sectionId);
        if (!s || s->projectId != projectId) return false;
    }
    const QList<TaskId> family = taskAndDescendants(id);
    QStringList neighbours;
    for (const Task *other : tasksIn(projectId, sectionId))
        if (other->id != id) neighbours << other->order;
    index = std::clamp(index, 0, static_cast<int>(neighbours.size()));
    const QString landed = order::between(index > 0 ? neighbours.at(index - 1) : QString(), index < neighbours.size() ? neighbours.at(index) : QString());
    for (const TaskId &member : family) {
        if (Task *t = taskMut(member)) {
            t->projectId = projectId;
            if (member == id) {
                t->sectionId = sectionId;
                t->order = landed;
            }
            t->touch(now);
        }
    }
    // Nothing to do about the list it came from: a key says where a task sits
    // relative to its neighbours, not how many there are.
    return true;
}

QList<const Task *> Store::subtasks(const TaskId &parent) const {
    QList<const Task *> children;
    for (const Task &task : m_tasks)
        if (task.parentId && *task.parentId == parent) children.append(&task);
    std::stable_sort(children.begin(), children.end(), byOrder);
    return children;
}

QList<TaskId> Store::taskAndDescendants(const TaskId &root) const {
    QList<TaskId> found{root};
    for (int index = 0; index < found.size(); ++index)
        for (const Task *child : subtasks(found.at(index)))
            if (!found.contains(child->id)) found.append(child->id);
    return found;
}

QList<Task> Store::removeTask(const TaskId &id, const QDateTime &now) {
    const QList<TaskId> doomed = taskAndDescendants(id);
    const QList<Task> removed = extract(m_tasks, [&](const Task &t) { return doomed.contains(t.id); });
    // Every subtask by name: replaying "the parent went" elsewhere would leave
    // the children orphaned rather than gone.
    for (const Task &t : removed) markDeleted(RecordKind::Task, t.id, now);
    return removed;
}

void Store::restoreTasks(const QList<Task> &tasks) {
    for (const Task &t : tasks) unmark(RecordKind::Task, t.id);
    m_tasks.append(tasks);
}

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
