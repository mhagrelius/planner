#include <QTemporaryDir>
#include <QtTest>

#include "agent.h"
#include "store.h"

using namespace planner;

static QDate today() { return QDate(2026, 7, 30); }
static QDateTime now() { return QDateTime(today(), QTime(12, 0), QTimeZone::utc()); }

// Split a line into arguments, honouring single quotes, so a test reads like
// the command it stands for.
static QStringList shellWords(const QString &line) {
    QStringList words;
    QString current;
    bool quoted = false, started = false;
    for (const QChar c : line) {
        if (c == u'\'') { quoted = !quoted; started = true; }
        else if (c.isSpace() && !quoted) { if (started) { words << current; current.clear(); started = false; } }
        else { current += c; started = true; }
    }
    if (started) words << current;
    return words;
}

class TestAgent : public QObject {
    Q_OBJECT
    QTemporaryDir dir;
    Store store() { return Store::openAt(dir.path() + "/planner.json"); }
    static agent::Result run(Store &store, const char *line) { return agent::run(store, shellWords(QString::fromUtf8(line)), now(), today()); }
    static QJsonObject json(Store &store, const char *line) {
        const QString text = agent::render(run(store, line));
        return QJsonDocument::fromJson(text.toUtf8()).object();
    }
private slots:
    void aTaskIsAddedFromTheSameLineTheDialogWouldTake() {
        Store s = store();
        const ProjectId work = s.addProject(Project::create("Work", Color::Blue), now());
        s.addSection(Section::create(work, "Admin"), now());
        const QJsonObject response = json(s, "add Email Sam about the lease #Work /Admin @email p2 friday 9am");
        QCOMPARE(response["ok"].toBool(), true);
        QCOMPARE(response["action"].toString(), "added");
        const QJsonObject task = response["task"].toObject();
        QCOMPARE(task["content"].toString(), "Email Sam about the lease");
        QCOMPARE(task["project"].toString(), "Work");
        QCOMPARE(task["section"].toString(), "Admin");
        QCOMPARE(task["priority"].toString(), "p2");
        QCOMPARE(task["labels"].toArray()[0].toString(), "email");
        QCOMPARE(task["due"].toString(), "2026-07-31 09:00");
        QCOMPARE(json(s, "add Buy milk")["task"].toObject()["project"].toString(), "Inbox");
        QCOMPARE(json(s, "add Water the plants every other monday")["task"].toObject()["repeats"].toString(), "every other monday");
        QCOMPARE(json(s, "add Something #Wrok")["task"].toObject()["project"].toString(), "Inbox");
        QCOMPARE(s.projects().size(), 2);
        const agent::Result tokensOnly = run(s, "add #Work p1 friday");
        QVERIFY(!tokensOnly.ok);
        QCOMPARE(tokensOnly.error.kind, "bad-value");
        QVERIFY(!tokensOnly.error.hint.isEmpty());
    }
    void aSubtaskSharesItsParentsProject() {
        Store s = store();
        const ProjectId work = s.addProject(Project::create("Work", Color::Blue), now());
        s.addTask(Task::create(work, "Move house", now()));
        const QJsonObject task = json(s, "subtask 'Move house' Pack the books #Inbox p1")["task"].toObject();
        QCOMPARE(task["content"].toString(), "Pack the books");
        QCOMPARE(task["project"].toString(), "Work");
        QCOMPARE(task["priority"].toString(), "p1");
    }
    void completing() {
        Store s = store();
        s.addTask(Task::create(inboxId(), "Email Sam", now()));
        QJsonObject response = json(s, "complete Email Sam");
        QCOMPARE(response["outcome"].toString(), "done");
        QCOMPARE(response["task"].toObject()["completed"].toBool(), true);
        const agent::Result again = run(s, "complete Email Sam");
        QCOMPARE(again.body["outcome"].toString(), "already-done");
        QVERIFY(!again.changedStore);
        run(s, "add Water the plants every week");
        response = json(s, "complete Water the plants");
        QCOMPARE(response["outcome"].toString(), "completed-and-repeats");
        QCOMPARE(response["next_due"].toString(), "2026-08-06");
        QVERIFY(!response["task"].toObject().contains("completed"));
        s.addTask(Task::create(inboxId(), "Still to do", now()));
        const agent::Result reopen = run(s, "reopen Still to do");
        QCOMPARE(reopen.body["reopened"].toBool(), false);
        QVERIFY(!reopen.changedStore);
        s.addTask(Task::create(inboxId(), "Move house", now()));
        run(s, "subtask 'Move house' Pack");
        run(s, "complete Move house");
        run(s, "reopen Pack");
        QVERIFY(!json(s, "show Move house")["task"].toObject().contains("completed"));
    }
    void namingTheThingYouMeant() {
        Store s = store();
        const ProjectId work = s.addProject(Project::create("Work", Color::Blue), now());
        s.addTask(Task::create(inboxId(), "Email Sam", now()));
        s.addTask(Task::create(work, "Email Sam again", now()));
        const agent::Result ambiguous = run(s, "complete Email");
        QCOMPARE(ambiguous.error.kind, "ambiguous");
        QCOMPARE(ambiguous.error.candidates.size(), 2);
        for (const agent::Candidate &candidate : ambiguous.error.candidates) QVERIFY(!candidate.id.isEmpty() && !candidate.context.isEmpty());
        for (const Task &task : s.tasks()) QVERIFY(!task.checked);
        const TaskId old = s.addTask(Task::create(inboxId(), "Weekly report", now()));
        s.completeTask(old, now(), today());
        const TaskId current = s.addTask(Task::create(inboxId(), "Weekly report", now()));
        QCOMPARE(agent::resolveTask(s, "Weekly report", nullptr), current);
        const TaskId exact = s.addTask(Task::create(inboxId(), "Pack", now()));
        s.addTask(Task::create(inboxId(), "Pack the books", now()));
        QCOMPARE(agent::resolveTask(s, "Pack", nullptr), exact);
        const TaskId done = s.addTask(Task::create(inboxId(), "Filed the taxes", now()));
        s.completeTask(done, now(), today());
        QCOMPARE(json(s, "reopen Filed the taxes")["ok"].toBool(), true);
        // Ambiguous by name, exact by id.
        s.addTask(Task::create(inboxId(), "Email Sam", now()));
        QCOMPARE(run(s, "show Email Sam").error.kind, "ambiguous");
        const TaskId first = s.tasks()[0].id;
        QCOMPARE(json(s, qPrintable("show " + first))["task"].toObject()["id"].toString(), first);
        const agent::Result missing = run(s, "complete Nothing like this");
        QCOMPARE(missing.error.kind, "not-found");
        QVERIFY(missing.error.hint.contains("search"));
        QCOMPARE(run(s, qPrintable("update " + first + " project=Nowhere")).error.kind, "not-found");
    }
    void listingAndUnderstanding() {
        Store s = store();
        s.addProject(Project::create("Work", Color::Blue), now());
        run(s, "add Urgent thing #Work p1 today");
        run(s, "add Lesser thing #Work p3 today");
        run(s, "add Home thing today");
        QJsonObject response = json(s, "list #Work & p1");
        QCOMPARE(response["count"].toInt(), 1);
        QCOMPARE(response["tasks"].toArray()[0].toObject()["content"].toString(), "Urgent thing");
        Store t = store();
        run(t, "add Later next friday");
        run(t, "add Sooner tomorrow");
        run(t, "add Undated");
        QStringList order;
        for (const QJsonValue &task : json(t, "list")["tasks"].toArray()) order << task.toObject()["content"].toString();
        QCOMPARE(order, (QStringList{"Sooner", "Later", "Undated"}));
        Store u = store();
        for (int i = 0; i < 10; ++i) run(u, qPrintable(QStringLiteral("add Task number %1").arg(i)));
        response = json(u, "list limit=3");
        QCOMPARE(response["count"].toInt(), 3);
        QCOMPARE(response["matched"].toInt(), 10);
        QCOMPARE(response["truncated"].toBool(), true);
        QVERIFY(!json(t, "list").contains("truncated"));
        run(t, "complete Undated");
        QCOMPARE(json(t, "list")["count"].toInt(), 2);
        QCOMPARE(json(t, "list completed")["count"].toInt(), 1);
        const agent::Result bad = run(t, "list due: nonsenseday");
        QCOMPARE(bad.error.kind, "bad-query");
        QVERIFY(bad.error.hint.contains("help list"));
    }
    void overviewShowAndSearch() {
        Store s = store();
        const ProjectId work = s.addProject(Project::create("Work", Color::Blue), now());
        s.addSection(Section::create(work, "Admin"), now());
        run(s, "add Overdue thing #Work yesterday");
        run(s, "add Due today #Work today @email");
        run(s, "add Inbox thing");
        run(s, "add Done thing");
        run(s, "complete Done thing");
        const QJsonObject response = json(s, "overview");
        QJsonObject workView;
        for (const QJsonValue &project : response["projects"].toArray())
            if (project.toObject()["name"].toString() == "Work") workView = project.toObject();
        QCOMPARE(workView["sections"].toArray()[0].toString(), "Admin");
        QCOMPARE(workView["open"].toInt(), 2);
        QCOMPARE(response["labels"].toArray()[0].toObject()["name"].toString(), "email");
        QCOMPARE(response["labels"].toArray()[0].toObject()["open"].toInt(), 1);
        const QJsonObject counts = response["counts"].toObject();
        QCOMPARE(counts["open"].toInt(), 3);
        QCOMPARE(counts["completed"].toInt(), 1);
        QCOMPARE(counts["overdue"].toInt(), 1);
        QCOMPARE(counts["due_today"].toInt(), 1);
        QCOMPARE(counts["inbox"].toInt(), 1);
        run(s, "add Move house friday !30m");
        run(s, "subtask 'Move house' Pack the books");
        s.taskMut(*agent::resolveTask(s, "Move house", nullptr))->description = "Ring the agent first";
        const QJsonObject shown = json(s, "show Move house")["task"].toObject();
        QCOMPARE(shown["description"].toString(), "Ring the agent first");
        QCOMPARE(shown["subtasks"].toArray()[0].toObject()["content"].toString(), "Pack the books");
        QCOMPARE(shown["reminders"].toArray()[0].toString(), "30 minutes before");
        s.addProject(Project::create("Leasehold", Color::Blue), now());
        run(s, "add Email Sam about the lease");
        QStringList kinds;
        for (const QJsonValue &hit : json(s, "search lease")["hits"].toArray()) kinds << hit.toObject()["kind"].toString();
        QVERIFY(kinds.contains("task") && kinds.contains("project"));
    }
    void updating() {
        Store s = store();
        s.addProject(Project::create("Work", Color::Blue), now());
        run(s, "add Email Sam");
        QJsonObject response = json(s, "update Email Sam due=next friday 9am");
        QCOMPARE(response["task"].toObject()["due"].toString(), "2026-08-07 09:00");
        QCOMPARE(response["applied"].toArray()[0].toString(), "due → 2026-08-07 09:00");
        run(s, "add Water the plants");
        QCOMPARE(json(s, "update Water due=every 3 days")["task"].toObject()["repeats"].toString(), "every 3 days");
        response = json(s, "update Water due=none");
        QVERIFY(!response["task"].toObject().contains("due"));
        QVERIFY(!response["task"].toObject().contains("repeats"));
        const agent::Result badDate = run(s, "update Email due=sometime friday");
        QCOMPARE(badDate.error.kind, "bad-date");
        QVERIFY(badDate.error.message.contains("sometime"));
        run(s, "update Email priority=p1");
        const agent::Result same = run(s, "update Email priority=p1");
        QVERIFY(same.body["applied"].toArray().isEmpty());
        QVERIFY(!same.changedStore);
        response = json(s, "update Email project=Work priority=p2 add-label=urgent pinned=true");
        QCOMPARE(response["task"].toObject()["project"].toString(), "Work");
        QCOMPARE(response["task"].toObject()["priority"].toString(), "p2");
        QCOMPARE(response["task"].toObject()["labels"].toArray()[0].toString(), "urgent");
        QCOMPARE(response["task"].toObject()["pinned"].toBool(), true);
        QCOMPARE(response["applied"].toArray().size(), 4);
        QCOMPARE(s.labels().size(), 1);
        response = json(s, "update Email remove-label=urgent");
        QVERIFY(!response["task"].toObject().contains("labels"));
        QCOMPARE(s.labels().size(), 1);
        // Sections are looked for in the project the task is moving to.
        const ProjectId work = s.projectByName("Work")->id;
        s.addSection(Section::create(work, "Doing"), now());
        run(s, "add Chase Pat");
        response = json(s, "update Chase project=Work section=Doing");
        QCOMPARE(response["task"].toObject()["section"].toString(), "Doing");
        const agent::Result noSection = run(s, "update Chase section=Blocked");
        QCOMPARE(noSection.error.kind, "not-found");
        QVERIFY(noSection.error.hint.contains("Doing"));
        // A moved task takes its subtasks with it.
        run(s, "add Move house");
        run(s, "subtask 'Move house' Pack the books");
        run(s, "update Move house project=Work");
        QCOMPARE(json(s, "show Pack the books")["task"].toObject()["project"].toString(), "Work");
    }
    void deletingAndProjects() {
        Store s = store();
        run(s, "add Move house");
        run(s, "subtask 'Move house' Pack the books");
        run(s, "subtask 'Move house' Book a van");
        QCOMPARE(json(s, "delete Move house")["count"].toInt(), 3);
        QVERIFY(s.tasks().isEmpty());
        s.addProject(Project::create("Home", Color::Blue), now());
        QJsonObject response = json(s, "add-project Loft conversion parent=Home");
        QCOMPARE(response["project"].toObject()["name"].toString(), "Loft conversion");
        QCOMPARE(response["project"].toObject()["parent"].toString(), "Home");
        s.addProject(Project::create("Work", Color::Blue), now());
        run(s, "add Email Sam #Work");
        run(s, "add Ring Pat #Work");
        response = json(s, "remove-project Work");
        QCOMPARE(response["name"].toString(), "Work");
        QCOMPARE(response["projects"].toInt(), 1);
        QCOMPARE(response["tasks"].toInt(), 2);
        QCOMPARE(run(s, "remove-project Inbox").error.kind, "refused");
        QCOMPARE(run(s, "rename-project Inbox Elsewhere").error.kind, "refused");
        QVERIFY(s.project(inboxId()));
    }
    void theWire() {
        Store s = store();
        const QJsonObject good = json(s, "overview");
        QCOMPARE(good["ok"].toBool(), true);
        QCOMPARE(good["action"].toString(), "overview");
        const QJsonObject bad = json(s, "show Nothing");
        QCOMPARE(bad["ok"].toBool(), false);
        QCOMPARE(bad["error"].toString(), "not-found");
        QVERIFY(bad["message"].toString().endsWith('.'));
        const QString help = agent::render(run(s, "help"));
        QVERIFY(help.startsWith("planner agent"));
        QVERIFY(agent::render(run(s, "")).startsWith("planner agent"));
        QVERIFY(agent::render(run(s, "help update")).contains("ARGUMENTS"));
        QVERIFY(agent::render(run(s, "add help")).contains("QUICK-ADD LINES"));
        QVERIFY(!agent::render(run(s, "help overview")).contains("QUICK-ADD LINES"));
        QCOMPARE(run(s, "frobnicate").error.kind, "unknown-verb");
        QCOMPARE(agent::canonicalVerb("done"), QString("complete"));
        QCOMPARE(json(s, "describe")["verbs"].toArray().size(), agent::verbs().size());
        for (const QString &line : agent::helpOverview().split('\n')) {
            if (line.contains("QUICK-ADD")) break;
            QVERIFY2(line.size() <= 88, qPrintable(line));
        }
    }
    void aReadOnlyStoreRefusesWrites() {
        QTemporaryDir other;
        const QString path = other.path() + "/planner.json";
        QFile file(path);
        file.open(QIODevice::WriteOnly);
        file.write(QStringLiteral(R"({"version":%1,"projects":[],"labels":[],"tasks":[]})").arg(kSchemaVersion + 1).toUtf8());
        file.close();
        Store s = Store::openAt(path);
        QCOMPARE(run(s, "add Anything").error.kind, "read-only");
        QCOMPARE(json(s, "overview")["ok"].toBool(), true);
    }
};

QTEST_APPLESS_MAIN(TestAgent)
#include "test_agent.moc"
