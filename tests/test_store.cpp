#include <QTemporaryDir>
#include <QtTest>

#include "store.h"

using namespace planner;

static QDate d(int y, int m, int day) { return QDate(y, m, day); }
static QDateTime instant(int y, int m, int day) { return QDateTime(d(y, m, day), QTime(12, 0), QTimeZone::utc()); }

class TestStore : public QObject {
    Q_OBJECT
    static TaskId task(Store &store, const char *content) { return store.addTask(Task::create(inboxId(), QString::fromUtf8(content), instant(2026, 7, 30))); }
private slots:
    void aFreshStoreHasAnInboxAndNothingElse() {
        QTemporaryDir dir;
        LoadOutcome outcome;
        const Store store = Store::openAt(dir.path() + "/planner.json", &outcome);
        QCOMPARE(outcome.kind, LoadOutcome::Fresh);
        QCOMPARE(store.projects().size(), 1);
        QVERIFY(store.projects()[0].isInbox());
        QVERIFY(store.tasks().isEmpty());
    }
    void aStoreSurvivesARoundTripThroughTheFile() {
        QTemporaryDir dir;
        const QString path = dir.path() + "/planner.json";
        TaskId id;
        {
            Store store = Store::openAt(path);
            id = task(store, "Water the plants");
            store.taskMut(id)->due = Due::at(d(2026, 8, 1), QTime(9, 0));
            store.taskMut(id)->reminders.append(Reminder::beforeDue(30));
            store.taskMut(id)->deadline = d(2026, 8, 3);
            store.taskMut(id)->priority = Priority::P1;
            store.taskMut(id)->labels.append(store.labelForName("errand"));
            QVERIFY(!store.save());
        }
        LoadOutcome outcome;
        const Store reopened = Store::openAt(path, &outcome);
        QCOMPARE(outcome.kind, LoadOutcome::Loaded);
        const Task *t = reopened.task(id);
        QVERIFY(t);
        QCOMPARE(t->content, "Water the plants");
        QCOMPARE(t->due->date, d(2026, 8, 1));
        QCOMPARE(t->due->time, QTime(9, 0));
        QCOMPARE(t->deadline, d(2026, 8, 3));
        QCOMPARE(t->priority, Priority::P1);
        QCOMPARE(t->reminders.size(), 1);
        QCOMPARE(t->reminders[0].trigger.minutes, 30);
        QCOMPARE(t->addedAt, instant(2026, 7, 30));
        QCOMPARE(reopened.labels().size(), 1);
        for (const QFileInfo &entry : QDir(dir.path()).entryInfoList(QDir::Files)) QVERIFY(!entry.fileName().endsWith(".tmp"));
    }
    void aFileWrittenByTheRustAppReads() {
        QTemporaryDir dir;
        const QString path = dir.path() + "/planner.json";
        QFile file(path);
        QVERIFY(file.open(QIODevice::WriteOnly));
        file.write(R"({"version":1,"projects":[{"id":"inbox","name":"Inbox","color":"slate","view_style":"list","sort_by":"manual","order":-1},
            {"id":"w1","name":"Work","color":"blue","sections":[{"id":"s1","name":"Doing","order":0}],"view_style":"board","sort_by":"manual","order":0}],
            "labels":[{"id":"l1","name":"email","color":"blue","order":0}],
            "tasks":[{"id":"t1","content":"Email Sam","project_id":"w1","section_id":"s1","due":{"date":"2026-07-31","time":"09:00:00",
            "recurrence":{"interval":2,"unit":"week","weekdays":["Mon","Fri"],"from_completion":true,"end":{"kind":"after","remaining":2}}},
            "deadline":"2026-08-01","priority":"P2","labels":["l1"],"reminders":[{"id":"r1","trigger":{"kind":"before-due","minutes":30}}],
            "pinned":true,"added_at":"2026-07-01T12:00:00.123456789Z","updated_at":"2026-07-01T12:00:00Z","order":0}],"filters":[]})");
        file.close();
        LoadOutcome outcome;
        const Store store = Store::openAt(path, &outcome);
        QCOMPARE(outcome.kind, LoadOutcome::Loaded);
        const Task *t = store.task("t1");
        QVERIFY(t);
        QCOMPARE(t->sectionId, QString("s1"));
        QCOMPARE(t->due->time, QTime(9, 0));
        QVERIFY(t->due->recurrence->fromCompletion);
        QCOMPARE(t->due->recurrence->weekdays, (QList<Weekday>{Weekday::Mon, Weekday::Fri}));
        QCOMPARE(t->due->recurrence->end, End::after(2));
        QCOMPARE(t->priority, Priority::P2);
        QVERIFY(t->pinned);
        QCOMPARE(t->addedAt, QDateTime(d(2026, 7, 1), QTime(12, 0, 0, 123), QTimeZone::utc()));
        QCOMPARE(store.project("w1")->viewStyle, ViewStyle::Board);
        QCOMPARE(store.project("w1")->sections[0].name, "Doing");
        // And writes back the same spellings.
        const QJsonObject json = t->toJson();
        QCOMPARE(json.value("priority").toString(), "P2");
        QCOMPARE(json.value("due").toObject().value("time").toString(), "09:00:00");
        QCOMPARE(json.value("reminders").toArray()[0].toObject().value("trigger").toObject().value("kind").toString(), "before-due");
        QVERIFY(!json.contains("description"));
        QVERIFY(!json.contains("checked"));
    }
    void aCorruptFileIsSetAsideAndTheAppStillStarts() {
        QTemporaryDir dir;
        const QString path = dir.path() + "/planner.json";
        QFile file(path);
        QVERIFY(file.open(QIODevice::WriteOnly));
        file.write("{ this is not json");
        file.close();
        LoadOutcome outcome;
        const Store store = Store::openAt(path, &outcome);
        QCOMPARE(outcome.kind, LoadOutcome::Recovered);
        QVERIFY(QFile::exists(outcome.backup));
        QVERIFY(!QFile::exists(path));
        QVERIFY(store.tasks().isEmpty());
        QVERIFY(!store.save());
    }
    void aFileFromANewerVersionIsNeverOverwritten() {
        QTemporaryDir dir;
        const QString path = dir.path() + "/planner.json";
        const QByteArray future = QStringLiteral(R"({"version":%1,"projects":[],"labels":[],"tasks":[]})").arg(kSchemaVersion + 1).toUtf8();
        QFile file(path);
        QVERIFY(file.open(QIODevice::WriteOnly));
        file.write(future);
        file.close();
        LoadOutcome outcome;
        const Store store = Store::openAt(path, &outcome);
        QCOMPARE(outcome.kind, LoadOutcome::ReadOnly);
        QCOMPARE(outcome.version, kSchemaVersion + 1);
        QVERIFY(store.isReadOnly());
        const auto error = store.save();
        QVERIFY(error && error->kind == SaveError::Newer);
        QFile check(path);
        check.open(QIODevice::ReadOnly);
        QCOMPARE(check.readAll(), future);
        QVERIFY(store.project(inboxId()));   // and a lost Inbox comes back
    }
    void deletingAProjectTakesItsSubprojectsAndTheirTasks() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const ProjectId parent = store.addProject(Project::create("Work", Color::Blue));
        Project child = Project::create("Admin", Color::Teal);
        child.parentId = parent;
        const ProjectId childId = store.addProject(child);
        store.addTask(Task::create(childId, "File the thing", instant(2026, 7, 30)));
        const TaskId survivor = task(store, "Untouched inbox task");
        const auto removed = store.removeProject(parent);
        QVERIFY(removed);
        QCOMPARE(removed->projects.size(), 2);
        QCOMPARE(removed->tasks.size(), 1);
        QVERIFY(!store.project(childId));
        QVERIFY(store.task(survivor));
        store.restoreProject(*removed);
        QVERIFY(store.project(childId));
        QVERIFY(!store.removeProject(inboxId()));
        store.projectMut(parent)->parentId = childId;   // a cycle
        QCOMPARE(store.projectAndDescendants(parent).size(), 2);
    }
    void sections() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const ProjectId work = store.addProject(Project::create("Work", Color::Blue));
        const ProjectId home = store.addProject(Project::create("Home", Color::Teal));
        const SectionId first = store.projectMut(work)->addSection(Section::create("Doing"));
        const SectionId second = store.projectMut(work)->addSection(Section::create("Done"));
        Task t = Task::create(work, "In a column", instant(2026, 7, 30));
        t.sectionId = first;
        const TaskId taskId = store.addTask(t);
        const auto removed = store.removeSection(first, instant(2026, 7, 31));
        QVERIFY(removed);
        QVERIFY(!store.task(taskId)->sectionId);
        QCOMPARE(store.task(taskId)->updatedAt, instant(2026, 7, 31));
        store.restoreSection(*removed, instant(2026, 8, 1));
        QCOMPARE(store.task(taskId)->sectionId, first);
        const auto ordered = store.project(work)->sectionsOrdered();
        QCOMPARE(ordered[0]->name, "Doing");
        QCOMPARE(ordered[1]->id, second);
        // A task that moved project is not dragged back by an undo.
        const auto again = store.removeSection(first, instant(2026, 7, 31));
        store.taskMut(taskId)->projectId = home;
        store.restoreSection(*again, instant(2026, 8, 1));
        QCOMPARE(store.task(taskId)->projectId, home);
        QVERIFY(!store.task(taskId)->sectionId);
        QVERIFY(store.renameSection(second, "Finished"));
        QCOMPARE(store.section(second).second->name, "Finished");
    }
    void labels() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const LabelId label = store.labelForName("Errand");
        QCOMPARE(store.labelForName("errand"), label);
        QCOMPARE(store.labels().size(), 1);
        const TaskId a = task(store, "One");
        const TaskId b = task(store, "Two");
        store.taskMut(a)->addLabel(label);
        store.taskMut(b)->addLabel(label);
        QCOMPARE(store.labelCounts().value(label), 2);
        store.completeTask(a, instant(2026, 7, 30), d(2026, 7, 30));
        QCOMPARE(store.labelCounts().value(label), 1);
        store.removeLabel(label, instant(2026, 7, 31));
        QVERIFY(!store.label(label));
        QVERIFY(store.task(b)->labels.isEmpty());
    }
    void subtasksAndCompletion() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const TaskId parent = task(store, "Parent");
        const TaskId child = task(store, "Child");
        const TaskId grandchild = task(store, "Grandchild");
        store.taskMut(child)->parentId = parent;
        store.taskMut(grandchild)->parentId = child;
        QCOMPARE(store.progress(inboxId()), std::make_pair(0, 3));
        store.completeTask(parent, instant(2026, 7, 30), d(2026, 7, 30));
        QVERIFY(store.task(parent)->checked && store.task(child)->checked && store.task(grandchild)->checked);
        store.uncompleteTask(grandchild, instant(2026, 7, 31));
        QVERIFY(!store.task(child)->checked && !store.task(parent)->checked);
        const auto removed = store.removeTask(parent);
        QCOMPARE(removed.size(), 3);
        QVERIFY(store.tasks().isEmpty());
        store.restoreTasks(removed);
        QCOMPARE(store.task(child)->content, "Child");
    }
    void completingARecurringParentLeavesItsSubtasksAlone() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const TaskId parent = task(store, "Weekly review");
        const TaskId child = task(store, "Step one");
        store.taskMut(child)->parentId = parent;
        store.taskMut(parent)->due = Due::on(d(2026, 7, 30)).repeating(Recurrence::every(1, Unit::Week));
        const auto outcome = store.completeTask(parent, instant(2026, 7, 30), d(2026, 7, 30));
        QCOMPARE(outcome->kind, Completion::Rescheduled);
        QCOMPARE(store.task(parent)->due->date, d(2026, 8, 6));
        QVERIFY(!store.task(child)->checked);
    }
    void orderingAndMoving() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const ProjectId work = store.addProject(Project::create("Work", Color::Blue));
        const TaskId first = task(store, "Inbox one");
        const TaskId second = task(store, "Inbox two");
        const TaskId elsewhere = store.addTask(Task::create(work, "Work one", instant(2026, 7, 30)));
        QCOMPARE(store.task(first)->order, 0);
        QCOMPARE(store.task(second)->order, 1);
        QCOMPARE(store.task(elsewhere)->order, 0);
        // Move the second above the first.
        QVERIFY(store.moveTask(second, inboxId(), std::nullopt, 0, instant(2026, 7, 31)));
        QCOMPARE(store.tasksIn(inboxId(), std::nullopt)[0]->id, second);
        QCOMPARE(store.task(first)->order, 1);
        // Move across projects: the vacated list closes up and subtasks follow.
        const TaskId child = task(store, "Child");
        store.taskMut(child)->parentId = first;
        QVERIFY(store.moveTask(first, work, std::nullopt, 0, instant(2026, 7, 31)));
        QCOMPARE(store.task(child)->projectId, work);
        QCOMPARE(store.task(second)->order, 0);
        QCOMPARE(store.tasksIn(work, std::nullopt).size(), 2);
        // A section from another project is refused.
        const SectionId section = store.projectMut(work)->addSection(Section::create("Doing"));
        QVERIFY(!store.moveTask(second, inboxId(), section, 0, instant(2026, 7, 31)));
        QVERIFY(store.moveTask(second, work, section, 0, instant(2026, 7, 31)));
        QCOMPARE(store.tasksIn(work, section).size(), 1);
    }
    void quickAddFilesByName() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const ProjectId work = store.addProject(Project::create("Work", Color::Blue));
        const SectionId admin = store.projectMut(work)->addSection(Section::create("Admin"));
        const QuickAdd parsed = parseQuickAdd("Email Sam #Work /Admin @email p2 friday 9am !30m", d(2026, 7, 30), store.vocabulary());
        const TaskId id = store.addFromQuickAdd(parsed, inboxId(), std::nullopt, instant(2026, 7, 30));
        const Task *t = store.task(id);
        QCOMPARE(t->projectId, work);
        QCOMPARE(t->sectionId, admin);
        QCOMPARE(t->labels.size(), 1);
        QCOMPARE(store.label(t->labels[0])->name, "email");
        QCOMPARE(t->priority, Priority::P2);
        QCOMPARE(t->reminders[0].trigger.minutes, 30);
        // An unknown project is not created; the task lands in the default.
        const QuickAdd unknown = parseQuickAdd("Buy milk #Nowhere", d(2026, 7, 30), store.vocabulary());
        QCOMPARE(store.task(store.addFromQuickAdd(unknown, inboxId(), std::nullopt, instant(2026, 7, 30)))->projectId, inboxId());
        QCOMPARE(store.projects().size(), 2);
        // A default section only applies in its own project.
        const QuickAdd plain = parseQuickAdd("Chase it", d(2026, 7, 30), store.vocabulary());
        QCOMPARE(store.task(store.addFromQuickAdd(plain, work, admin, instant(2026, 7, 30)))->sectionId, admin);
        QCOMPARE(store.task(store.addFromQuickAdd(plain, inboxId(), admin, instant(2026, 7, 30)))->sectionId, std::nullopt);
    }
};

QTEST_APPLESS_MAIN(TestStore)
#include "test_store.moc"
