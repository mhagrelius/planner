#include <QtTest>

#include "quickadd.h"

using namespace planner;

static QDate today() { return QDate(2026, 7, 30); }
static QDate d(int y, int m, int day) { return QDate(y, m, day); }
static QuickAdd parse(const char *text) { return parseQuickAdd(QString::fromUtf8(text), today(), Vocabulary()); }

class TestQuickAdd : public QObject {
    Q_OBJECT
private slots:
    void aLineWithNoTokensIsAllTitle() {
        const QuickAdd parsed = parse("Email Sam about the lease");
        QCOMPARE(parsed.title, "Email Sam about the lease");
        QVERIFY(parsed.spans.isEmpty());
        QVERIFY(!parsed.due && !parsed.priority);
    }
    void everyTokenIsStrippedFromTheTitle() {
        const QuickAdd parsed = parse("Email Sam #Work /Admin @email p2 friday !30m");
        QCOMPARE(parsed.title, "Email Sam");
        QCOMPARE(parsed.project, "Work");
        QCOMPARE(parsed.section, "Admin");
        QCOMPARE(parsed.labels, QStringList{"email"});
        QCOMPARE(parsed.priority, Priority::P2);
        QCOMPARE(parsed.due->date, d(2026, 7, 31));
        QCOMPARE(parsed.reminders, QList<qint64>{30});
    }
    void tokensAreReportedAsSpansOverTheOriginalText() {
        const QString text = "Call Sam p1 tomorrow";
        const QuickAdd parsed = parseQuickAdd(text, today(), Vocabulary());
        QCOMPARE(parsed.spans.size(), 2);
        QCOMPARE(parsed.spans[0].kind, SpanKind::Priority);
        QCOMPARE(parsed.spans[1].kind, SpanKind::Date);
        QCOMPARE(text.mid(parsed.spans[0].start, parsed.spans[0].end - parsed.spans[0].start), "p1");
        QCOMPARE(text.mid(parsed.spans[1].start, parsed.spans[1].end - parsed.spans[1].start), "tomorrow");
        const QuickAdd unordered = parse("tomorrow Call Sam p1 @work");
        for (int i = 1; i < unordered.spans.size(); ++i) QVERIFY(unordered.spans[i - 1].start <= unordered.spans[i].start);
    }
    void multiWordNamesNeedTheVocabulary() {
        Vocabulary vocabulary;
        vocabulary.projects << "My Big Project";
        QuickAdd parsed = parseQuickAdd("Do the thing #My Big Project", today(), vocabulary);
        QCOMPARE(parsed.project, "My Big Project");
        QCOMPARE(parsed.title, "Do the thing");
        parsed = parse("Do the thing #My Big Project");
        QCOMPARE(parsed.project, "My");
        QCOMPARE(parsed.title, "Do the thing Big Project");
        Vocabulary two;
        two.projects << "Work" << "Work Admin";
        parsed = parseQuickAdd("File it #Work Admin", today(), two);
        QCOMPARE(parsed.project, "Work Admin");
        QCOMPARE(parsed.title, "File it");
    }
    void labelsCollectAndDeduplicate() {
        QCOMPARE(parse("Shop @errand @town @ERRAND").labels, (QStringList{"errand", "town"}));
        const QuickAdd bare = parse("Think about it @");
        QVERIFY(bare.labels.isEmpty());
        QCOMPARE(bare.title, "Think about it @");
    }
    void datesAndTimesCombine() {
        QuickAdd parsed = parse("Standup friday 9am");
        QCOMPARE(parsed.due->date, d(2026, 7, 31));
        QCOMPARE(parsed.due->time, QTime(9, 0));
        QCOMPARE(parsed.title, "Standup");
        parsed = parse("Standup 9am");
        QCOMPARE(parsed.due->date, today());
        QCOMPARE(parsed.due->time, QTime(9, 0));
        QCOMPARE(parse("Review next friday").due->date, d(2026, 8, 7));
        QCOMPARE(parse("Review friday").due->date, d(2026, 7, 31));
    }
    void bareNumbersAreNotDatesOrTimes() {
        QuickAdd parsed = parse("Buy 3 apples");
        QCOMPARE(parsed.title, "Buy 3 apples");
        QVERIFY(!parsed.due);
        parsed = parse("Read chapter 9");
        QCOMPARE(parsed.title, "Read chapter 9");
        QVERIFY(!parsed.due);
        parsed = parse("Rent 1st");
        QCOMPARE(parsed.due->date, d(2026, 8, 1));
        QCOMPARE(parsed.title, "Rent");
    }
    void repeats() {
        QuickAdd parsed = parse("Bins every monday");
        QCOMPARE(parsed.due->date, d(2026, 8, 3));
        QCOMPARE(parsed.due->recurrence, Recurrence::weeklyOn(1, {Weekday::Mon}));
        QCOMPARE(parsed.title, "Bins");
        parsed = parse("Bins every week 3 august");
        QCOMPARE(parsed.due->date, d(2026, 8, 3));
        QCOMPARE(parsed.due->recurrence->unit, Unit::Week);
        parsed = parse("Weekly review");
        QCOMPARE(parsed.title, "Weekly review");
        QVERIFY(!parsed.due);
        parsed = parse("Daily standup every weekday");
        QCOMPARE(parsed.title, "Daily standup");
        QCOMPARE(parsed.due->recurrence, Recurrence::everyWeekday());
        parsed = parse("Water the plants every! 10 days");
        QVERIFY(parsed.due->recurrence->fromCompletion);
        QCOMPARE(parsed.due->recurrence->interval, 10);
        QCOMPARE(parsed.title, "Water the plants");
    }
    void reminders() {
        QCOMPARE(parse("A !30m").reminders, QList<qint64>{30});
        QCOMPARE(parse("A !2h").reminders, QList<qint64>{120});
        QCOMPARE(parse("A !1d").reminders, QList<qint64>{1440});
        QCOMPARE(parse("A !45").reminders, QList<qint64>{45});
        QCOMPARE(parse("A !30m !2h").reminders, (QList<qint64>{30, 120}));
        const QuickAdd shout = parse("Shout !!! today");
        QCOMPARE(shout.title, "Shout !!!");
        QVERIFY(shout.reminders.isEmpty());
    }
    void priorityIsItsOwnWord() {
        QCOMPARE(parse("p1 Call Sam").priority, Priority::P1);
        QCOMPARE(parse("Call Sam p4").priority, Priority::P4);
        const QuickAdd parsed = parse("Review p1s report");
        QVERIFY(!parsed.priority);
        QCOMPARE(parsed.title, "Review p1s report");
    }
    void emptyAndTokenOnlyLines() {
        QuickAdd parsed = parse("   ");
        QCOMPARE(parsed.title, "");
        QVERIFY(!parsed.due && parsed.spans.isEmpty());
        parsed = parse("#Work p1 tomorrow");
        QCOMPARE(parsed.title, "");
        QCOMPARE(parsed.project, "Work");
        parsed = parse("Café review tomorrow p1");
        QCOMPARE(parsed.title, "Café review");
    }
};

QTEST_APPLESS_MAIN(TestQuickAdd)
#include "test_quickadd.moc"
