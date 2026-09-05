#include <QTemporaryDir>
#include <QtTest>

#include "store.h"
#include "sync.h"

using namespace planner;
using namespace planner::sync;

static QDateTime at(int minute) { return QDateTime(QDate(2026, 8, 4), QTime(12, 0), QTimeZone::utc()).addSecs(60 * minute); }
static Key task(const char *id) { return Key{RecordKind::Task, QString::fromLatin1(id)}; }
static Snapshot snap(std::initializer_list<std::pair<Key, Version>> entries) {
    Snapshot s;
    for (const auto &[k, v] : entries) s.insert(k, v);
    return s;
}

// A server in memory with the real server's one rule: a write whose version is
// not newer than the stored one is refused.
class FakeRemote : public Remote {
public:
    struct Row { Version version; std::optional<QJsonObject> body; };
    QMap<Key, Row> rows;
    int calls = 0;
    std::optional<Snapshot> snapshot(Error *) override {
        ++calls;
        Snapshot s;
        for (auto it = rows.begin(); it != rows.end(); ++it) s.insert(it.key(), it.value().version);
        return s;
    }
    std::optional<QList<Record>> fetch(const QList<Key> &keys, Error *) override {
        QList<Record> out;
        for (const Key &key : keys)
            if (rows.contains(key) && !rows[key].version.deleted) out.append(Record{key.kind, key.id, rows[key].version.at, rows[key].body});
        return out;
    }
    bool push(const QList<Record> &records, Error *) override {
        for (const Record &r : records) {
            const Key key{r.kind, r.id};
            if (rows.contains(key) && rows[key].version.at >= r.updatedAt) continue;   // stale, refused
            rows[key] = Row{Version::live(r.updatedAt), r.body};
        }
        return true;
    }
    bool remove(const QList<Record> &records, Error *) override {
        for (const Record &r : records) {
            const Key key{r.kind, r.id};
            if (rows.contains(key) && rows[key].version.at >= r.updatedAt) continue;
            rows[key] = Row{Version::gone(r.updatedAt), std::nullopt};
        }
        return true;
    }
};

// One machine: its document and what it last agreed with the server.
struct Machine {
    Store store;
    Snapshot base;
    explicit Machine(const QString &path) : store(Store::openAt(path)) {}
    void sync(FakeRemote &remote) {
        const Snapshot local = snapshotOf(store);
        Error error;
        const auto incoming = gather(remote, base, local, [&](const Key &key) { return store.recordBody(key.kind, key.id); }, &error);
        QVERIFY2(incoming.has_value(), qPrintable(error.message));
        base = apply(store, *incoming, [](const Key &) { return false; }).second;
    }
    QStringList titles() const {
        QStringList out;
        for (const Task &t : store.tasks()) out << t.content;
        out.sort();
        return out;
    }
    TaskId add(const char *title, const QDateTime &now) { return store.addTask(Task::create(inboxId(), QString::fromUtf8(title), now)); }
};

class TestSync : public QObject {
    Q_OBJECT
    QTemporaryDir dir;
private slots:
    void nothingChangedMeansNothingToDo() {
        const Snapshot state = snap({{task("a"), Version::live(at(1))}});
        QVERIFY(plan(state, state, state).isEmpty());
        const Snapshot gone = snap({{task("a"), Version::gone(at(1))}});
        QVERIFY(plan(gone, gone, gone).isEmpty());
        QVERIFY(plan(state, Snapshot(), Snapshot()).isEmpty());
    }
    void oneSideMoved() {
        const Snapshot base = snap({{task("a"), Version::live(at(1))}});
        Plan p = plan(base, snap({{task("a"), Version::live(at(2))}}), base);
        QCOMPARE(p.push, QList<Key>{task("a")});
        QCOMPARE(p.size(), 1);
        p = plan(base, base, snap({{task("a"), Version::live(at(2))}}));
        QCOMPARE(p.pull, QList<Key>{task("a")});
        p = plan(Snapshot(), snap({{task("new"), Version::live(at(1))}}), Snapshot());
        QCOMPARE(p.push, QList<Key>{task("new")});
        QVERIFY(p.deleteLocal.isEmpty());
        p = plan(base, snap({{task("a"), Version::gone(at(2))}}), base);
        QCOMPARE(p.deleteRemote, QList<Key>{task("a")});
        p = plan(base, base, snap({{task("a"), Version::gone(at(2))}}));
        QCOMPARE(p.deleteLocal, QList<Key>{task("a")});
    }
    void anEditBeatsADeletionWhicheverSideDeleted() {
        const Snapshot base = snap({{task("a"), Version::live(at(1))}});
        Plan one = plan(base, snap({{task("a"), Version::gone(at(5))}}), snap({{task("a"), Version::live(at(2))}}));
        QCOMPARE(one.pull, QList<Key>{task("a")});
        QVERIFY(one.deleteRemote.isEmpty());
        Plan two = plan(base, snap({{task("a"), Version::live(at(2))}}), snap({{task("a"), Version::gone(at(5))}}));
        QCOMPARE(two.push, QList<Key>{task("a")});
        QVERIFY(two.deleteLocal.isEmpty());
    }
    void twoEditsKeepTheLaterOneAndTiesGoToTheServer() {
        const Snapshot base = snap({{task("a"), Version::live(at(1))}});
        QCOMPARE(plan(base, snap({{task("a"), Version::live(at(3))}}), snap({{task("a"), Version::live(at(2))}})).push, QList<Key>{task("a")});
        QCOMPARE(plan(base, snap({{task("a"), Version::live(at(2))}}), snap({{task("a"), Version::live(at(3))}})).pull, QList<Key>{task("a")});
        const Snapshot same = snap({{task("a"), Version::live(at(2))}});
        QVERIFY(plan(base, same, same).isEmpty());
    }
    void aMachineThatWasAwayCatchesUpInOnePass() {
        const Snapshot base = snap({{task("a"), Version::live(at(1))}, {task("b"), Version::live(at(1))}, {task("c"), Version::live(at(1))}});
        const Snapshot remote = snap({{task("a"), Version::live(at(5))}, {task("b"), Version::gone(at(6))}, {task("c"), Version::live(at(1))}, {task("d"), Version::live(at(7))}});
        const Plan p = plan(base, base, remote);
        QCOMPARE(p.pull, (QList<Key>{task("a"), task("d")}));
        QCOMPARE(p.deleteLocal, QList<Key>{task("b")});
        QVERIFY(p.push.isEmpty());
    }
    void kindsDoNotCollideOnTheSameId() {
        const Snapshot local = snap({{Key{RecordKind::Task, "x"}, Version::live(at(1))}, {Key{RecordKind::Project, "x"}, Version::live(at(1))}});
        QCOMPARE(plan(Snapshot(), local, Snapshot()).push.size(), 2);
    }
    void baseFilesRoundTripInTheRustClientsShape() {
        const Snapshot base = snap({{task("a"), Version::live(at(1))}, {Key{RecordKind::Section, "s"}, Version::gone(at(2))}});
        const QString path = dir.path() + "/sync-base.json";
        QVERIFY(saveBase(base, path));
        QFile file(path);
        file.open(QIODevice::ReadOnly);
        const QByteArray raw = file.readAll();
        QVERIFY(raw.contains("\"Live\"") && raw.contains("\"Deleted\"") && raw.contains("\"section\""));
        QCOMPARE(loadBase(path), base);
        QVERIFY(loadBase(dir.path() + "/missing.json").isEmpty());
        QCOMPARE(defaultBasePath("/x/y/planner.json"), "/x/y/sync-base.json");
    }
    // Two machines against one server, the scenario the Rust sync-check drove.
    void twoMachinesOneServer() {
        FakeRemote server;
        Machine a(dir.path() + "/a/planner.json");
        Machine b(dir.path() + "/b/planner.json");

        a.add("Email Sam", at(0));
        a.sync(server); b.sync(server);
        QVERIFY(b.titles().contains("Email Sam"));
        QVERIFY(b.store.project(inboxId()));

        // A pass that changes nothing sends nothing.
        const Snapshot before = snapshotOf(b.store);
        const int calls = server.calls;
        b.sync(server);
        QCOMPARE(snapshotOf(b.store), before);
        QCOMPARE(server.calls, calls + 1);

        // An edit propagates and does not bounce back or duplicate.
        TaskId id;
        for (const Task &t : b.store.tasks()) if (t.content == "Email Sam") id = t.id;
        b.store.taskMut(id)->content = "Email Sam about the lease";
        b.store.taskMut(id)->touch(at(10));
        b.sync(server); a.sync(server);
        QCOMPARE(a.titles(), QStringList{"Email Sam about the lease"});

        // A deletion survives rather than being resurrected.
        a.store.removeTask(id, at(20));
        a.sync(server); b.sync(server);
        QVERIFY(b.titles().isEmpty());
        b.sync(server); a.sync(server);
        QVERIFY(a.titles().isEmpty());

        // Both sides edit: one winner, the later one, and nothing lost.
        const TaskId contested = a.add("Contested", at(30));
        a.sync(server); b.sync(server);
        a.store.taskMut(contested)->content = "Contested by A"; a.store.taskMut(contested)->touch(at(40));
        b.store.taskMut(contested)->content = "Contested by B"; b.store.taskMut(contested)->touch(at(41));
        a.sync(server); b.sync(server); a.sync(server);
        QCOMPARE(a.titles(), QStringList{"Contested by B"});
        QCOMPARE(b.titles(), QStringList{"Contested by B"});

        // An edit beats a deletion.
        const TaskId survivor = a.add("Survivor", at(60));
        a.sync(server); b.sync(server);
        a.store.removeTask(survivor, at(70));
        b.store.taskMut(survivor)->content = "Survivor, edited"; b.store.taskMut(survivor)->touch(at(71));
        a.sync(server); b.sync(server); a.sync(server);
        QCOMPARE(a.titles(), (QStringList{"Contested by B", "Survivor, edited"}));
        QVERIFY(!a.store.isDeleted(RecordKind::Task, survivor));

        // Sections and labels travel as records of their own.
        const ProjectId work = a.store.addProject(Project::create("Work", Color::Blue), at(80));
        const SectionId doing = a.store.addSection(Section::create(work, "Doing"), at(80));
        a.store.labelForName("errand", at(80));
        a.sync(server); b.sync(server);
        QVERIFY(b.store.project(work));
        QCOMPARE(b.store.sectionsIn(work).size(), 1);
        QCOMPARE(b.store.sectionsIn(work)[0]->id, doing);
        QVERIFY(b.store.labelByName("errand"));

        // A machine away for several passes catches up in one.
        Machine c(dir.path() + "/c/planner.json");
        c.sync(server);
        QCOMPARE(c.titles(), a.titles());
        // And the next pass has nothing left to do.
        QVERIFY(plan(c.base, snapshotOf(c.store), *server.snapshot(nullptr)).isEmpty());
    }
    void aHeldRecordIsLeftForTheNextPass() {
        FakeRemote server;
        Machine a(dir.path() + "/h1/planner.json");
        Machine b(dir.path() + "/h2/planner.json");
        const TaskId id = a.add("Open in the pane", at(0));
        a.sync(server); b.sync(server);
        a.store.taskMut(id)->content = "Edited elsewhere"; a.store.taskMut(id)->touch(at(5));
        a.sync(server);
        const Snapshot local = snapshotOf(b.store);
        Error error;
        const auto incoming = gather(server, b.base, local, [&](const Key &k) { return b.store.recordBody(k.kind, k.id); }, &error);
        QVERIFY(incoming);
        const auto [report, base] = apply(b.store, *incoming, [&](const Key &k) { return k.id == id; });
        QCOMPARE(report.written, 0);
        QCOMPARE(b.store.task(id)->content, "Open in the pane");
        QVERIFY(!plan(base, snapshotOf(b.store), *server.snapshot(nullptr)).pull.isEmpty());   // offered again
    }
};

QTEST_APPLESS_MAIN(TestSync)
#include "test_sync.moc"
