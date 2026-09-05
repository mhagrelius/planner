// Deciding what to do when a task list exists in more than one place.
//
// A pure planner over three snapshots: what this machine holds, what the
// server holds, and what the two agreed on last time. Local against base says
// whether *this* machine changed something; remote against base says whether
// another one did. Per record, last-writer-wins by `updated_at`, and a
// deletion never beats an edit. `gather` talks to the network and writes
// nothing; `apply` writes, on the thread that owns the store.
#pragma once

#include "model.h"

#include <QJsonObject>
#include <QMap>

#include <functional>

namespace planner {

class Store;

namespace sync {

struct Key {
    RecordKind kind = RecordKind::Task;
    QString id;
    bool operator==(const Key &other) const { return kind == other.kind && id == other.id; }
    bool operator<(const Key &other) const { return kind != other.kind ? kind < other.kind : id < other.id; }
};

// The timestamp is the version: two sides carrying the same one hold the same record.
struct Version {
    bool deleted = false;
    QDateTime at;
    static Version live(const QDateTime &at) { return {false, at.toUTC()}; }
    static Version gone(const QDateTime &at) { return {true, at.toUTC()}; }
    bool operator==(const Version &other) const { return deleted == other.deleted && at == other.at; }
    bool operator!=(const Version &other) const { return !(*this == other); }
};

using Snapshot = QMap<Key, Version>;

// The work one pass should do. Empty when nothing changed, and that is worth
// asserting on: a plan that is never empty re-uploads the list on a timer.
struct Plan {
    QList<Key> push, pull, deleteLocal, deleteRemote;
    bool isEmpty() const { return push.isEmpty() && pull.isEmpty() && deleteLocal.isEmpty() && deleteRemote.isEmpty(); }
    int size() const { return static_cast<int>(push.size() + pull.size() + deleteLocal.size() + deleteRemote.size()); }
};

Plan plan(const Snapshot &base, const Snapshot &local, const Snapshot &remote);

// One record, with its contents, on its way between machines.
struct Record {
    RecordKind kind = RecordKind::Task;
    QString id;
    QDateTime updatedAt;
    std::optional<QJsonObject> body;   // absent for a deletion
    QJsonObject toJson() const;
    static std::optional<Record> fromJson(const QJsonObject &json);
};

// Why a pass could not finish: one sentence for a log line.
struct Error {
    QString message;
};

// The other side, whatever is carrying it. The app answers it over HTTP; a
// test answers it in memory.
class Remote {
public:
    virtual ~Remote() = default;
    virtual std::optional<Snapshot> snapshot(Error *error) = 0;
    virtual std::optional<QList<Record>> fetch(const QList<Key> &keys, Error *error) = 0;
    virtual bool push(const QList<Record> &records, Error *error) = 0;
    virtual bool remove(const QList<Record> &records, Error *error) = 0;
};

// What one pass brought back, waiting to be applied. `base` records what
// happened, not what was planned: a failed push is left out so the next pass
// retries instead of believing itself.
struct Incoming {
    QList<Record> records;
    QList<Record> deletions;
    Snapshot base;
};

// The network half. Pushes go first and deletions last.
std::optional<Incoming> gather(Remote &remote, const Snapshot &base, const Snapshot &local,
                               const std::function<std::optional<QJsonObject>(const Key &)> &bodies, Error *error);

struct Report {
    int written = 0;
    int removed = 0;
    int unreadable = 0;   // a newer schema, or a kind this build has no type for
    bool isEmpty() const { return written == 0 && removed == 0 && unreadable == 0; }
};

// The writing half. Deletions go last. `held` names what to leave for later —
// the task open in the detail pane — and what is held stays out of the base.
std::pair<Report, Snapshot> apply(Store &store, const Incoming &incoming, const std::function<bool(const Key &)> &held);

// The agreed snapshot lives beside the document, per machine.
QString defaultBasePath(const QString &documentPath);
Snapshot loadBase(const QString &path);   // missing or unreadable = empty
bool saveBase(const Snapshot &base, const QString &path);
Snapshot snapshotOf(const Store &store);
// How many live records a snapshot holds.
int liveCount(const Snapshot &snapshot);

} // namespace sync
} // namespace planner
