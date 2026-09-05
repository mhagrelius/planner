#include "app.h"

#include "agent.h"
#include "config.h"
#include "dates.h"
#include "demo.h"
#include "palette.h"
#include "present.h"
#include "query.h"
#include "remote.h"
#include "search.h"

#include <QDBusConnection>
#include <QDBusInterface>
#include <QDir>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QProcess>
#include <QStandardPaths>

#include <algorithm>
#include <thread>

using namespace planner;

App *App::s_instance = nullptr;
QMutex App::s_instanceMutex;

namespace {
constexpr int kSyncTickMs = 180 * 1000;
constexpr int kSyncAfterEditMs = 3000;
constexpr int kSyncFailuresBeforeSayingSo = 3;
}

namespace {

QVariantMap hint(const QString &key, const QString &text, const QString &accent = QString()) {
    return {{QStringLiteral("key"), key}, {QStringLiteral("text"), text}, {QStringLiteral("accent"), accent}};
}

QString dueRoleFor(const QString &cls) {
    if (cls == u"overdue") return QStringLiteral("negative");
    if (cls == u"today") return QStringLiteral("positive");
    return QString();
}

QString demoPath() {
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::GenericDataLocation) + QStringLiteral("/planner-demo");
    QDir(dir).removeRecursively();
    return dir + QStringLiteral("/planner.json");
}

} // namespace

App::App(Palette *palette, const Options &options, QObject *parent)
    : QObject(parent), m_palette(palette), m_store(Store::detached()), m_pinnedToday(options.today) {
    s_instance = this;

    QString path = options.dataPath;
    if (options.demo) path = demoPath();
    if (path.isEmpty()) path = Store::defaultPath();
    else if (!path.endsWith(QStringLiteral(".json"))) path += QStringLiteral("/planner.json");
    m_store = Store::openAt(path, &m_loadOutcome);
    if (options.demo) {
        seedDemo(m_store, todayDate(), now());
        m_dirty = true;
    }
    if (m_loadOutcome.kind == LoadOutcome::Recovered)
        toast(QStringLiteral("The planner file could not be read and was set aside as %1").arg(QFileInfo(m_loadOutcome.backup).fileName()), false);
    if (m_loadOutcome.kind == LoadOutcome::ReadOnly)
        m_saveError = QStringLiteral("this file is from a newer Planner (v%1) and is open read-only").arg(m_loadOutcome.version);

    if (!options.screen.isEmpty()) go(options.screen);

    // Anything already overdue at startup is marked as seen rather than shown.
    m_schedule.catchUp(m_store, now(), QTimeZone::systemTimeZone());
    m_reminderTick.setInterval(30 * 1000);
    connect(&m_reminderTick, &QTimer::timeout, this, &App::fireReminders);
    m_reminderTick.start();

    m_saveTick.setInterval(2000);
    connect(&m_saveTick, &QTimer::timeout, this, &App::saveNow);
    m_saveTick.start();

    m_toastTimer.setSingleShot(true);
    m_toastTimer.setInterval(6000);
    connect(&m_toastTimer, &QTimer::timeout, this, [this]() { m_toast.clear(); recompute(); });

    connect(palette, &Palette::changed, this, [this]() { recompute(); });
    readSummonKey();
    recompute();
    for (const QString &name : options.acts) act(name);
    // Scratch stores stay off the server: a demo must never push its sample
    // tasks into the real list.
    if (!options.demo) startSync();
}

App::~App() {
    saveNow();
    QMutexLocker lock(&s_instanceMutex);
    s_instance = nullptr;
}

App *App::create(QQmlEngine *, QJSEngine *) {
    Q_ASSERT(s_instance);
    QJSEngine::setObjectOwnership(s_instance, QJSEngine::CppOwnership);
    return s_instance;
}

QDate App::todayDate() const { return m_pinnedToday.isValid() ? m_pinnedToday : QDate::currentDate(); }

QDateTime App::now() const {
    if (m_pinnedToday.isValid()) return QDateTime(m_pinnedToday, QTime(12, 0), QTimeZone::utc());
    return QDateTime::currentDateTimeUtc();
}

// Every mutation goes through here so no caller can change a task and forget
// to flag it.
template <typename F>
void App::mutate(F change) {
    change(m_store);
    m_dirty = true;
    // Something changed here; get it to the other machines. A pull being
    // applied goes through here too, and must not be pushed straight back.
    if (!m_applyingSync) syncAfterEdit();
}

// Run on the main thread, from a worker, if the App is still there.
template <typename F>
void App::onMainThread(F functor) {
    QMutexLocker lock(&s_instanceMutex);
    if (s_instance) QMetaObject::invokeMethod(s_instance, functor, Qt::QueuedConnection);
}

void App::saveNow() {
    if (!m_dirty) return;
    const auto error = m_store.save();
    if (!error) {
        m_dirty = false;
        if (!m_saveError.isEmpty()) {
            m_saveError.clear();
            recompute();
        }
        return;
    }
    // Stay dirty: the next tick tries again, so a transient failure heals itself.
    if (m_saveError != error->message) {
        m_saveError = error->message;
        recompute();
    }
}

std::pair<QString, bool> App::agentCommand(const QStringList &args) {
    const agent::Result result = agent::run(m_store, args, now(), todayDate());
    // Written out now rather than left to the tick: a caller has been told the
    // change happened, and "it is in the memory of a process you cannot see"
    // is not that.
    if (result.changedStore) {
        m_dirty = true;
        saveNow();
        syncAfterEdit();
    }
    recompute();
    return {agent::render(result), result.ok};
}

// --- sync -------------------------------------------------------------------------

// Sync is off until a URL and a token are written in the config on purpose:
// there is nothing sensible to guess at, and a planner that will not open
// because a NAS is down is worse than one that does not sync.
void App::startSync() {
    const Config config = Config::load();
    m_syncTarget = config.syncTarget();
    if (!m_syncTarget) return;
    m_syncBase = sync::loadBase(sync::defaultBasePath(m_store.path()));
    // A first pass shortly after startup, then a backstop tick; the long poll
    // does the rest.
    m_syncTick.setInterval(kSyncTickMs);
    connect(&m_syncTick, &QTimer::timeout, this, &App::syncNow);
    m_syncTick.start();
    m_syncSoon.setSingleShot(true);
    m_syncSoon.setInterval(kSyncAfterEditMs);
    connect(&m_syncSoon, &QTimer::timeout, this, &App::syncNow);
    syncNow();
}

// One pass: network on a worker, every local write back here. The worker is
// handed a snapshot and the bodies it needs and gives back records; nothing
// below this opens the planner file.
void App::syncNow() {
    if (!m_syncTarget || m_syncing) return;
    QString parseError;
    auto remote = HttpRemote::parse(m_syncTarget->first, m_syncTarget->second, &parseError);
    if (!remote) {
        reportSyncFailure(parseError);
        return;
    }
    const sync::Snapshot base = m_syncBase;
    const sync::Snapshot local = sync::snapshotOf(m_store);
    QMap<sync::Key, QJsonObject> bodies;
    for (auto it = local.begin(); it != local.end(); ++it)
        if (const auto body = m_store.recordBody(it.key().kind, it.key().id)) bodies.insert(it.key(), *body);
    m_syncing = true;
    std::thread([remote = *remote, base, local, bodies]() mutable {
        sync::Error error;
        const auto incoming = sync::gather(remote, base, local, [&](const sync::Key &key) -> std::optional<QJsonObject> {
            const auto it = bodies.constFind(key);
            if (it == bodies.constEnd()) return std::nullopt;
            return it.value();
        }, &error);
        const QString message = error.message;
        onMainThread([incoming, message]() { s_instance->finishSync(incoming, message); });
    }).detach();
}

void App::finishSync(const std::optional<sync::Incoming> &incoming, const QString &error) {
    m_syncing = false;
    if (!incoming) {
        reportSyncFailure(error);
        return;
    }
    // A pull landing on the task open in the detail pane would take the text
    // out from under the cursor; held back, and offered again next pass.
    const QString open = m_openTask;
    const auto held = [open](const sync::Key &key) { return !open.isEmpty() && key.kind == RecordKind::Task && key.id == open; };
    m_applyingSync = true;
    sync::Report report;
    mutate([&](Store &store) { std::tie(report, m_syncBase) = sync::apply(store, *incoming, held); });
    m_applyingSync = false;
    sync::saveBase(m_syncBase, sync::defaultBasePath(m_store.path()));
    m_syncFailures = 0;
    m_syncLastPass = QDateTime::currentDateTimeUtc();
    m_syncLastFailure.clear();
    m_syncError.clear();
    // Nothing at all on a clean pass. Sync is awareness, not applause.
    recompute();
    waitForChanges();
}

// Park a worker on the server until another machine writes, so an edit made
// elsewhere arrives in about as long as the network takes.
void App::waitForChanges() {
    if (!m_syncTarget || m_syncing) return;
    QString parseError;
    auto remote = HttpRemote::parse(m_syncTarget->first, m_syncTarget->second, &parseError);
    if (!remote) return;
    m_syncing = true;
    const QDateTime since = m_syncCursor.value_or(epoch());
    std::thread([remote = *remote, since]() mutable {
        sync::Error error;
        const auto answer = remote.waitForChange(since, &error);
        const bool ok = answer.has_value();
        const bool changed = ok && answer->first;
        const QDateTime cursor = ok ? answer->second : since;
        onMainThread([ok, changed, cursor]() { s_instance->finishWait(ok, changed, cursor); });
    }).detach();
}

void App::finishWait(bool ok, bool changed, const QDateTime &cursor) {
    m_syncing = false;
    // A failed wait says nothing: the backstop tick is what a NAS that went
    // away is for, and a banner per failed wait would be one a minute.
    if (!ok) return;
    m_syncCursor = cursor;
    if (changed) syncNow();
    else waitForChanges();
}

// Debounced: typing a title fires this on every keystroke.
void App::syncAfterEdit() {
    if (!m_syncTarget || m_applyingSync) return;
    m_syncSoon.start();
}

// Not reported the first time: a NAS asleep, a laptop between networks and a
// suspended machine all produce one failed pass. It becomes an ongoing
// condition worth a line once it has kept failing.
void App::reportSyncFailure(const QString &message) {
    m_syncFailures += 1;
    m_syncLastFailure = message;
    if (m_syncFailures >= kSyncFailuresBeforeSayingSo && m_syncError != message) {
        m_syncError = message;
        recompute();
    }
}

static QString ago(const QDateTime &when) {
    const qint64 seconds = when.secsTo(QDateTime::currentDateTimeUtc());
    if (seconds < 60) return QStringLiteral("just now");
    if (seconds < 3600) return QStringLiteral("%1 min ago").arg(seconds / 60);
    if (seconds < 86400) return QStringLiteral("%1 h ago").arg(seconds / 3600);
    return when.toLocalTime().toString(QStringLiteral("d MMM HH:mm"));
}

QJsonObject App::syncStatusJson() const {
    QJsonObject json;
    json.insert(QStringLiteral("configured"), m_syncTarget.has_value());
    if (m_syncTarget) json.insert(QStringLiteral("server"), m_syncTarget->first);
    json.insert(QStringLiteral("config"), Config::defaultPath());
    json.insert(QStringLiteral("file"), m_store.path());
    const int here = sync::liveCount(sync::snapshotOf(m_store));
    json.insert(QStringLiteral("records"), here);
    if (m_syncTarget) {
        json.insert(QStringLiteral("agreed"), sync::liveCount(m_syncBase));
        json.insert(QStringLiteral("busy"), m_syncing);
        if (m_syncLastPass) json.insert(QStringLiteral("last_pass"), instantSerial(*m_syncLastPass));
        if (!m_syncLastFailure.isEmpty()) json.insert(QStringLiteral("last_failure"), m_syncLastFailure);
    }
    return json;
}

void App::showSyncStatus() {
    m_promptRows.clear();
    const int here = sync::liveCount(sync::snapshotOf(m_store));
    auto row = [&](const QString &k, const QString &v) { m_promptRows.append(QVariantMap{{QStringLiteral("k"), k}, {QStringLiteral("v"), v}}); };
    row(QStringLiteral("server"), m_syncTarget ? m_syncTarget->first : QStringLiteral("not set up — this planner stays on this machine"));
    row(QStringLiteral("records here"), countOf(here).replace(QStringLiteral("task"), QStringLiteral("record")));
    if (m_syncTarget) {
        // How many records this machine and the server last agreed on. Short
        // of the local count means work left, not something broken.
        const int agreed = sync::liveCount(m_syncBase);
        row(QStringLiteral("synced"), here == 0 && agreed == 0 ? QStringLiteral("nothing to sync yet") : agreed == 0 ? QStringLiteral("not yet — the first pass has not finished")
                                     : agreed >= here ? QStringLiteral("all %1").arg(here) : QStringLiteral("%1 of %2, the rest on the next pass").arg(agreed).arg(here));
        row(QStringLiteral("last pass"), !m_syncLastFailure.isEmpty() ? QStringLiteral("failed — %1").arg(m_syncLastFailure)
                                        : m_syncLastPass ? ago(*m_syncLastPass) : m_syncing ? QStringLiteral("running now") : QStringLiteral("not since Planner was opened"));
    }
    row(QStringLiteral("file"), m_store.path());
    m_prompt = QStringLiteral("status");
    m_promptTitle = !m_syncTarget ? QStringLiteral("Syncing is off. Set sync_url and sync_token in %1 to share this planner between machines.").arg(Config::defaultPath())
                  : !m_syncLastFailure.isEmpty() ? QStringLiteral("Nothing here is at risk — the copy on this machine is the one that counts, and the next pass will try again.")
                  : QStringLiteral("A pass runs when an edit settles, and the server holds a request open so a change made elsewhere arrives as it happens.");
    m_promptQuery.clear();
    recompute();
}

std::pair<QString, bool> App::syncCommand(const QStringList &args) {
    const QString verb = args.value(1, QStringLiteral("status"));
    if (verb == u"now") {
        if (!m_syncTarget) return {QStringLiteral("{\"ok\": false, \"error\": \"not-configured\", \"message\": \"Set sync_url and sync_token in %1.\"}").arg(Config::defaultPath()), false};
        syncNow();
        QJsonObject json = syncStatusJson();
        json.insert(QStringLiteral("ok"), true);
        json.insert(QStringLiteral("started"), true);
        return {QString::fromUtf8(QJsonDocument(json).toJson(QJsonDocument::Indented)).trimmed(), true};
    }
    QJsonObject json = syncStatusJson();
    json.insert(QStringLiteral("ok"), true);
    return {QString::fromUtf8(QJsonDocument(json).toJson(QJsonDocument::Indented)).trimmed(), true};
}

// --- the Hyprland summon binding ------------------------------------------------

// The rail shows the key that summons the window, if the user bound one. It is
// read from Hyprland rather than assumed, so the keycap is never a lie.
void App::readSummonKey() {
    if (qEnvironmentVariableIsEmpty("HYPRLAND_INSTANCE_SIGNATURE")) return;
    QProcess hyprctl;
    hyprctl.start(QStringLiteral("hyprctl"), {QStringLiteral("binds"), QStringLiteral("-j")});
    if (!hyprctl.waitForFinished(400)) return;
    const QJsonArray binds = QJsonDocument::fromJson(hyprctl.readAllStandardOutput()).array();
    for (const QJsonValue &value : binds) {
        const QJsonObject bind = value.toObject();
        const QString arg = bind.value(QStringLiteral("arg")).toString();
        if (!arg.contains(QStringLiteral("planner"))) continue;
        const int mods = bind.value(QStringLiteral("modmask")).toInt();
        QStringList parts;
        if (mods & 64) parts << QStringLiteral("SUPER");
        if (mods & 4) parts << QStringLiteral("CTRL");
        if (mods & 8) parts << QStringLiteral("ALT");
        if (mods & 1) parts << QStringLiteral("SHIFT");
        parts << bind.value(QStringLiteral("key")).toString().toUpper();
        m_summonKey = parts.join(u' ');
        return;
    }
}

// --- navigation ---------------------------------------------------------------

std::optional<View> App::currentView() const {
    for (const View &view : builtinViews())
        if (view.id == m_viewId) return view;
    for (const View &view : filterViews(m_store))
        if (view.id == m_viewId) return view;
    for (const View &view : projectViews(m_store))
        if (view.id == m_viewId) return view;
    return std::nullopt;
}

void App::go(const QString &viewId) {
    QString target = viewId;
    // `project:Work` by name, for --screen and the palette.
    if (target.startsWith(QStringLiteral("project:")) && !m_store.project(target.mid(8))) {
        if (const Project *project = m_store.projectByName(target.mid(8))) target = QStringLiteral("project:") + project->id;
    }
    if (target == m_viewId) return;
    m_viewId = target;
    // A selection belongs to the list it was made in, and so does the open
    // task: the pane would otherwise show something the new view has no row for.
    m_selecting = false;
    m_selection.clear();
    m_openTask.clear();
    m_cursor = 0;
    m_boardColumn = 0;
    m_cursorWanted.clear();
    recompute();
}

void App::goToKey(int key) {
    const QList<View> views = builtinViews();
    if (key >= 1 && key <= views.size()) go(views.at(key - 1).id);
}

void App::toggleRail() {
    m_railVisible = !m_railVisible;
    recompute();
}

void App::moveCursor(int delta) {
    if (m_rowIds.isEmpty()) return;
    if (m_board) {
        // Within the column: find rows in the cursor's lane.
        const int lane = m_cursor < m_rowPlaces.size() ? m_rowPlaces.at(m_cursor).first : m_boardColumn;
        QList<int> inLane;
        for (int i = 0; i < m_rowPlaces.size(); ++i)
            if (m_rowPlaces.at(i).first == lane) inLane.append(i);
        if (inLane.isEmpty()) return;
        int position = static_cast<int>(inLane.indexOf(m_cursor));
        if (position < 0) position = 0;
        position = std::clamp(position + delta, 0, static_cast<int>(inLane.size()) - 1);
        m_cursor = inLane.at(position);
    } else {
        m_cursor = std::clamp(m_cursor + delta, 0, static_cast<int>(m_rowIds.size()) - 1);
    }
    m_cursorWanted = m_rowIds.at(m_cursor);
    followCursor();
    recompute();
}

void App::cursorTo(int flat) {
    if (flat < 0 || flat >= m_rowIds.size()) return;
    m_cursor = flat;
    m_cursorWanted = m_rowIds.at(flat);
    if (m_board) m_boardColumn = m_rowPlaces.at(flat).first;
    followCursor();
    recompute();
}

// While the pane is open it shows the cursor row, so browsing with the arrow
// keys reads each task in turn rather than leaving the first one on screen.
void App::followCursor() {
    if (m_openTask.isEmpty()) return;
    if (const auto id = cursorId()) m_openTask = *id;
}

void App::moveColumn(int delta) {
    if (!m_board || m_laneSections.isEmpty()) return;
    const int column = std::clamp(m_boardColumn + delta, 0, static_cast<int>(m_laneSections.size()) - 1);
    if (column == m_boardColumn) return;
    m_boardColumn = column;
    // Keep the row position where the column allows it.
    const int index = m_cursor < m_rowPlaces.size() ? m_rowPlaces.at(m_cursor).second : 0;
    int best = -1;
    for (int i = 0; i < m_rowPlaces.size(); ++i) {
        if (m_rowPlaces.at(i).first != column) continue;
        if (best < 0 || m_rowPlaces.at(i).second <= index) best = i;
    }
    if (best >= 0) {
        m_cursor = best;
        m_cursorWanted = m_rowIds.at(best);
    }
    followCursor();
    recompute();
}

bool App::escape() {
    if (m_picker.open) { closePicker(); return true; }
    if (!m_prompt.isEmpty()) { closePrompt(); return true; }
    if (m_selecting) { clearSelection(); return true; }
    if (!m_openTask.isEmpty()) { closeDetail(); return true; }
    return false;
}

// --- the cursor row --------------------------------------------------------------

std::optional<TaskId> App::cursorId() const {
    if (m_cursor < 0 || m_cursor >= m_rowIds.size()) return std::nullopt;
    return m_rowIds.at(m_cursor);
}

QList<TaskId> App::targets() const {
    if (m_selecting && !m_selection.isEmpty()) return m_selection;
    if (const auto id = cursorId()) return {*id};
    return {};
}

void App::space() {
    const auto id = cursorId();
    if (!id) return;
    if (m_selecting) {
        toggleSelected(*id);
        return;
    }
    toggleTask(*id);
}

void App::toggleTask(const QString &id) {
    const Task *task = m_store.task(id);
    if (!task) return;
    if (task->checked) {
        mutate([&](Store &store) { store.uncompleteTask(id, now()); });
        toast(QStringLiteral("Reopened"), false);
    } else {
        std::optional<Completion> outcome;
        mutate([&](Store &store) { outcome = store.completeTask(id, now(), todayDate()); });
        if (outcome && outcome->kind == Completion::Rescheduled) {
            // A repeating task keeps its reminders, so the next occurrence
            // has to be allowed to fire them again.
            m_schedule.forget(id);
            toast(QStringLiteral("Repeats — next on %1").arg(formatDate(outcome->due->date, todayDate())), false);
        } else if (outcome && outcome->kind == Completion::Done) {
            m_undo = Undo{QStringLiteral("completed"), {id}, {}, std::nullopt, std::nullopt};
            toast(QStringLiteral("Task completed"), true);
        }
    }
    m_cursorWanted = id;
    recompute();
}

void App::enter() {
    if (const auto id = cursorId()) openTaskId(*id);
}

void App::openTaskId(const QString &id) {
    if (!m_store.task(id)) return;
    m_openTask = id;
    recompute();
}

void App::closeDetail() {
    m_openTask.clear();
    recompute();
    // A pull held back for the open task is offered again now.
    if (m_syncTarget && !m_syncing) syncNow();
}

void App::pinCursor() {
    const QList<TaskId> ids = targets();
    if (ids.isEmpty()) return;
    // Pin them all, or unpin them all if every one already is.
    bool allPinned = true;
    for (const TaskId &id : ids)
        if (const Task *task = m_store.task(id)) allPinned = allPinned && task->pinned;
    mutate([&](Store &store) {
        for (const TaskId &id : ids) {
            if (Task *task = store.taskMut(id)) {
                task->pinned = !allPinned;
                task->touch(now());
            }
        }
    });
    toast(allPinned ? QStringLiteral("Unpinned %1").arg(countOf(ids.size())) : QStringLiteral("Pinned %1").arg(countOf(ids.size())), false);
    recompute();
}

void App::completeIds(const QList<TaskId> &ids) {
    QList<TaskId> completed;
    mutate([&](Store &store) {
        for (const TaskId &id : ids) {
            const Task *task = store.task(id);
            if (!task || task->checked) continue;
            // A repeating task moves on rather than finishing, so undoing the
            // batch must not reopen it — it was never closed.
            const auto outcome = store.completeTask(id, now(), todayDate());
            if (outcome && outcome->kind == Completion::Done) completed.append(id);
            else if (outcome && outcome->kind == Completion::Rescheduled) m_schedule.forget(id);
        }
    });
    m_undo = Undo{QStringLiteral("completed"), completed, {}, std::nullopt, std::nullopt};
    toast(QStringLiteral("Completed %1").arg(countOf(completed.size())), !completed.isEmpty());
    m_selecting = false;
    m_selection.clear();
    recompute();
}

void App::deleteIds(const QList<TaskId> &ids) {
    QList<Task> removed;
    mutate([&](Store &store) {
        for (const TaskId &id : ids) removed.append(store.removeTask(id, now()));
    });
    if (removed.isEmpty()) return;
    m_undo = Undo{QStringLiteral("deleted"), {}, removed, std::nullopt, std::nullopt};
    toast(QStringLiteral("Deleted %1").arg(countOf(removed.size())), true);
    if (!m_openTask.isEmpty() && !m_store.task(m_openTask)) m_openTask.clear();
    m_selecting = false;
    m_selection.clear();
    recompute();
}

void App::deleteKey() {
    const QList<TaskId> ids = targets();
    if (!ids.isEmpty()) deleteIds(ids);
}

void App::moveTaskVertical(int delta) {
    const auto id = cursorId();
    if (!id || !m_isProject || m_cursor >= m_rowPlaces.size()) return;
    const Task *task = m_store.task(*id);
    const auto [lane, index] = m_rowPlaces.at(m_cursor);
    const int target = std::max(0, index + delta);
    mutate([&](Store &store) { store.moveTask(*id, task->projectId, task->sectionId, target, now()); });
    m_cursorWanted = *id;
    recompute();
}

void App::moveTaskColumn(int delta) {
    const auto id = cursorId();
    if (!id || !m_isProject || m_cursor >= m_rowPlaces.size()) return;
    const int lane = std::clamp(m_rowPlaces.at(m_cursor).first + delta, 0, static_cast<int>(m_laneSections.size()) - 1);
    if (lane == m_rowPlaces.at(m_cursor).first) return;
    const Task *task = m_store.task(*id);
    const ProjectId project = task->projectId;
    mutate([&](Store &store) { store.moveTask(*id, project, m_laneSections.at(lane), 1 << 30, now()); });
    m_boardColumn = lane;
    m_cursorWanted = *id;
    recompute();
}

// --- selection --------------------------------------------------------------------

void App::selectAll() {
    m_selecting = true;
    m_selection = m_rowIds;
    recompute();
}

void App::clearSelection() {
    m_selecting = false;
    m_selection.clear();
    recompute();
}

void App::toggleSelected(const QString &id) {
    m_selecting = true;
    if (m_selection.contains(id)) m_selection.removeAll(id);
    else m_selection.append(id);
    m_cursorWanted = id;
    recompute();
}

void App::setPriorityKey(int level) {
    if (level < 1 || level > 4) return;
    const Priority priority = kAllPriorities[level - 1];
    const QList<TaskId> ids = targets();
    if (ids.isEmpty()) return;
    int changed = 0;
    mutate([&](Store &store) {
        for (const TaskId &id : ids) {
            Task *task = store.taskMut(id);
            if (task && task->priority != priority) {
                task->priority = priority;
                task->touch(now());
                ++changed;
            }
        }
    });
    // Priority reports a plain count with no undo: retyping the old level is
    // one keystroke.
    toast(QStringLiteral("%1 set to %2").arg(countOf(changed), priorityToken(priority)), false);
    recompute();
}

void App::undo() {
    if (!m_undo) return;
    const Undo undo = *m_undo;
    m_undo.reset();
    if (undo.kind == u"completed") {
        mutate([&](Store &store) { for (const TaskId &id : undo.ids) store.uncompleteTask(id, now()); });
        toast(QStringLiteral("Reopened %1").arg(countOf(undo.ids.size())), false);
    } else if (undo.kind == u"deleted") {
        mutate([&](Store &store) { store.restoreTasks(undo.tasks); });
        toast(QStringLiteral("Restored %1").arg(countOf(undo.tasks.size())), false);
    } else if (undo.kind == u"section" && undo.section) {
        mutate([&](Store &store) { store.restoreSection(*undo.section, now()); });
        toast(QStringLiteral("Section restored"), false);
    } else if (undo.kind == u"project" && undo.project) {
        mutate([&](Store &store) { store.restoreProject(*undo.project); });
        toast(QStringLiteral("Project restored"), false);
    }
    recompute();
}

// --- the detail pane -------------------------------------------------------------

void App::setTitle(const QString &id, const QString &text) {
    const QString trimmed = text.trimmed();
    if (trimmed.isEmpty()) return;
    mutate([&](Store &store) {
        if (Task *task = store.taskMut(id); task && task->content != trimmed) {
            task->content = trimmed;
            task->touch(now());
        }
    });
    recompute();
}

void App::setDescription(const QString &id, const QString &text) {
    mutate([&](Store &store) {
        if (Task *task = store.taskMut(id); task && task->description != text) {
            task->description = text;
            task->touch(now());
        }
    });
    recompute();
}

void App::cyclePriority(const QString &id) {
    const Task *task = m_store.task(id);
    if (!task) return;
    // p4 → p1 → p2 → p3 → p4, most urgent first after unset.
    const Priority next = task->priority == Priority::P4 ? Priority::P1 : task->priority == Priority::P3 ? Priority::P4 : kAllPriorities[priorityRank(task->priority) + 1];
    setPriority(id, priorityToken(next));
}

void App::setPriority(const QString &id, const QString &token) {
    const auto priority = priorityFromToken(token);
    if (!priority) return;
    mutate([&](Store &store) {
        if (Task *task = store.taskMut(id); task && task->priority != *priority) {
            task->priority = *priority;
            task->touch(now());
        }
    });
    recompute();
}

void App::toggleLabel(const QString &id, const QString &name, bool on) {
    const QString trimmed = name.trimmed();
    if (trimmed.isEmpty()) return;
    mutate([&](Store &store) {
        const LabelId label = store.labelForName(trimmed, now());
        if (Task *task = store.taskMut(id)) {
            if (on) task->addLabel(label);
            else task->removeLabel(label);
            task->touch(now());
        }
    });
    recompute();
}

void App::addSubtask(const QString &parentId, const QString &line) {
    if (line.trimmed().isEmpty()) return;
    const Task *parent = m_store.task(parentId);
    if (!parent) return;
    const ProjectId project = parent->projectId;
    const std::optional<SectionId> section = parent->sectionId;
    mutate([&](Store &store) {
        const QuickAdd parsed = parseQuickAdd(line, todayDate(), store.vocabulary());
        if (parsed.title.trimmed().isEmpty()) return;
        const TaskId id = store.addFromQuickAdd(parsed, project, section, now());
        // A subtask shares its parent's project whatever the line says.
        Task *child = store.taskMut(id);
        child->parentId = parentId;
        child->projectId = project;
    });
    recompute();
}

void App::addNote(const QString &id, const QString &text) {
    bool added = false;
    mutate([&](Store &store) { added = store.addNote(id, text, now()); });
    if (added) recompute();
}

void App::removeNote(const QString &id, const QString &at) {
    const auto when = instantFromSerial(at);
    if (!when) return;
    mutate([&](Store &store) { store.removeNote(id, *when, now()); });
    recompute();
}

// --- prompts ----------------------------------------------------------------------

void App::openPrompt(const QString &kind) {
    m_prompt = kind;
    m_promptQuery.clear();
    m_promptIndex = 0;
    m_promptError.clear();
    if (kind != u"input" && kind != u"confirm") {
        m_promptTitle.clear();
        m_promptPlaceholder = kind == u"add" ? QStringLiteral("Email Sam about the lease #Work @email p2 friday 9am")
                            : kind == u"find" ? QStringLiteral("tasks, projects and labels") : QStringLiteral("type an action, a view or a task");
    }
    recompute();
}

void App::closePrompt() {
    m_prompt.clear();
    m_promptQuery.clear();
    m_promptIndex = 0;
    m_inputAction.clear();
    m_inputPayload.clear();
    m_promptError.clear();
    m_keepAdding = false;
    m_promptRows.clear();
    recompute();
}

void App::setPromptQuery(const QString &text) {
    if (m_promptQuery == text) return;
    m_promptQuery = text;
    m_promptIndex = 0;
    recompute();
}

void App::movePrompt(int delta) {
    int items = 0;
    for (const QVariant &entry : m_promptResults)
        if (entry.toMap().value(QStringLiteral("kind")).toString() != u"heading") ++items;
    if (items == 0) return;
    m_promptIndex = std::clamp(m_promptIndex + delta, 0, items - 1);
    recompute();
}

void App::promptTo(int index) {
    m_promptIndex = std::max(0, index);
    recompute();
}

void App::beginInput(const QString &action, const QString &title, const QString &placeholder, const QString &prefill, const QVariantMap &payload) {
    m_prompt = QStringLiteral("input");
    m_inputAction = action;
    m_inputPayload = payload;
    m_promptTitle = title;
    m_promptPlaceholder = placeholder;
    m_promptQuery = prefill;
    m_promptIndex = 0;
    m_promptError.clear();
    recompute();
}

void App::finishInput() {
    const QString text = m_promptQuery.trimmed();
    const QString action = m_inputAction;
    const QVariantMap payload = m_inputPayload;
    if (action == u"new-section") {
        if (text.isEmpty()) return;
        const ProjectId project = payload.value(QStringLiteral("project")).toString();
        mutate([&](Store &store) { if (store.project(project)) store.addSection(Section::create(project, text), now()); });
        toast(QStringLiteral("Section “%1” added").arg(text), false);
    } else if (action == u"rename-section") {
        if (text.isEmpty()) return;
        mutate([&](Store &store) { store.renameSection(payload.value(QStringLiteral("section")).toString(), text, now()); });
    } else if (action == u"new-project") {
        if (text.isEmpty()) return;
        std::optional<ProjectId> parent;
        if (payload.contains(QStringLiteral("parent"))) parent = payload.value(QStringLiteral("parent")).toString();
        ProjectId id;
        mutate([&](Store &store) {
            Project project = Project::create(text, store.nextProjectColor());
            project.parentId = parent;
            id = store.addProject(project, now());
        });
        closePrompt();
        go(QStringLiteral("project:") + id);
        return;
    } else if (action == u"rename-project") {
        if (text.isEmpty()) return;
        mutate([&](Store &store) {
            if (Project *project = store.projectMut(payload.value(QStringLiteral("project")).toString())) {
                project->name = text;
                project->touch(now());
            }
        });
    } else if (action == u"filter-query") {
        // A saved filter that will not parse matches nothing, and the editor
        // says why while you type; Enter on a broken one changes nothing.
        QueryError error;
        if (!Query::parse(text, &error)) {
            m_promptError = text.isEmpty() ? QStringLiteral("a query is needed") : error.message;
            recompute();
            return;
        }
        QVariantMap next = payload;
        next.insert(QStringLiteral("query"), text);
        beginInput(QStringLiteral("filter-name"), QStringLiteral("name the filter"), QStringLiteral("Errands"), payload.value(QStringLiteral("name")).toString(), next);
        return;
    } else if (action == u"filter-name") {
        if (text.isEmpty()) return;
        FilterId id;
        mutate([&](Store &store) {
            SavedFilter filter = SavedFilter::create(text, payload.value(QStringLiteral("query")).toString(), store.nextFilterColor());
            if (payload.contains(QStringLiteral("filter"))) {
                if (const SavedFilter *existing = store.filter(payload.value(QStringLiteral("filter")).toString())) {
                    filter.id = existing->id;
                    filter.color = existing->color;
                    filter.order = existing->order;
                }
            }
            id = store.putFilter(filter, now());
        });
        closePrompt();
        go(QStringLiteral("filter:") + id);
        return;
    } else if (action == u"confirm-delete-project") {
        const ProjectId project = payload.value(QStringLiteral("project")).toString();
        std::optional<RemovedProject> removed;
        mutate([&](Store &store) { removed = store.removeProject(project, now()); });
        if (removed) {
            m_undo = Undo{QStringLiteral("project"), {}, {}, std::nullopt, removed};
            toast(QStringLiteral("Deleted the project and %1").arg(countOf(removed->tasks.size())), true);
        }
        closePrompt();
        if (m_viewId == QStringLiteral("project:") + project) go(QStringLiteral("today"));
        return;
    }
    closePrompt();
}

void App::runPrompt() {
    if (m_prompt == u"add") {
        const QString line = m_promptQuery;
        const QuickAdd parsed = parseQuickAdd(line, todayDate(), m_store.vocabulary());
        if (parsed.title.trimmed().isEmpty()) return;
        // The new task lands in the project being looked at, or the Inbox.
        ProjectId project = inboxId();
        if (const auto view = currentView(); view && view->projectId()) project = *view->projectId();
        std::optional<SectionId> section;
        if (m_isProject && m_cursor < m_rowPlaces.size()) section = m_laneSections.value(m_rowPlaces.at(m_cursor).first);
        TaskId id;
        mutate([&](Store &store) { id = store.addFromQuickAdd(parseQuickAdd(line, todayDate(), store.vocabulary()), project, section, now()); });
        const bool keep = m_keepAdding;
        if (keep) {
            m_promptQuery.clear();
            m_keepAdding = true;
            toast(QStringLiteral("Added “%1”").arg(parsed.title.trimmed()), false);
            recompute();
        } else {
            closePrompt();
            m_cursorWanted = id;
            recompute();
        }
        return;
    }
    if (m_prompt == u"input") { finishInput(); return; }
    if (m_prompt == u"confirm") { finishInput(); return; }
    if (m_prompt == u"status") { closePrompt(); return; }
    // Palette and find: run the highlighted result.
    int seen = 0;
    for (const QVariant &entry : m_promptResults) {
        const QVariantMap item = entry.toMap();
        if (item.value(QStringLiteral("kind")).toString() == u"heading") continue;
        if (seen++ == m_promptIndex) {
            runPaletteItem(item);
            return;
        }
    }
}

void App::submitKeepAdding() {
    if (m_prompt != u"add") return;
    m_keepAdding = true;
    runPrompt();
}

void App::runPaletteItem(const QVariantMap &item) {
    const QString kind = item.value(QStringLiteral("kind")).toString();
    const QString id = item.value(QStringLiteral("id")).toString();
    if (kind == u"task") {
        closePrompt();
        // Show the task where it lives, then open it.
        if (const Task *task = m_store.task(id)) {
            const QString view = QStringLiteral("project:") + task->projectId;
            if (task->projectId == inboxId()) go(QStringLiteral("inbox"));
            else if (!currentView() || !m_rowIds.contains(id)) go(view);
        }
        m_cursorWanted = id;
        m_openTask = id;
        recompute();
        return;
    }
    if (kind == u"view") { closePrompt(); go(id); return; }
    if (kind == u"project") { closePrompt(); go(QStringLiteral("project:") + id); return; }
    if (kind == u"label") { closePrompt(); beginInput(QStringLiteral("filter-query"), QStringLiteral("save a filter"), QStringLiteral("@errand | @town"), QLatin1Char('@') + item.value(QStringLiteral("title")).toString(), {}); return; }
    if (kind != u"action") return;

    const QString action = id;
    const auto view = currentView();
    const std::optional<ProjectId> project = view ? view->projectId() : std::nullopt;
    const std::optional<FilterId> filter = view ? view->filterId() : std::nullopt;
    if (action == u"new-task") { openPrompt(QStringLiteral("add")); return; }
    if (action == u"find") { openPrompt(QStringLiteral("find")); return; }
    if (action == u"select") { closePrompt(); selectAll(); return; }
    if (action == u"toggle-style") { closePrompt(); toggleStyle(); return; }
    if (action == u"toggle-rail") { closePrompt(); toggleRail(); return; }
    if (action == u"pin") { closePrompt(); pinCursor(); return; }
    if (action == u"complete") { closePrompt(); completeIds(targets()); return; }
    if (action == u"delete-task") { closePrompt(); deleteKey(); return; }
    if (action == u"date") { closePrompt(); openDatePicker(); return; }
    if (action == u"deadline") { closePrompt(); openDeadlinePicker(); return; }
    if (action == u"undo") { closePrompt(); undo(); return; }
    if (action == u"sync-status") { closePrompt(); showSyncStatus(); return; }
    if (action == u"sync-now") { closePrompt(); syncNow(); toast(QStringLiteral("Syncing with %1").arg(m_syncTarget ? m_syncTarget->first : QString()), false); return; }
    if (action == u"new-section" && project) { beginInput(action, QStringLiteral("new section in #%1").arg(m_store.project(*project)->name), QStringLiteral("In progress"), {}, {{QStringLiteral("project"), *project}}); return; }
    if (action == u"rename-section") {
        const SectionId section = item.value(QStringLiteral("section")).toString();
        beginInput(action, QStringLiteral("rename the section"), {}, m_store.section(section)->name, {{QStringLiteral("section"), section}});
        return;
    }
    if (action == u"delete-section") {
        // Deleting a section leaves its tasks in the project, and undo puts it back.
        const SectionId section = item.value(QStringLiteral("section")).toString();
        std::optional<RemovedSection> removed;
        mutate([&](Store &store) { removed = store.removeSection(section, now()); });
        closePrompt();
        if (removed) {
            m_undo = Undo{QStringLiteral("section"), {}, {}, removed, std::nullopt};
            toast(QStringLiteral("Deleted the section; its %1 stayed in the project").arg(countOf(removed->tasks.size())), true);
        }
        recompute();
        return;
    }
    if (action == u"new-project") { beginInput(action, QStringLiteral("new project"), QStringLiteral("Loft conversion"), {}, {}); return; }
    if (action == u"new-subproject" && project) { beginInput(QStringLiteral("new-project"), QStringLiteral("new project under #%1").arg(m_store.project(*project)->name), QStringLiteral("Admin"), {}, {{QStringLiteral("parent"), *project}}); return; }
    if (action == u"rename-project" && project) { beginInput(action, QStringLiteral("rename the project"), {}, m_store.project(*project)->name, {{QStringLiteral("project"), *project}}); return; }
    if (action == u"delete-project" && project) {
        // Deleting a project asks first, because it takes subprojects and their tasks.
        const int tasks = static_cast<int>(m_store.progress(*project).second);
        m_prompt = QStringLiteral("confirm");
        m_inputAction = QStringLiteral("confirm-delete-project");
        m_inputPayload = {{QStringLiteral("project"), *project}};
        m_promptTitle = QStringLiteral("Delete #%1 and its %2? Subprojects go with it.").arg(m_store.project(*project)->name, countOf(tasks));
        m_promptQuery.clear();
        recompute();
        return;
    }
    if (action == u"new-filter") { beginInput(QStringLiteral("filter-query"), QStringLiteral("save a filter — a query, then a name"), QStringLiteral("p1 & due before: next week"), {}, {}); return; }
    if (action == u"edit-filter" && filter) {
        const SavedFilter *saved = m_store.filter(*filter);
        beginInput(QStringLiteral("filter-query"), QStringLiteral("edit the filter"), {}, saved->query, {{QStringLiteral("filter"), *filter}, {QStringLiteral("name"), saved->name}});
        return;
    }
    if (action == u"delete-filter" && filter) {
        mutate([&](Store &store) { store.removeFilter(*filter, now()); });
        closePrompt();
        go(QStringLiteral("today"));
        return;
    }
    if (action == u"show-completed" && project) {
        mutate([&](Store &store) { if (Project *p = store.projectMut(*project)) { p->showCompleted = !p->showCompleted; p->touch(now()); } });
        closePrompt();
        return;
    }
    closePrompt();
}

void App::toggleStyle() {
    const auto view = currentView();
    if (!view || !view->projectId()) return;
    mutate([&](Store &store) {
        if (Project *project = store.projectMut(*view->projectId())) {
            project->viewStyle = project->viewStyle == ViewStyle::Board ? ViewStyle::List : ViewStyle::Board;
            project->touch(now());
        }
    });
    recompute();
}

void App::newSection() {
    const auto view = currentView();
    if (!view || !view->projectId()) return;
    beginInput(QStringLiteral("new-section"), QStringLiteral("new section in #%1").arg(m_store.project(*view->projectId())->name), QStringLiteral("In progress"), {}, {{QStringLiteral("project"), *view->projectId()}});
}

void App::newProject() {
    beginInput(QStringLiteral("new-project"), QStringLiteral("new project"), QStringLiteral("Loft conversion"), {}, {});
}

// --- the date picker ----------------------------------------------------------------

void App::openDatePicker() {
    const QList<TaskId> ids = targets();
    if (ids.isEmpty()) return;
    m_picker = Picker();
    m_picker.open = true;
    m_picker.tasks = ids;
    m_picker.mode = ids.size() > 1 ? QStringLiteral("bulk") : QStringLiteral("due");
    const Task *task = m_store.task(ids.first());
    if (ids.size() == 1 && task && task->due) {
        m_picker.date = task->due->date;
        m_picker.hadDate = true;
        if (task->due->time) m_picker.time = task->due->time->toString(QStringLiteral("HH:mm"));
        m_picker.rule = task->due->recurrence;
        if (task->due->recurrence) m_picker.repeat = task->due->recurrence->describe();
    }
    const QDate anchor = m_picker.date.value_or(todayDate());
    m_picker.month = QDate(anchor.year(), anchor.month(), 1);
    recompute();
}

void App::openDeadlinePicker() {
    const QString id = !m_openTask.isEmpty() ? m_openTask : cursorId().value_or(QString());
    const Task *task = m_store.task(id);
    if (!task) return;
    m_picker = Picker();
    m_picker.open = true;
    m_picker.tasks = {id};
    m_picker.mode = QStringLiteral("deadline");
    m_picker.date = task->deadline;
    m_picker.hadDate = task->deadline.has_value();
    const QDate anchor = m_picker.date.value_or(todayDate());
    m_picker.month = QDate(anchor.year(), anchor.month(), 1);
    recompute();
}

void App::closePicker() {
    m_picker = Picker();
    recompute();
}

// What the picker chose, written to the task(s) at once: every change in the
// popover is live, so Escape just closes it.
void App::applyPicker() {
    if (!m_picker.open) return;
    const QDate today = todayDate();
    if (m_picker.mode == u"deadline") {
        mutate([&](Store &store) {
            if (Task *task = store.taskMut(m_picker.tasks.first()); task && task->deadline != m_picker.date) {
                task->deadline = m_picker.date;
                task->touch(now());
            }
        });
        return;
    }
    if (m_picker.mode == u"bulk") {
        // A bulk "do these on Friday" is about the day: keep each task's own
        // time and rule.
        mutate([&](Store &store) {
            for (const TaskId &id : m_picker.tasks) {
                Task *task = store.taskMut(id);
                if (!task) continue;
                std::optional<Due> updated;
                if (m_picker.date) {
                    updated = task->due ? *task->due : Due::on(*m_picker.date);
                    updated->date = *m_picker.date;
                }
                if (task->due != updated) {
                    task->due = updated;
                    task->touch(now());
                }
            }
        });
        return;
    }
    // A repeat with no date lands on the rule's first occurrence.
    std::optional<QDate> date = m_picker.date;
    if (!date && m_picker.rule) date = m_picker.rule->firstOccurrence(today);
    std::optional<Due> due;
    if (date) {
        due = Due::on(*date);
        if (const auto time = parseTime(m_picker.time)) due->time = time;
        due->recurrence = m_picker.rule;
    }
    mutate([&](Store &store) {
        if (Task *task = store.taskMut(m_picker.tasks.first()); task && task->due != due) {
            task->due = due;
            task->touch(now());
        }
    });
}

void App::pickerType(const QString &text) {
    if (!m_picker.open) return;
    // The same parser as the entry: "next friday 9am" sets both.
    const QuickAdd parsed = parseQuickAdd(text, todayDate(), Vocabulary());
    if (!parsed.due || !parsed.title.trimmed().isEmpty()) {
        m_picker.repeatError = text.trimmed().isEmpty() ? QString() : QStringLiteral("Not a date");
        recompute();
        return;
    }
    m_picker.repeatError.clear();
    m_picker.date = parsed.due->date;
    m_picker.hadDate = true;
    m_picker.month = QDate(parsed.due->date.year(), parsed.due->date.month(), 1);
    if (parsed.due->time) m_picker.time = parsed.due->time->toString(QStringLiteral("HH:mm"));
    if (parsed.due->recurrence && m_picker.mode == u"due") {
        m_picker.rule = parsed.due->recurrence;
        m_picker.repeat = parsed.due->recurrence->describe();
    }
    applyPicker();
    recompute();
}

void App::pickerQuick(const QString &which) {
    if (!m_picker.open) return;
    const QDate today = todayDate();
    if (which == u"today") m_picker.date = today;
    else if (which == u"tomorrow") m_picker.date = today.addDays(1);
    else if (which == u"nextweek") m_picker.date = today.addDays(7);
    else if (which == u"none") {
        m_picker.date.reset();
        m_picker.time.clear();
        m_picker.rule.reset();
        m_picker.repeat.clear();
    }
    m_picker.hadDate = m_picker.date.has_value();
    if (m_picker.date) m_picker.month = QDate(m_picker.date->year(), m_picker.date->month(), 1);
    applyPicker();
    recompute();
}

void App::pickerMonth(int delta) {
    if (!m_picker.open) return;
    m_picker.month = m_picker.month.addMonths(delta);
    recompute();
}

void App::pickerDay(const QString &iso) {
    const QDate date = QDate::fromString(iso, Qt::ISODate);
    if (!m_picker.open || !date.isValid()) return;
    m_picker.date = date;
    m_picker.hadDate = true;
    m_picker.month = QDate(date.year(), date.month(), 1);
    applyPicker();
    recompute();
}

void App::pickerTime(const QString &text) {
    if (!m_picker.open || m_picker.mode != u"due") return;
    const QString trimmed = text.trimmed();
    if (!trimmed.isEmpty() && !parseTime(trimmed)) {
        m_picker.repeatError = QStringLiteral("Not a time");
        recompute();
        return;
    }
    m_picker.repeatError.clear();
    m_picker.time = trimmed.isEmpty() ? QString() : parseTime(trimmed)->toString(QStringLiteral("HH:mm"));
    if (!m_picker.date && !trimmed.isEmpty()) {
        m_picker.date = todayDate();
        m_picker.hadDate = true;
    }
    applyPicker();
    recompute();
}

// The repeat control is a text box, and must stay one: `every!` has no
// obvious widget. An empty box stops the repeat and keeps the date; a phrase
// that will not parse changes nothing rather than guessing.
void App::pickerRepeat(const QString &text) {
    if (!m_picker.open || m_picker.mode != u"due") return;
    const QString trimmed = text.trimmed();
    m_picker.repeat = text;
    if (trimmed.isEmpty()) {
        m_picker.rule.reset();
        m_picker.repeatError.clear();
        applyPicker();
        recompute();
        return;
    }
    const auto rule = parseRecurrence(trimmed, todayDate());
    if (!rule) {
        m_picker.repeatError = QStringLiteral("Not a repeat");
        recompute();
        return;
    }
    m_picker.repeatError.clear();
    m_picker.rule = rule;
    if (!m_picker.date) {
        m_picker.date = rule->firstOccurrence(todayDate());
        m_picker.hadDate = true;
        m_picker.month = QDate(m_picker.date->year(), m_picker.date->month(), 1);
    }
    applyPicker();
    recompute();
}

void App::pickerClearRepeat() { pickerRepeat(QString()); }

// --- reminders ---------------------------------------------------------------------

void App::fireReminders() {
    for (const Firing &firing : m_schedule.takeDue(m_store, QDateTime::currentDateTimeUtc(), QTimeZone::systemTimeZone()))
        notify(firing.title, QStringLiteral("Task due"));
}

void App::notify(const QString &title, const QString &body) {
    QDBusInterface notifications(QStringLiteral("org.freedesktop.Notifications"), QStringLiteral("/org/freedesktop/Notifications"),
                                 QStringLiteral("org.freedesktop.Notifications"), QDBusConnection::sessionBus());
    if (!notifications.isValid()) return;
    notifications.asyncCall(QStringLiteral("Notify"), QStringLiteral("Planner"), 0u, QStringLiteral("planner"), title, body, QStringList(),
                            QVariantMap(), -1);
}

void App::toast(const QString &text, bool undoable) {
    m_toast = {{QStringLiteral("text"), text}, {QStringLiteral("undo"), undoable}};
    m_toastTimer.start();
}

void App::act(const QString &name) {
    if (name == u"detail") enter();
    else if (name == u"palette") { openPrompt(QStringLiteral("palette")); setPromptQuery(QStringLiteral("sec")); }
    else if (name == u"add") { openPrompt(QStringLiteral("add")); setPromptQuery(QStringLiteral("Email Sam about the lease #Work @email p1 friday 9am !30m")); }
    else if (name == u"find") { openPrompt(QStringLiteral("find")); setPromptQuery(QStringLiteral("the")); }
    else if (name == u"select") { selectAll(); m_selection = m_rowIds.mid(0, 2); m_cursor = std::min(2, static_cast<int>(m_rowIds.size()) - 1); m_cursorWanted = m_rowIds.value(m_cursor); recompute(); }
    else if (name == u"board") { if (!m_board) toggleStyle(); moveColumn(1); }
    else if (name == u"picker") { cursorTo(2); openDatePicker(); }
    else if (name == u"norail") toggleRail();
    else if (name == u"sync") showSyncStatus();
    else if (name == u"space") space();
    else if (name.startsWith(u"note:")) { enter(); addNote(m_openTask, name.mid(5)); }
    else if (name.startsWith(u"cursor:")) cursorTo(name.mid(7).toInt());
}

// --- derived state ------------------------------------------------------------------

void App::recompute() {
    buildRail();
    buildContent();
    buildDetail();
    buildPrompt();
    buildPicker();
    buildStatus();
    emit changed();
}

void App::buildRail() {
    m_rail.clear();
    const QDate today = todayDate();
    int number = 0;
    auto push = [&](const View &view, const QString &kind, int keycap) {
        QVariantMap item;
        item.insert(QStringLiteral("id"), view.id);
        item.insert(QStringLiteral("title"), view.title);
        item.insert(QStringLiteral("icon"), view.icon);
        item.insert(QStringLiteral("number"), keycap);
        item.insert(QStringLiteral("color"), view.color ? colorRole(*view.color) : QString());
        item.insert(QStringLiteral("depth"), view.depth);
        item.insert(QStringLiteral("kind"), kind);
        item.insert(QStringLiteral("selected"), view.id == m_viewId);
        // Completed is a count of things already done; showing it beside the
        // open counts would read as work outstanding.
        int count = -1;
        if (view.id != u"completed") {
            if (const auto query = Query::parse(view.query)) count = static_cast<int>(query->run(m_store, today).size());
        }
        item.insert(QStringLiteral("count"), count);
        m_rail.append(item);
    };
    for (const View &view : builtinViews()) push(view, QStringLiteral("builtin"), ++number);
    const QList<View> filters = filterViews(m_store);
    if (!filters.isEmpty()) m_rail.append(QVariantMap{{QStringLiteral("kind"), QStringLiteral("heading")}, {QStringLiteral("title"), QStringLiteral("filters")}});
    for (const View &view : filters) push(view, QStringLiteral("filter"), 0);
    m_rail.append(QVariantMap{{QStringLiteral("kind"), QStringLiteral("heading")}, {QStringLiteral("title"), QStringLiteral("projects")}});
    for (const View &view : projectViews(m_store)) push(view, QStringLiteral("project"), 0);
    // The one thing the rail lets you make from here; everything else is a view.
    m_rail.append(QVariantMap{{QStringLiteral("kind"), QStringLiteral("new-project")}, {QStringLiteral("title"), QStringLiteral("new project")}});
}

QVariantMap App::row(const Task &task, int lane, int index, int flat) const {
    const QDate today = todayDate();
    QVariantMap row;
    row.insert(QStringLiteral("id"), task.id);
    row.insert(QStringLiteral("content"), task.content);
    row.insert(QStringLiteral("checked"), task.checked);
    row.insert(QStringLiteral("priorityRole"), priorityRole(task.priority));
    if (task.due) {
        const auto [label, cls] = formatDue(task.due->date, task.due->time, today);
        row.insert(QStringLiteral("due"), label);
        row.insert(QStringLiteral("dueRole"), task.checked ? QString() : dueRoleFor(cls));
        row.insert(QStringLiteral("repeat"), task.due->recurrence ? task.due->recurrence->describe() : QString());
    } else {
        row.insert(QStringLiteral("due"), QString());
        row.insert(QStringLiteral("dueRole"), QString());
        row.insert(QStringLiteral("repeat"), QString());
    }
    row.insert(QStringLiteral("deadline"), task.deadline ? QStringLiteral("Due %1").arg(formatDate(*task.deadline, today)) : QString());
    row.insert(QStringLiteral("deadlineRole"), task.isPastDeadline(today) ? QStringLiteral("negative") : QString());
    QStringList labels;
    for (const LabelId &id : task.labels)
        if (const Label *label = m_store.label(id)) labels << label->name;
    row.insert(QStringLiteral("labels"), labels);
    const QList<const Task *> children = m_store.subtasks(task.id);
    int done = 0;
    for (const Task *child : children) done += child->checked ? 1 : 0;
    row.insert(QStringLiteral("subtasks"), children.isEmpty() ? QString() : QStringLiteral("%1 of %2").arg(done).arg(children.size()));
    row.insert(QStringLiteral("pinned"), task.pinned);
    row.insert(QStringLiteral("lane"), lane);
    row.insert(QStringLiteral("index"), index);
    row.insert(QStringLiteral("flat"), flat);
    row.insert(QStringLiteral("cursor"), flat == m_cursor);
    row.insert(QStringLiteral("selected"), m_selecting && m_selection.contains(task.id));
    return row;
}

void App::buildContent() {
    const QDate today = todayDate();
    m_lanes.clear();
    m_rowIds.clear();
    m_rowPlaces.clear();
    m_laneSections.clear();
    m_empty.clear();

    auto view = currentView();
    if (!view) {
        // The view went away — a deleted project or filter.
        m_viewId = QStringLiteral("today");
        view = currentView();
    }
    m_viewTitle = view->title;
    const std::optional<ProjectId> projectId = view->projectId();
    const Project *project = projectId ? m_store.project(*projectId) : nullptr;
    m_isProject = project != nullptr;
    m_board = project && project->viewStyle == ViewStyle::Board;

    QList<QList<const Task *>> laneTasks;
    QStringList laneNames;
    int total = 0;
    if (project) {
        laneNames << QString();
        m_laneSections.append(std::nullopt);
        laneTasks.append(m_store.tasksIn(project->id, std::nullopt));
        for (const Section *section : m_store.sectionsIn(project->id)) {
            laneNames << section->name;
            m_laneSections.append(section->id);
            laneTasks.append(m_store.tasksIn(project->id, section->id));
        }
        if (!project->showCompleted) {
            for (QList<const Task *> &tasks : laneTasks)
                tasks.erase(std::remove_if(tasks.begin(), tasks.end(), [](const Task *task) { return task->checked; }), tasks.end());
        }
        const auto [done, count] = m_store.progress(project->id);
        m_viewSubtitle = count == 0 ? QString() : QStringLiteral("%1 of %2 done").arg(done).arg(count);
    } else {
        laneNames << QString();
        m_laneSections.append(std::nullopt);
        const auto query = Query::parse(view->query);
        // A saved filter that will not parse matches nothing.
        laneTasks.append(query ? query->run(m_store, today) : QList<const Task *>());
        m_viewSubtitle = view->query;
    }

    // Keep the cursor on the task it was on; otherwise clamp.
    int flat = 0;
    int wanted = -1;
    for (int lane = 0; lane < laneTasks.size(); ++lane) {
        for (int index = 0; index < laneTasks.at(lane).size(); ++index) {
            const Task *task = laneTasks.at(lane).at(index);
            if (task->id == m_cursorWanted) wanted = flat;
            m_rowIds.append(task->id);
            m_rowPlaces.append({lane, index});
            ++flat;
            ++total;
        }
    }
    if (wanted >= 0) m_cursor = wanted;
    m_cursor = m_rowIds.isEmpty() ? 0 : std::clamp(m_cursor, 0, static_cast<int>(m_rowIds.size()) - 1);
    if (!m_rowIds.isEmpty()) m_cursorWanted = m_rowIds.at(m_cursor);
    if (m_board && m_cursor < m_rowPlaces.size()) m_boardColumn = m_rowPlaces.at(m_cursor).first;
    m_boardColumn = m_laneSections.isEmpty() ? 0 : std::clamp(m_boardColumn, 0, static_cast<int>(m_laneSections.size()) - 1);

    flat = 0;
    for (int lane = 0; lane < laneTasks.size(); ++lane) {
        QVariantList rows;
        for (int index = 0; index < laneTasks.at(lane).size(); ++index) rows.append(row(*laneTasks.at(lane).at(index), lane, index, flat++));
        QVariantMap entry;
        entry.insert(QStringLiteral("section"), m_laneSections.at(lane).value_or(QString()));
        entry.insert(QStringLiteral("name"), laneNames.at(lane).isEmpty() ? (m_board ? QStringLiteral("no section") : QString()) : laneNames.at(lane));
        entry.insert(QStringLiteral("count"), static_cast<int>(rows.size()));
        entry.insert(QStringLiteral("tasks"), rows);
        entry.insert(QStringLiteral("current"), m_board && lane == m_boardColumn);
        m_lanes.append(entry);
    }

    // A selection can only hold rows that are still on screen.
    if (m_selecting) {
        QStringList kept;
        for (const QString &id : m_selection)
            if (m_rowIds.contains(id)) kept << id;
        m_selection = kept;
    }
    // The pane shows a row of this list. A task completed out of a filter, or
    // deleted, hands the pane to the cursor row; in a view that still shows
    // it, it stays, struck through, so reopening it is one keystroke away.
    if (!m_openTask.isEmpty() && !m_rowIds.contains(m_openTask)) m_openTask = m_rowIds.isEmpty() ? QString() : m_rowIds.at(m_cursor);

    if (total == 0) {
        m_empty.insert(QStringLiteral("title"), view->emptyTitle);
        m_empty.insert(QStringLiteral("description"), view->emptyDescription);
        m_empty.insert(QStringLiteral("icon"), view->icon);
        QString key, text;
        if (view->id == u"pinned") { key = QStringLiteral("ctrl+shift+p"); text = QStringLiteral("pins whatever the cursor is on"); }
        else if (view->id == u"completed") { key = QStringLiteral("space"); text = QStringLiteral("completes the cursor row in any view"); }
        else if (view->id == u"upcoming") { key = QStringLiteral("ctrl+d"); text = QStringLiteral("gives a task a date"); }
        else { key = QStringLiteral("ctrl+n"); text = QStringLiteral("adds a task here"); }
        m_empty.insert(QStringLiteral("key"), key);
        m_empty.insert(QStringLiteral("hint"), text);
    }
}

void App::buildDetail() {
    m_detail.clear();
    const Task *task = m_openTask.isEmpty() ? nullptr : m_store.task(m_openTask);
    if (!task) return;
    const QDate today = todayDate();
    m_detail.insert(QStringLiteral("id"), task->id);
    m_detail.insert(QStringLiteral("title"), task->content);
    m_detail.insert(QStringLiteral("description"), task->description);
    m_detail.insert(QStringLiteral("checked"), task->checked);
    m_detail.insert(QStringLiteral("schedule"), describeDue(task->due, today));
    m_detail.insert(QStringLiteral("hasSchedule"), task->due.has_value());
    m_detail.insert(QStringLiteral("scheduleRole"), task->due && !task->checked ? dueRoleFor(formatDue(task->due->date, std::nullopt, today).second) : QString());
    m_detail.insert(QStringLiteral("deadline"), describeDeadline(task->deadline, today));
    m_detail.insert(QStringLiteral("hasDeadline"), task->deadline.has_value());
    m_detail.insert(QStringLiteral("deadlineRole"), task->isPastDeadline(today) ? QStringLiteral("negative") : QString());
    m_detail.insert(QStringLiteral("priorityRole"), priorityRole(task->priority));
    m_detail.insert(QStringLiteral("priority"), QStringLiteral("%1 %2").arg(priorityToken(task->priority), priorityLabel(task->priority)));
    const Project *project = m_store.project(task->projectId);
    m_detail.insert(QStringLiteral("project"), QLatin1Char('#') + (project ? project->name : QStringLiteral("?")));
    if (task->sectionId) {
        if (const Section *section = m_store.section(*task->sectionId)) m_detail.insert(QStringLiteral("section"), QLatin1Char('/') + section->name);
    }
    QStringList labels;
    for (const LabelId &id : task->labels)
        if (const Label *label = m_store.label(id)) labels << label->name;
    m_detail.insert(QStringLiteral("labels"), labels);
    m_detail.insert(QStringLiteral("pinned"), task->pinned);
    QVariantList subtasks;
    int done = 0;
    for (const Task *child : m_store.subtasks(task->id)) {
        subtasks.append(QVariantMap{{QStringLiteral("id"), child->id}, {QStringLiteral("content"), child->content}, {QStringLiteral("checked"), child->checked}});
        done += child->checked ? 1 : 0;
    }
    m_detail.insert(QStringLiteral("subtasks"), subtasks);
    m_detail.insert(QStringLiteral("subtaskLine"), subtasks.isEmpty() ? QString() : QStringLiteral("%1 of %2 done").arg(done).arg(subtasks.size()));
    if (task->parentId) {
        if (const Task *parent = m_store.task(*task->parentId)) {
            m_detail.insert(QStringLiteral("parent"), parent->id);
            m_detail.insert(QStringLiteral("parentTitle"), parent->content);
        }
    }
    // Activity, oldest first, each stamped the way a row reads a date.
    QVariantList notes;
    for (const Note &note : task->notes) {
        const QDateTime local = note.at.toLocalTime();
        const QString when = formatDate(local.date(), today) + QLatin1Char(' ') + local.toString(QStringLiteral("HH:mm"));
        notes.append(QVariantMap{{QStringLiteral("at"), instantSerial(note.at)}, {QStringLiteral("when"), when}, {QStringLiteral("text"), note.text}});
    }
    m_detail.insert(QStringLiteral("notes"), notes);
    QStringList reminders;
    for (const Reminder &reminder : task->reminders)
        if (reminder.trigger.kind == Trigger::BeforeDue) reminders << duration(reminder.trigger.minutes) + QStringLiteral(" before");
    m_detail.insert(QStringLiteral("reminders"), reminders);
}

void App::buildPrompt() {
    m_promptResults.clear();
    m_promptCount.clear();
    m_addPreview.clear();
    if (m_prompt.isEmpty()) return;
    const QDate today = todayDate();
    const QString query = m_promptQuery.trimmed();

    if (m_prompt == u"add") {
        const QuickAdd parsed = parseQuickAdd(m_promptQuery, today, m_store.vocabulary());
        QVariantList spans;
        for (const Span &span : parsed.spans) spans.append(QVariantMap{{QStringLiteral("start"), span.start}, {QStringLiteral("end"), span.end}});
        QString destination = QStringLiteral("Inbox");
        if (const auto view = currentView(); view && view->projectId()) {
            if (const Project *project = m_store.project(*view->projectId())) destination = project->name;
        }
        if (parsed.project) {
            if (const Project *named = m_store.projectByName(*parsed.project)) destination = named->name;
        }
        QVariantList chips;
        for (const Chip &chip : describeQuickAdd(parsed, today, destination))
            chips.append(QVariantMap{{QStringLiteral("icon"), chip.icon}, {QStringLiteral("text"), chip.text}, {QStringLiteral("role"), chip.role}});
        m_addPreview.insert(QStringLiteral("spans"), spans);
        m_addPreview.insert(QStringLiteral("chips"), chips);
        m_addPreview.insert(QStringLiteral("destination"), QStringLiteral("lands in #%1").arg(destination));
        m_addPreview.insert(QStringLiteral("canSubmit"), !parsed.title.trimmed().isEmpty());
        m_addPreview.insert(QStringLiteral("hint"), QString::fromUtf8(kQuickAddHint));
        return;
    }

    if (m_prompt == u"find") {
        const QList<Hit> hits = search(m_store, query, 12);
        for (const Hit &hit : hits) {
            QVariantMap item;
            item.insert(QStringLiteral("kind"), hit.kind == Hit::TaskHit ? QStringLiteral("task") : hit.kind == Hit::ProjectHit ? QStringLiteral("project") : QStringLiteral("label"));
            item.insert(QStringLiteral("id"), hit.id);
            item.insert(QStringLiteral("title"), hit.title);
            item.insert(QStringLiteral("context"), hit.kind == Hit::TaskHit ? QLatin1Char('#') + hit.context : hit.context);
            item.insert(QStringLiteral("priorityRole"), priorityRole(hit.priority));
            item.insert(QStringLiteral("completed"), hit.completed);
            item.insert(QStringLiteral("icon"), hit.kind == Hit::ProjectHit ? QStringLiteral("folder") : hit.kind == Hit::LabelHit ? QStringLiteral("user-bookmarks") : QString());
            m_promptResults.append(item);
        }
        m_promptCount = hits.isEmpty() ? (query.isEmpty() ? QString() : QStringLiteral("no hits")) : hits.size() == 1 ? QStringLiteral("1 hit") : QStringLiteral("%1 hits").arg(hits.size());
        m_promptIndex = hits.isEmpty() ? 0 : std::clamp(m_promptIndex, 0, static_cast<int>(hits.size()) - 1);
        return;
    }

    if (m_prompt == u"input" || m_prompt == u"confirm" || m_prompt == u"status") {
        if (m_inputAction == u"filter-query") {
            QueryError error;
            m_promptError = query.isEmpty() || Query::parse(query, &error) ? QString() : error.message;
        }
        return;
    }

    // The palette: actions, views and tasks in one list.
    struct Entry { QString group; QVariantMap item; int score; };
    QList<Entry> entries;
    const auto view = currentView();
    const std::optional<ProjectId> projectId = view ? view->projectId() : std::nullopt;
    const std::optional<FilterId> filterId = view ? view->filterId() : std::nullopt;
    const Project *project = projectId ? m_store.project(*projectId) : nullptr;
    const auto cursor = cursorId();
    auto action = [&](const QString &id, const QString &title, const QString &icon, const QString &key, const QString &chip, const QVariantMap &extra = {}) {
        const auto score = query.isEmpty() ? std::optional<int>(0) : searchScore(title, query);
        if (!score) return;
        QVariantMap item = extra;
        item.insert(QStringLiteral("kind"), QStringLiteral("action"));
        item.insert(QStringLiteral("id"), id);
        item.insert(QStringLiteral("title"), title);
        item.insert(QStringLiteral("icon"), icon);
        item.insert(QStringLiteral("key"), key);
        item.insert(QStringLiteral("chip"), chip);
        entries.append({QStringLiteral("actions"), item, *score});
    };
    action(QStringLiteral("new-task"), QStringLiteral("New Task…"), QStringLiteral("list-add"), QStringLiteral("ctrl+n"), {});
    action(QStringLiteral("find"), QStringLiteral("Quick Find…"), QStringLiteral("system-search"), QStringLiteral("ctrl+f"), {});
    if (project) {
        const QString in = QStringLiteral("in #%1").arg(project->name);
        action(QStringLiteral("new-section"), QStringLiteral("Add Section…"), QStringLiteral("list-add"), QStringLiteral("ctrl+shift+n"), in);
        if (cursor && m_cursor < m_rowPlaces.size()) {
            if (const auto section = m_laneSections.value(m_rowPlaces.at(m_cursor).first)) {
                const QString name = m_store.section(*section)->name;
                action(QStringLiteral("rename-section"), QStringLiteral("Rename Section…"), QStringLiteral("document-edit"), {}, QLatin1Char('/') + name, {{QStringLiteral("section"), *section}});
                action(QStringLiteral("delete-section"), QStringLiteral("Delete Section"), QStringLiteral("user-trash"), {}, QLatin1Char('/') + name, {{QStringLiteral("section"), *section}});
            }
        }
        action(QStringLiteral("toggle-style"), m_board ? QStringLiteral("Show as List") : QStringLiteral("Show as Board"), m_board ? QStringLiteral("view-list") : QStringLiteral("view-grid"), QStringLiteral("ctrl+shift+b"), {});
        action(QStringLiteral("show-completed"), project->showCompleted ? QStringLiteral("Hide Completed Tasks") : QStringLiteral("Show Completed Tasks"), QStringLiteral("object-select"), {}, in);
        action(QStringLiteral("new-subproject"), QStringLiteral("New Subproject…"), QStringLiteral("folder-new"), {}, in);
        action(QStringLiteral("rename-project"), QStringLiteral("Rename Project…"), QStringLiteral("document-edit"), {}, QLatin1Char('#') + project->name);
        action(QStringLiteral("delete-project"), QStringLiteral("Delete Project…"), QStringLiteral("user-trash"), {}, QLatin1Char('#') + project->name);
    }
    if (filterId) {
        action(QStringLiteral("edit-filter"), QStringLiteral("Edit Filter…"), QStringLiteral("edit-find"), {}, view->title);
        action(QStringLiteral("delete-filter"), QStringLiteral("Delete Filter"), QStringLiteral("user-trash"), {}, view->title);
    }
    if (!m_rowIds.isEmpty()) {
        action(QStringLiteral("select"), QStringLiteral("Select Tasks"), QStringLiteral("selection-mode"), QStringLiteral("ctrl+a"), {});
        action(QStringLiteral("complete"), m_selecting ? QStringLiteral("Complete Selected") : QStringLiteral("Complete Task"), QStringLiteral("object-select"), QStringLiteral("space"), {});
        action(QStringLiteral("date"), QStringLiteral("Schedule…"), QStringLiteral("x-office-calendar"), QStringLiteral("ctrl+d"), {});
        action(QStringLiteral("deadline"), QStringLiteral("Set Deadline…"), QStringLiteral("alarm"), {}, {});
        action(QStringLiteral("pin"), QStringLiteral("Pin Task"), QStringLiteral("view-pin"), QStringLiteral("ctrl+shift+p"), {});
        action(QStringLiteral("delete-task"), m_selecting ? QStringLiteral("Delete Selected") : QStringLiteral("Delete Task"), QStringLiteral("user-trash"), QStringLiteral("del"), {});
    }
    action(QStringLiteral("new-project"), QStringLiteral("New Project…"), QStringLiteral("folder-new"), {}, {});
    action(QStringLiteral("new-filter"), QStringLiteral("New Filter…"), QStringLiteral("edit-find"), {}, {});
    action(QStringLiteral("toggle-rail"), m_railVisible ? QStringLiteral("Hide Rail") : QStringLiteral("Show Rail"), QStringLiteral("sidebar-show"), QStringLiteral("ctrl+b"), {});
    action(QStringLiteral("sync-status"), QStringLiteral("Sync…"), QStringLiteral("view-continuous"), {}, m_syncTarget ? (m_syncLastFailure.isEmpty() ? (m_syncLastPass ? ago(*m_syncLastPass) : QString()) : QStringLiteral("failing")) : QStringLiteral("off"));
    if (m_syncTarget) action(QStringLiteral("sync-now"), QStringLiteral("Sync Now"), QStringLiteral("view-continuous"), {}, m_syncTarget->first);
    if (m_undo) action(QStringLiteral("undo"), QStringLiteral("Undo"), QStringLiteral("edit-clear"), QStringLiteral("ctrl+z"), {});

    auto viewEntry = [&](const View &v) {
        const auto score = query.isEmpty() ? std::optional<int>(0) : searchScore(v.title, query);
        if (!score) return;
        QVariantMap item{{QStringLiteral("kind"), QStringLiteral("view")}, {QStringLiteral("id"), v.id}, {QStringLiteral("title"), v.title},
                         {QStringLiteral("icon"), v.icon}, {QStringLiteral("chip"), v.query}, {QStringLiteral("color"), v.color ? colorRole(*v.color) : QString()}};
        entries.append({QStringLiteral("views"), item, *score});
    };
    for (const View &v : builtinViews()) viewEntry(v);
    for (const View &v : filterViews(m_store)) viewEntry(v);
    for (const View &v : projectViews(m_store)) viewEntry(v);

    if (!query.isEmpty()) {
        for (const Hit &hit : search(m_store, query, 8)) {
            if (hit.kind != Hit::TaskHit) continue;
            const Task *task = m_store.task(hit.id);
            QVariantMap item{{QStringLiteral("kind"), QStringLiteral("task")}, {QStringLiteral("id"), hit.id}, {QStringLiteral("title"), hit.title},
                             {QStringLiteral("priorityRole"), priorityRole(hit.priority)}, {QStringLiteral("completed"), hit.completed}};
            QString context = QLatin1Char('#') + hit.context;
            QString role;
            if (task && task->due && !task->checked) {
                const auto [label, cls] = formatDue(task->due->date, task->due->time, today);
                context = label;
                role = dueRoleFor(cls);
            }
            item.insert(QStringLiteral("context"), context);
            item.insert(QStringLiteral("contextRole"), role);
            entries.append({QStringLiteral("tasks"), item, *searchScore(hit.title, query)});
        }
    }

    std::stable_sort(entries.begin(), entries.end(), [](const Entry &a, const Entry &b) { return a.score > b.score; });
    int total = 0;
    for (const QString &group : {QStringLiteral("actions"), QStringLiteral("views"), QStringLiteral("tasks")}) {
        bool any = false;
        for (const Entry &entry : entries) {
            if (entry.group != group) continue;
            if (!any) {
                m_promptResults.append(QVariantMap{{QStringLiteral("kind"), QStringLiteral("heading")}, {QStringLiteral("title"), group}});
                any = true;
            }
            m_promptResults.append(entry.item);
            ++total;
        }
    }
    m_promptIndex = total == 0 ? 0 : std::clamp(m_promptIndex, 0, total - 1);
    m_promptCount = total == 0 ? QStringLiteral("nothing matches") : QStringLiteral("%1 of %2").arg(m_promptIndex + 1).arg(total);
}

void App::buildPicker() {
    m_pickerMap.clear();
    m_pickerMap.insert(QStringLiteral("open"), m_picker.open);
    if (!m_picker.open) return;
    const QDate today = todayDate();
    m_pickerMap.insert(QStringLiteral("mode"), m_picker.mode);
    m_pickerMap.insert(QStringLiteral("count"), static_cast<int>(m_picker.tasks.size()));
    m_pickerMap.insert(QStringLiteral("monthLabel"), QStringLiteral("%1 %2").arg(QLocale::c().monthName(m_picker.month.month(), QLocale::LongFormat)).arg(m_picker.month.year()));
    m_pickerMap.insert(QStringLiteral("selected"), m_picker.date ? dateSerial(*m_picker.date) : QString());
    m_pickerMap.insert(QStringLiteral("time"), m_picker.time);
    m_pickerMap.insert(QStringLiteral("repeat"), m_picker.repeat);
    m_pickerMap.insert(QStringLiteral("error"), m_picker.repeatError);
    m_pickerMap.insert(QStringLiteral("hasRepeat"), m_picker.mode == u"due");
    m_pickerMap.insert(QStringLiteral("hasTime"), m_picker.mode == u"due");
    // A Monday-first grid of six weeks, with the days outside the month dimmed.
    QVariantList days;
    const QDate first = m_picker.month;
    QDate day = first.addDays(-(first.dayOfWeek() - 1));
    for (int i = 0; i < 42; ++i) {
        days.append(QVariantMap{{QStringLiteral("iso"), dateSerial(day)}, {QStringLiteral("day"), day.day()}, {QStringLiteral("inMonth"), day.month() == first.month()},
                                {QStringLiteral("selected"), m_picker.date && *m_picker.date == day}, {QStringLiteral("today"), day == today}});
        day = day.addDays(1);
    }
    m_pickerMap.insert(QStringLiteral("days"), days);
    m_pickerMap.insert(QStringLiteral("summary"), m_picker.mode == u"deadline" ? describeDeadline(m_picker.date, today)
                                                 : m_picker.date ? describeDue(Due::on(*m_picker.date), today) : QStringLiteral("No date"));
}

void App::buildStatus() {
    m_headerHints.clear();
    m_status.clear();
    const auto view = currentView();
    const int total = static_cast<int>(m_rowIds.size());

    if (m_selecting) {
        m_headerHints << hint(QStringLiteral("ctrl+a"), QStringLiteral("all"), QStringLiteral("caution")) << hint(QStringLiteral("esc"), QStringLiteral("clear"));
    } else {
        m_headerHints << hint(QStringLiteral("ctrl+n"), QStringLiteral("add")) << hint(QStringLiteral("ctrl+f"), QStringLiteral("find")) << hint(QStringLiteral("ctrl+k"), QStringLiteral("palette"));
    }

    QString left, middle, right;
    QVariantList keys;
    if (m_selecting) {
        left = QStringLiteral("%1 selected").arg(countOf(static_cast<int>(m_selection.size())));
        keys << hint(QStringLiteral("space"), QStringLiteral("complete"), QStringLiteral("positive")) << hint(QStringLiteral("ctrl+d"), QStringLiteral("schedule"))
             << hint(QStringLiteral("ctrl+1-4"), QStringLiteral("priority")) << hint(QStringLiteral("del"), QStringLiteral("delete"), QStringLiteral("negative"))
             << hint(QStringLiteral("ctrl+z"), QStringLiteral("undo"));
    } else if (!m_openTask.isEmpty() && m_detail.contains(QStringLiteral("title"))) {
        left = m_detail.value(QStringLiteral("title")).toString();
        right = QStringLiteral("ctrl+d date · ctrl+enter subtask · ctrl+shift+enter note · esc back");
    } else if (m_isProject && m_board) {
        left = QStringLiteral("#%1 · board").arg(view->title);
        right = QStringLiteral("←→ column · ctrl+←→ move task · ctrl+shift+b list");
    } else if (m_isProject) {
        left = QLatin1Char('#') + view->title;
        int sections = 0;
        for (const QVariant &lane : m_lanes)
            if (lane.toMap().value(QStringLiteral("count")).toInt() > 0 || !lane.toMap().value(QStringLiteral("section")).toString().isEmpty()) ++sections;
        middle = sections == 1 ? QStringLiteral("1 section") : QStringLiteral("%1 sections").arg(sections);
        right = QStringLiteral("ctrl+shift+b board · ctrl+shift+n new section · ctrl+↑↓ move task");
    } else {
        left = view->id.startsWith(u"filter:") ? view->title.toLower() : view->id;
        int overdue = 0;
        for (const TaskId &id : m_rowIds)
            if (const Task *task = m_store.task(id); task && task->isOverdue(todayDate())) ++overdue;
        middle = countOf(total);
        if (overdue > 0) middle += QStringLiteral(" · %1 overdue").arg(overdue);
        right = QStringLiteral("↑↓ move · ctrl+b rail · ctrl+k palette · esc close");
    }
    if (!m_toast.isEmpty()) {
        // A toast is what just happened; for six seconds it outranks the hints.
        middle = m_toast.value(QStringLiteral("text")).toString();
        if (m_toast.value(QStringLiteral("undo")).toBool()) middle += QStringLiteral(" · ctrl+z undo");
        right.clear();
    }
    // An active save failure outranks a sync problem: that is data not being
    // written right now.
    if (!m_syncError.isEmpty() && m_toast.isEmpty()) middle = QStringLiteral("sync: %1").arg(m_syncError);
    if (!m_saveError.isEmpty()) middle = m_saveError;
    m_status.insert(QStringLiteral("left"), left);
    m_status.insert(QStringLiteral("middle"), middle);
    m_status.insert(QStringLiteral("middleRole"), !m_saveError.isEmpty() ? QStringLiteral("negative") : (!m_syncError.isEmpty() && m_toast.isEmpty()) ? QStringLiteral("warning") : !m_toast.isEmpty() ? QStringLiteral("text") : QString());
    m_status.insert(QStringLiteral("right"), right);
    m_status.insert(QStringLiteral("keys"), keys);
    m_status.insert(QStringLiteral("selecting"), m_selecting);
}
