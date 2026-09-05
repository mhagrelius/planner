#include <QTemporaryDir>
#include <QtTest>

#include "present.h"
#include "store.h"

using namespace planner;

static QDate d(int y, int m, int day) { return QDate(y, m, day); }
static QDateTime now() { return QDateTime(d(2026, 7, 30), QTime(12, 0), QTimeZone::utc()); }

class TestPresent : public QObject {
    Q_OBJECT
private slots:
    void datesReadRelativeToToday() {
        const QDate today = d(2026, 9, 5);
        QCOMPARE(formatDate(today, today), "Today");
        QCOMPARE(formatDate(d(2026, 9, 6), today), "Tomorrow");
        QCOMPARE(formatDate(d(2026, 9, 4), today), "Yesterday");
        QCOMPARE(formatDate(d(2026, 9, 8), today), "Tue");
        QCOMPARE(formatDate(d(2026, 9, 14), today), "14 Sep");
        QCOMPARE(formatDate(d(2027, 1, 3), today), "3 Jan 2027");
        QCOMPARE(formatDue(today, QTime(9, 0), today), std::make_pair(QString("Today 09:00"), QString("today")));
        QCOMPARE(formatDue(d(2026, 9, 4), std::nullopt, today), std::make_pair(QString("Yesterday"), QString("overdue")));
    }
    void schedulesReadAsThePhrase() {
        const QDate today = d(2026, 7, 30);
        QCOMPARE(describeDue(std::nullopt, today), "No date");
        QCOMPARE(describeDeadline(std::nullopt, today), "No deadline");
        QCOMPARE(describeDue(Due::at(today, QTime(9, 0)), today), "Today at 09:00");
        QCOMPARE(describeDue(Due::on(d(2026, 8, 3)).repeating(Recurrence::every(1, Unit::Week)), today), "Mon · every week");
        QCOMPARE(describeDue(Due::on(d(2026, 8, 3)).repeating(Recurrence::everyWeekday()), today), "Mon · every weekday");
    }
    void durationsAndCounts() {
        QCOMPARE(duration(30), "30 minutes");
        QCOMPARE(duration(60), "1 hour");
        QCOMPARE(duration(2880), "2 days");
        QCOMPARE(countOf(1), "1 task");
        QCOMPARE(countOf(6), "6 tasks");
        QCOMPARE(capitalise("every day"), "Every day");
    }
    void quickAddChipsInParserOrder() {
        const QDate today = d(2026, 9, 5);   // Saturday
        const QuickAdd parsed = parseQuickAdd("Email Sam about the lease #Work @email p1 friday 9am !30m", today, Vocabulary());
        const QList<Chip> chips = describeQuickAdd(parsed, today, "Inbox");
        QStringList texts;
        for (const Chip &chip : chips) texts << chip.icon + ":" + chip.text;
        QCOMPARE(texts, (QStringList{"folder:Work", "x-office-calendar:Fri 09:00", "emblem-important:Urgent", "user-bookmarks:email", "alarm:30 minutes before"}));
        QCOMPARE(chips[2].role, "negative");
        QCOMPARE(describeQuickAdd(parseQuickAdd("Buy milk", today, Vocabulary()), today, "Inbox")[0].text, "Inbox");
    }
    void viewsAreQueries() {
        const QList<View> views = builtinViews();
        QCOMPARE(views.size(), 5);
        QCOMPARE(views[1].query, "due: today | overdue");
        QCOMPARE(views[3].emptyTitle, "Nothing pinned");
        QTemporaryDir dir;
        Store store = Store::openAt(dir.path() + "/planner.json");
        const ProjectId work = store.addProject(Project::create("R&D", Color::Blue), now());
        Project child = Project::create("Admin", Color::Teal);
        child.parentId = work;
        store.addProject(child, now());
        store.addProject(Project::create("Home", Color::Green), now());
        const QList<View> projects = projectViews(store);
        QCOMPARE(projects.size(), 3);
        QCOMPARE(projects[0].query, "#R\\&D");
        QCOMPARE(projects[1].depth, 1);
        QCOMPARE(projects[1].title, "Admin");
        QCOMPARE(projects[2].title, "Home");
        QCOMPARE(projects[0].projectId(), work);
        store.putFilter(SavedFilter::create("Errands", "@errand", Color::Pink), now());
        QCOMPARE(filterViews(store).size(), 1);
        QVERIFY(filterViews(store)[0].filterId().has_value());
    }
};

QTEST_APPLESS_MAIN(TestPresent)
#include "test_present.moc"
