#include <QTemporaryDir>
#include <QtTest>

#include "schedule.h"
#include "store.h"

using namespace planner;

static QDate d(int y, int m, int day) { return QDate(y, m, day); }
static QDateTime at(int y, int m, int day, int h, int min) { return QDateTime(d(y, m, day), QTime(h, min), QTimeZone::utc()); }
static QTimeZone zone() { return QTimeZone::utc(); }

class TestSchedule : public QObject {
    Q_OBJECT
    QTemporaryDir dir;
    Store store() { return Store::openAt(dir.path() + "/planner.json"); }
    // A task due at 09:00 on 31 July 2026, with a reminder N minutes before.
    static TaskId withReminder(Store &store, qint64 minutes) {
        const TaskId id = store.addTask(Task::create(inboxId(), "Standup", at(2026, 7, 30, 12, 0)));
        store.taskMut(id)->due = Due::at(d(2026, 7, 31), QTime(9, 0));
        store.taskMut(id)->reminders = {Reminder::beforeDue(minutes)};
        return id;
    }
private slots:
    void aReminderFiresTheStatedTimeBeforeAndOnlyOnce() {
        Store s = store();
        const TaskId id = withReminder(s, 30);
        Schedule schedule;
        QVERIFY(schedule.takeDue(s, at(2026, 7, 31, 8, 29), zone()).isEmpty());
        const auto due = schedule.takeDue(s, at(2026, 7, 31, 8, 30), zone());
        QCOMPARE(due.size(), 1);
        QCOMPARE(due[0].task, id);
        QCOMPARE(due[0].title, "Standup");
        QCOMPARE(due[0].at, at(2026, 7, 31, 8, 30));
        QVERIFY(schedule.takeDue(s, at(2026, 7, 31, 9, 0), zone()).isEmpty());
        QVERIFY(schedule.takeDue(s, at(2026, 7, 31, 23, 0), zone()).isEmpty());
    }
    void catchingUpSilencesOnlyWhatHasPassed() {
        Store s = store();
        withReminder(s, 30);
        Schedule late;
        late.catchUp(s, at(2026, 8, 1, 10, 0), zone());
        QVERIFY(late.takeDue(s, at(2026, 8, 1, 10, 0), zone()).isEmpty());
        Schedule early;
        early.catchUp(s, at(2026, 7, 31, 8, 0), zone());
        QCOMPARE(early.takeDue(s, at(2026, 7, 31, 8, 30), zone()).size(), 1);
    }
    void completedAndUntimedTasksDoNotRemind() {
        Store s = store();
        const TaskId id = withReminder(s, 30);
        s.completeTask(id, at(2026, 7, 31, 8, 0), d(2026, 7, 31));
        Schedule schedule;
        QVERIFY(schedule.takeDue(s, at(2026, 7, 31, 9, 0), zone()).isEmpty());
        Store t = store();
        const TaskId untimed = withReminder(t, 30);
        t.taskMut(untimed)->due = Due::on(d(2026, 7, 31));
        QVERIFY(Schedule().takeDue(t, at(2026, 8, 1, 0, 0), zone()).isEmpty());
    }
    void absoluteRemindersAndTheNextOne() {
        Store s = store();
        withReminder(s, 30);
        const TaskId id = s.addTask(Task::create(inboxId(), "Later", at(2026, 7, 30, 12, 0)));
        s.taskMut(id)->reminders = {Reminder::absolute(at(2026, 7, 31, 18, 0))};
        Schedule schedule;
        QCOMPARE(schedule.nextAfter(s, at(2026, 7, 31, 0, 0), zone()), at(2026, 7, 31, 8, 30));
        QCOMPARE(schedule.nextAfter(s, at(2026, 7, 31, 9, 0), zone()), at(2026, 7, 31, 18, 0));
        QVERIFY(!schedule.nextAfter(s, at(2026, 8, 2, 0, 0), zone()));
        QVERIFY(schedule.takeDue(s, at(2026, 7, 31, 17, 59), zone()).size() == 1);
        QCOMPARE(schedule.takeDue(s, at(2026, 7, 31, 18, 0), zone()).size(), 1);
    }
    void aRecurringTaskRemindsAgainAfterForget() {
        Store s = store();
        const TaskId id = withReminder(s, 30);
        s.taskMut(id)->due = Due::at(d(2026, 7, 31), QTime(9, 0)).repeating(Recurrence::every(1, Unit::Day));
        Schedule schedule;
        QCOMPARE(schedule.takeDue(s, at(2026, 7, 31, 8, 30), zone()).size(), 1);
        s.completeTask(id, at(2026, 7, 31, 9, 0), d(2026, 7, 31));
        schedule.forget(id);
        QCOMPARE(schedule.takeDue(s, at(2026, 8, 1, 8, 30), zone()).size(), 1);
    }
    void resolvedInTheLocalZoneOldestFirst() {
        Store s = store();
        const TaskId id = withReminder(s, 0);
        const QTimeZone ahead = QTimeZone::fromSecondsAheadOfUtc(2 * 3600);
        Schedule schedule;
        QVERIFY(schedule.takeDue(s, at(2026, 7, 31, 6, 59), ahead).isEmpty());
        QCOMPARE(schedule.takeDue(s, at(2026, 7, 31, 7, 0), ahead).size(), 1);
        s.taskMut(id)->reminders = {Reminder::beforeDue(30), Reminder::beforeDue(60)};
        Schedule fresh;
        const auto due = fresh.takeDue(s, at(2026, 7, 31, 9, 0), zone());
        QCOMPARE(due.size(), 2);
        QCOMPARE(due[0].at, at(2026, 7, 31, 8, 0));
        QCOMPARE(due[1].at, at(2026, 7, 31, 8, 30));
    }
};

QTEST_APPLESS_MAIN(TestSchedule)
#include "test_schedule.moc"
