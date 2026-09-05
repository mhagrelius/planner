#include <QTemporaryDir>
#include <QtTest>

#include "query.h"
#include "store.h"

using namespace planner;

static QDate today() { return QDate(2026, 7, 30); }
static QDate d(int y, int m, int day) { return QDate(y, m, day); }
static QDateTime now() { return QDateTime(today(), QTime(12, 0), QTimeZone::utc()); }

class TestQuery : public QObject {
    Q_OBJECT
    QTemporaryDir dir;
    Store store() { return Store::openAt(dir.path() + "/planner.json"); }
    static TaskId task(Store &store, const char *content) { return store.addTask(Task::create(inboxId(), QString::fromUtf8(content), now())); }
    static QStringList matching(const Store &store, const char *source) {
        const auto query = Query::parse(QString::fromUtf8(source));
        if (!query) return {"<parse error>"};
        QStringList names;
        for (const Task *task : query->run(store, today())) names << task->content;
        names.sort();
        return names;
    }
private slots:
    void aBareDatePhraseFiltersOnTheDueDate() {
        const auto query = Query::parse("today");
        QVERIFY(query);
        Term term;
        term.kind = Term::Due;
        term.date = {DateFilter::On, "today"};
        QVERIFY(query->lists() == QList<Filter>{Filter::of(term)});
        QVERIFY(Query::parse("due: friday")->lists() == Query::parse("due friday")->lists());
        QVERIFY(Query::parse("due: friday")->lists() == Query::parse("friday")->lists());
    }
    void beforeAndAfterParseOnEitherField() {
        Term deadline;
        deadline.kind = Term::Deadline;
        deadline.date = {DateFilter::Before, "friday"};
        QVERIFY(Query::parse("deadline before: friday")->lists() == QList<Filter>{Filter::of(deadline)});
        Term due;
        due.kind = Term::Due;
        due.date = {DateFilter::After, "monday"};
        QVERIFY(Query::parse("due after monday")->lists() == QList<Filter>{Filter::of(due)});
    }
    void operatorsBindAndMoreTightlyThanOr() {
        const auto query = Query::parse("p1 & @work | p2");
        Term p1; p1.kind = Term::PriorityIs; p1.priority = Priority::P1;
        Term p2; p2.kind = Term::PriorityIs; p2.priority = Priority::P2;
        Term work; work.kind = Term::LabelIs; work.name = "work";
        QVERIFY(query->lists() == QList<Filter>{Filter::either(Filter::both(Filter::of(p1), Filter::of(work)), Filter::of(p2))});
        QCOMPARE(Query::parse("p1 & (@work | @home)")->lists()[0].kind, Filter::And);
        QCOMPARE(Query::parse("today, overdue")->lists().size(), 2);
    }
    void names() {
        auto query = Query::parse("#My Big Project");
        QCOMPARE(query->lists()[0].term.name, "My Big Project");
        QVERIFY(!query->lists()[0].term.includeSubprojects);
        query = Query::parse(R"(#R\&D)");
        QCOMPARE(query->lists()[0].term.name, "R&D");
        query = Query::parse("##Work");
        QVERIFY(query->lists()[0].term.includeSubprojects);
    }
    void aBrokenQuerySaysWhereItBroke() {
        QueryError error;
        QVERIFY(!Query::parse("p1 &", &error));
        QVERIFY(error.message.contains("ends where a condition was expected"));
        QVERIFY(!Query::parse("(p1", &error));
        QCOMPARE(error.at, 0);
        QVERIFY(error.message.contains("never closed"));
        QVERIFY(!Query::parse("p1)", &error));
        QVERIFY(error.message.contains("never opened"));
        QVERIFY(!Query::parse("due: lunchtime", &error));
        QVERIFY(error.message.contains("not a date I understand"));
        QVERIFY(!Query::parse("@"));
        QVERIFY(!Query::parse("#"));
    }
    void completedTasksAreHiddenUnlessAskedFor() {
        Store s = store();
        const TaskId open = task(s, "Open");
        const TaskId done = task(s, "Done");
        s.taskMut(open)->due = Due::on(today());
        s.taskMut(done)->due = Due::on(today());
        s.completeTask(done, now(), today());
        QCOMPARE(matching(s, "today"), QStringList{"Open"});
        QCOMPARE(matching(s, "completed"), QStringList{"Done"});
        QCOMPARE(matching(s, "today & completed"), QStringList{"Done"});
        Store buried = store();
        s.completeTask(task(buried, "Done"), now(), today());
        buried.completeTask(buried.tasks()[0].id, now(), today());
        QCOMPARE(matching(buried, "!(p1 & !completed)"), QStringList{"Done"});
    }
    void todayIsDueTodayOrOverdue() {
        Store s = store();
        s.taskMut(task(s, "Overdue"))->due = Due::on(d(2026, 7, 1));
        s.taskMut(task(s, "Due today"))->due = Due::on(today());
        s.taskMut(task(s, "Later"))->due = Due::on(d(2026, 8, 30));
        QCOMPARE(matching(s, "due: today | overdue"), (QStringList{"Due today", "Overdue"}));
        QCOMPARE(matching(s, "due before: today"), QStringList{"Overdue"});
        QCOMPARE(matching(s, "due after: today"), QStringList{"Later"});
    }
    void aSavedFilterResolvesTodayWhenItRuns() {
        Store s = store();
        s.taskMut(task(s, "Tomorrow's task"))->due = Due::on(d(2026, 7, 31));
        const auto query = Query::parse("due: today");
        QVERIFY(query->run(s, today()).isEmpty());
        QCOMPARE(query->run(s, d(2026, 7, 31)).size(), 1);
    }
    void deadlinesAreSeparateFromDueDates() {
        Store s = store();
        Task *t = s.taskMut(task(s, "Report"));
        t->due = Due::on(d(2026, 8, 4));
        t->deadline = d(2026, 7, 31);
        QCOMPARE(matching(s, "deadline before: 2026-08-01"), QStringList{"Report"});
        QVERIFY(matching(s, "due before: 2026-08-01").isEmpty());
    }
    void noDateAndRecurring() {
        Store s = store();
        task(s, "Someday");
        s.taskMut(task(s, "Weekly"))->due = Due::on(today()).repeating(Recurrence::every(1, Unit::Week));
        QCOMPARE(matching(s, "no date"), QStringList{"Someday"});
        QCOMPARE(matching(s, "recurring"), QStringList{"Weekly"});
    }
    void labelsMatchByNameRegardlessOfCase() {
        Store s = store();
        const LabelId label = s.labelForName("Errand");
        const TaskId id = task(s, "Post office");
        task(s, "Something else");
        s.taskMut(id)->addLabel(label);
        QCOMPARE(matching(s, "@errand"), QStringList{"Post office"});
        QCOMPARE(matching(s, "@ERRAND"), QStringList{"Post office"});
        QCOMPARE(matching(s, "no labels"), QStringList{"Something else"});
    }
    void projectsAndSections() {
        Store s = store();
        const ProjectId parent = s.addProject(Project::create("Work", Color::Blue));
        Project child = Project::create("Admin", Color::Teal);
        child.parentId = parent;
        const ProjectId childId = s.addProject(child);
        s.addTask(Task::create(parent, "In Work", now()));
        s.addTask(Task::create(childId, "In Admin", now()));
        QCOMPARE(matching(s, "#Work"), QStringList{"In Work"});
        QCOMPARE(matching(s, "##Work"), (QStringList{"In Admin", "In Work"}));
        QVERIFY(matching(s, "#Nonexistent").isEmpty());
        const SectionId section = s.projectMut(parent)->addSection(Section::create("Doing"));
        Task inSection = Task::create(parent, "In progress", now());
        inSection.sectionId = section;
        s.addTask(inSection);
        QCOMPARE(matching(s, "/Doing"), QStringList{"In progress"});
    }
    void searchNegationAndCommas() {
        Store s = store();
        const TaskId plumber = task(s, "Call the plumber");
        task(s, "Unrelated");
        s.taskMut(plumber)->description = "about the leaking TAP";
        QCOMPARE(matching(s, "search: plumber"), QStringList{"Call the plumber"});
        QCOMPARE(matching(s, "search: tap"), QStringList{"Call the plumber"});
        Store t = store();
        const TaskId a = task(t, "Urgent errand");
        const TaskId b = task(t, "Urgent desk job");
        const LabelId label = t.labelForName("errand");
        t.taskMut(a)->priority = Priority::P1;
        t.taskMut(a)->addLabel(label);
        t.taskMut(b)->priority = Priority::P1;
        QCOMPARE(matching(t, "p1 & !@errand"), QStringList{"Urgent desk job"});
        Store u = store();
        u.taskMut(task(u, "Pinned"))->pinned = true;
        u.taskMut(task(u, "Urgent"))->priority = Priority::P1;
        task(u, "Neither");
        QCOMPARE(matching(u, "pinned, p1"), (QStringList{"Pinned", "Urgent"}));
        const TaskId parent = task(u, "Parent");
        u.taskMut(task(u, "Child"))->parentId = parent;
        QCOMPARE(matching(u, "subtask"), QStringList{"Child"});
        QVERIFY(matching(u, "!subtask").contains("Parent"));
    }
    void theBuiltInViewsAllParse() {
        for (const char *source : {"due: today | overdue", "overdue", "pinned", "completed", "no date", "recurring", "no labels", "due after: today", "p1 | p2", "#Inbox"})
            QVERIFY2(Query::parse(QString::fromUtf8(source)).has_value(), source);
    }
};

QTEST_APPLESS_MAIN(TestQuery)
#include "test_query.moc"
