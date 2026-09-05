#include <QTemporaryDir>
#include <QtTest>

#include "search.h"
#include "store.h"

using namespace planner;

static QDateTime now() { return QDateTime(QDate(2026, 7, 30), QTime(12, 0), QTimeZone::utc()); }

class TestSearch : public QObject {
    Q_OBJECT
private slots:
    void scoresRankExactPrefixWordStartAnywhere() {
        QVERIFY(*searchScore("Plumber", "plumber") > *searchScore("Plumber call", "plumber"));
        QVERIFY(*searchScore("Plumber call", "plumber") > *searchScore("Call the plumber", "plumber"));
        QVERIFY(*searchScore("Call the plumber", "plumber") > *searchScore("Replumber", "plumber"));
        QVERIFY(!searchScore("Dentist", "plumber"));
        QVERIFY(*searchScore("Book the dentist", "the") > *searchScore("Email Sam about the lease", "the"));
    }
    void theRankingTheDesignShows() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const ProjectId work = store.addProject(Project::create("Work", Color::Blue), now());
        for (const char *title : {"Email Sam about the lease", "Water the plants", "Book the dentist", "File the tax return"})
            store.addTask(Task::create(inboxId(), QString::fromUtf8(title), now()));
        store.addTask(Task::create(work, "Tidy the shared drive", now()));
        const TaskId done = store.addTask(Task::create(inboxId(), "Cancel the old broadband", now()));
        store.completeTask(done, now(), QDate(2026, 7, 30));
        QStringList titles;
        for (const Hit &hit : search(store, "the", 50)) titles << hit.title;
        QCOMPARE(titles, (QStringList{"Book the dentist", "Water the plants", "File the tax return", "Tidy the shared drive", "Email Sam about the lease", "Cancel the old broadband"}));
        const QList<Hit> hits = search(store, "the", 50);
        QCOMPARE(hits.last().context, "Inbox · completed");
        QVERIFY(hits.last().completed);
        QCOMPARE(hits[3].context, "Work");
        QVERIFY(search(store, "  ", 50).isEmpty());
        QCOMPARE(search(store, "the", 2).size(), 2);
    }
    void projectsAndLabelsAreHitsToo() {
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        store.addProject(Project::create("Work", Color::Blue), now());
        store.labelForName("workout", now());
        const QList<Hit> hits = search(store, "work", 50);
        QCOMPARE(hits.size(), 2);
        QCOMPARE(hits[0].kind, Hit::ProjectHit);
        QCOMPARE(hits[1].kind, Hit::LabelHit);
    }
};

QTEST_APPLESS_MAIN(TestSearch)
#include "test_search.moc"
