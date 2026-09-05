#include "sync.h"

#include "store.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QSaveFile>
#include <QSet>

namespace planner::sync {

namespace {

enum class Side { Local, Remote };

// Both sides changed the same record. A deletion never wins over an edit;
// otherwise the later one wins, and on a tie the server's copy takes it so
// every machine lands on the same value.
std::pair<Side, Version> resolve(const Version &here, const Version &there) {
    if (here.deleted && !there.deleted) return {Side::Remote, there};
    if (!here.deleted && there.deleted) return {Side::Local, here};
    return here.at > there.at ? std::make_pair(Side::Local, here) : std::make_pair(Side::Remote, there);
}

} // namespace

Plan plan(const Snapshot &base, const Snapshot &local, const Snapshot &remote) {
    Plan plan;
    QList<Key> keys = base.keys() + local.keys() + remote.keys();
    std::sort(keys.begin(), keys.end());
    keys.erase(std::unique(keys.begin(), keys.end()), keys.end());

    for (const Key &key : keys) {
        const auto based = base.find(key);
        const auto here = local.find(key);
        const auto there = remote.find(key);
        const bool hasBase = based != base.end(), hasHere = here != local.end(), hasThere = there != remote.end();

        // Already agreed: the overwhelmingly common case.
        if (hasHere == hasThere && (!hasHere || here.value() == there.value())) continue;

        auto differs = [&](bool has, const Snapshot::const_iterator &it) {
            if (has != hasBase) return true;
            return has && it.value() != based.value();
        };
        const bool movedHere = differs(hasHere, here);
        const bool movedThere = differs(hasThere, there);

        std::optional<std::pair<Side, Version>> decision;
        if (hasHere && hasThere) {
            if (movedHere && !movedThere) decision = std::make_pair(Side::Local, here.value());
            else if (!movedHere && movedThere) decision = std::make_pair(Side::Remote, there.value());
            // Both moved, or neither did and the base is lying: resolve.
            else decision = resolve(here.value(), there.value());
        } else if (hasHere) {
            // A record simply absent on one side is not a deletion; the side
            // that has it wins.
            decision = std::make_pair(Side::Local, here.value());
        } else if (hasThere) {
            decision = std::make_pair(Side::Remote, there.value());
        }
        if (!decision) continue;   // gone from both, still in the base

        if (decision->first == Side::Local) (decision->second.deleted ? plan.deleteRemote : plan.push).append(key);
        else (decision->second.deleted ? plan.deleteLocal : plan.pull).append(key);
    }
    return plan;
}

QJsonObject Record::toJson() const {
    QJsonObject json{{QStringLiteral("kind"), recordKindSerial(kind)}, {QStringLiteral("id"), id}, {QStringLiteral("updated_at"), instantSerial(updatedAt)}};
    if (body) json.insert(QStringLiteral("body"), *body);
    return json;
}

std::optional<Record> Record::fromJson(const QJsonObject &json) {
    const auto kind = recordKindFromSerial(json.value(QStringLiteral("kind")).toString());
    const auto at = instantFromSerial(json.value(QStringLiteral("updated_at")).toString());
    if (!kind || !at) return std::nullopt;
    Record record;
    record.kind = *kind;
    record.id = json.value(QStringLiteral("id")).toString();
    record.updatedAt = *at;
    if (json.value(QStringLiteral("body")).isObject()) record.body = json.value(QStringLiteral("body")).toObject();
    return record;
}

std::optional<Incoming> gather(Remote &remote, const Snapshot &base, const Snapshot &local,
                               const std::function<std::optional<QJsonObject>(const Key &)> &bodies, Error *error) {
    const auto there = remote.snapshot(error);
    if (!there) return std::nullopt;
    const Plan work = plan(base, local, *there);
    Incoming incoming;
    incoming.base = base;

    // --- up
    QList<Record> outgoing;
    for (const Key &key : work.push) {
        // Gone between the snapshot and here: the next pass sends the deletion.
        const auto body = bodies(key);
        if (!body || !local.contains(key)) continue;
        outgoing.append(Record{key.kind, key.id, local.value(key).at, *body});
    }
    if (!outgoing.isEmpty()) {
        if (!remote.push(outgoing, error)) return std::nullopt;
        for (const Record &record : outgoing) incoming.base.insert(Key{record.kind, record.id}, Version::live(record.updatedAt));
    }
    QList<Record> removals;
    for (const Key &key : work.deleteRemote)
        if (local.contains(key)) removals.append(Record{key.kind, key.id, local.value(key).at, std::nullopt});
    if (!removals.isEmpty()) {
        if (!remote.remove(removals, error)) return std::nullopt;
        for (const Record &record : removals) incoming.base.insert(Key{record.kind, record.id}, Version::gone(record.updatedAt));
    }

    // --- down
    if (!work.pull.isEmpty()) {
        const auto fetched = remote.fetch(work.pull, error);
        if (!fetched) return std::nullopt;
        incoming.records = *fetched;
    }
    for (const Key &key : work.deleteLocal)
        if (there->contains(key)) incoming.deletions.append(Record{key.kind, key.id, there->value(key).at, std::nullopt});
    return incoming;
}

std::pair<Report, Snapshot> apply(Store &store, const Incoming &incoming, const std::function<bool(const Key &)> &held) {
    Report report;
    Snapshot base = incoming.base;
    for (const Record &record : incoming.records) {
        const Key key{record.kind, record.id};
        if (held(key)) continue;
        if (!record.body || !store.mergeRecord(record.kind, *record.body)) {
            report.unreadable += 1;
            continue;
        }
        report.written += 1;
        base.insert(key, Version::live(record.updatedAt));
    }
    for (const Record &record : incoming.deletions) {
        const Key key{record.kind, record.id};
        if (held(key)) continue;
        store.applyDeletion(record.kind, record.id, record.updatedAt);
        report.removed += 1;
        base.insert(key, Version::gone(record.updatedAt));
    }
    return {report, base};
}

QString defaultBasePath(const QString &documentPath) {
    return QFileInfo(documentPath).absolutePath() + QStringLiteral("/sync-base.json");
}

// Stored as the Rust client wrote it: a list of [key, version] pairs, with a
// version tagged `Live` or `Deleted`, so the two clients share a base file.
Snapshot loadBase(const QString &path) {
    Snapshot base;
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) return base;
    for (const QJsonValue &entry : QJsonDocument::fromJson(file.readAll()).array()) {
        const QJsonArray pair = entry.toArray();
        if (pair.size() != 2) continue;
        const QJsonObject key = pair.at(0).toObject();
        const auto kind = recordKindFromSerial(key.value(QStringLiteral("kind")).toString());
        if (!kind) continue;
        const QJsonObject version = pair.at(1).toObject();
        if (version.contains(QStringLiteral("Live"))) {
            if (const auto at = instantFromSerial(version.value(QStringLiteral("Live")).toString()))
                base.insert(Key{*kind, key.value(QStringLiteral("id")).toString()}, Version::live(*at));
        } else if (version.contains(QStringLiteral("Deleted"))) {
            if (const auto at = instantFromSerial(version.value(QStringLiteral("Deleted")).toString()))
                base.insert(Key{*kind, key.value(QStringLiteral("id")).toString()}, Version::gone(*at));
        }
    }
    return base;
}

bool saveBase(const Snapshot &base, const QString &path) {
    QJsonArray entries;
    for (auto it = base.begin(); it != base.end(); ++it) {
        const QJsonObject key{{QStringLiteral("kind"), recordKindSerial(it.key().kind)}, {QStringLiteral("id"), it.key().id}};
        const QJsonObject version{{it.value().deleted ? QStringLiteral("Deleted") : QStringLiteral("Live"), instantSerial(it.value().at)}};
        entries.append(QJsonArray{key, version});
    }
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly)) return false;
    file.write(QJsonDocument(entries).toJson(QJsonDocument::Compact));
    return file.commit();
}

Snapshot snapshotOf(const Store &store) {
    Snapshot snapshot;
    for (const Task &task : store.tasks()) snapshot.insert(Key{RecordKind::Task, task.id}, Version::live(task.updatedAt));
    for (const Project &project : store.projects()) snapshot.insert(Key{RecordKind::Project, project.id}, Version::live(project.updatedAt));
    for (const Section &section : store.sections()) snapshot.insert(Key{RecordKind::Section, section.id}, Version::live(section.updatedAt));
    for (const Label &label : store.labels()) snapshot.insert(Key{RecordKind::Label, label.id}, Version::live(label.updatedAt));
    for (const SavedFilter &filter : store.filters()) snapshot.insert(Key{RecordKind::Filter, filter.id}, Version::live(filter.updatedAt));
    // Markers last, so a record somehow both listed and deleted reads as deleted.
    for (const Tombstone &tombstone : store.tombstones()) snapshot.insert(Key{tombstone.kind, tombstone.id}, Version::gone(tombstone.deletedAt));
    return snapshot;
}

int liveCount(const Snapshot &snapshot) {
    int count = 0;
    for (const Version &version : snapshot)
        if (!version.deleted) ++count;
    return count;
}

} // namespace planner::sync
