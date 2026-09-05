#include <QtTest>

#include "dates.h"

using namespace planner;

static QDate today() { return QDate(2026, 7, 30); }
static QDate d(int y, int m, int day) { return QDate(y, m, day); }
static std::optional<Recurrence> parse(const char *text) { return parseRecurrence(QString::fromUtf8(text), today()); }

class TestRecurrence : public QObject {
    Q_OBJECT
private slots:
    void aDailyRuleStepsByItsInterval() {
        QCOMPARE(Recurrence::every(3, Unit::Day).nextAfter(d(2026, 7, 30)), d(2026, 8, 2));
        Recurrence zero = Recurrence::every(1, Unit::Day);
        zero.interval = 0;
        QCOMPARE(zero.nextAfter(d(2026, 7, 30)), d(2026, 7, 31));
    }
    void weeklyRules() {
        const auto next = Recurrence::every(2, Unit::Week).nextAfter(d(2026, 7, 30));
        QCOMPARE(next, d(2026, 8, 13));
        QCOMPARE(weekdayOf(*next), Weekday::Thu);
        const Recurrence monFri = Recurrence::weeklyOn(1, {Weekday::Mon, Weekday::Fri});
        QCOMPARE(monFri.nextAfter(d(2026, 7, 27)), d(2026, 7, 31));
        QCOMPARE(monFri.nextAfter(d(2026, 7, 31)), d(2026, 8, 3));
        const Recurrence other = Recurrence::weeklyOn(2, {Weekday::Mon, Weekday::Fri});
        const QDate first = *other.nextAfter(d(2026, 7, 27));
        QCOMPARE(first, d(2026, 7, 31));
        QCOMPARE(other.nextAfter(first), d(2026, 8, 10));
        QCOMPARE(Recurrence::everyWeekday().nextAfter(d(2026, 7, 31)), d(2026, 8, 3));
        QCOMPARE(Recurrence::everyWeekday().nextAfter(d(2026, 8, 3)), d(2026, 8, 4));
        QCOMPARE(Recurrence::weeklyOn(1, {Weekday::Fri, Weekday::Mon, Weekday::Fri}).weekdays, (QList<Weekday>{Weekday::Mon, Weekday::Fri}));
    }
    void monthsAndYearsClamp() {
        QCOMPARE(Recurrence::every(1, Unit::Month).nextAfter(d(2026, 1, 31)), d(2026, 2, 28));
        QCOMPARE(Recurrence::every(1, Unit::Month).nextAfter(d(2026, 3, 31)), d(2026, 4, 30));
        QCOMPARE(Recurrence::every(2, Unit::Month).nextAfter(d(2026, 1, 31)), d(2026, 3, 31));
        QCOMPARE(Recurrence::every(1, Unit::Year).nextAfter(d(2028, 2, 29)), d(2029, 2, 28));
    }
    void everyVersusEveryBang() {
        QCOMPARE(Recurrence::every(1, Unit::Week).advance(d(2026, 7, 6), d(2026, 7, 27))->first, d(2026, 7, 13));
        Recurrence bang = Recurrence::every(10, Unit::Day);
        bang.fromCompletion = true;
        QCOMPARE(bang.advance(d(2026, 7, 6), d(2026, 7, 27))->first, d(2026, 8, 6));
        QCOMPARE(bang.advance(d(2026, 7, 30), d(2026, 7, 23))->first, d(2026, 8, 9));
    }
    void endConditions() {
        Recurrence until = Recurrence::every(1, Unit::Week);
        until.end = End::onDate(d(2026, 8, 5));
        QVERIFY(!until.advance(d(2026, 7, 30), d(2026, 7, 30)));
        until.end = End::onDate(d(2026, 8, 6));
        QCOMPARE(until.advance(d(2026, 7, 30), d(2026, 7, 30))->first, d(2026, 8, 6));
        Recurrence spent = Recurrence::every(1, Unit::Day);
        spent.end = End::after(0);
        QVERIFY(!spent.advance(d(2026, 7, 30), d(2026, 7, 30)));
        Recurrence counted = Recurrence::every(1, Unit::Day);
        counted.end = End::after(3);
        QDate due = d(2026, 7, 30);
        QList<QDate> produced;
        while (auto next = counted.advance(due, due)) {
            produced.append(next->first);
            due = next->first;
            counted = next->second;
        }
        QCOMPARE(produced, (QList<QDate>{d(2026, 7, 31), d(2026, 8, 1), d(2026, 8, 2)}));
    }
    void aRuleReadsAsThePhraseThatMadeIt() {
        QCOMPARE(Recurrence::every(1, Unit::Day).describe(), "every day");
        QCOMPARE(Recurrence::every(2, Unit::Day).describe(), "every other day");
        QCOMPARE(Recurrence::every(3, Unit::Day).describe(), "every 3 days");
        QCOMPARE(Recurrence::every(1, Unit::Month).describe(), "every month");
        QCOMPARE(Recurrence::everyWeekday().describe(), "every weekday");
        QCOMPARE(Recurrence::weeklyOn(1, {Weekday::Mon}).describe(), "every monday");
        QCOMPARE(Recurrence::weeklyOn(2, {Weekday::Mon, Weekday::Fri}).describe(), "every other monday and friday");
        QCOMPARE(Recurrence::weeklyOn(1, {Weekday::Mon, Weekday::Wed, Weekday::Fri}).describe(), "every monday, wednesday and friday");
        Recurrence bang = Recurrence::every(10, Unit::Day);
        bang.fromCompletion = true;
        QCOMPARE(bang.describe(), "every! 10 days");
        Recurrence until = Recurrence::every(1, Unit::Week);
        until.end = End::onDate(d(2026, 9, 1));
        QCOMPARE(until.describe(), "every week until 1 Sep 2026");
        Recurrence counted = Recurrence::every(1, Unit::Day);
        counted.end = End::after(2);
        QCOMPARE(counted.describe(), "every day x3");
        counted.end = End::after(0);
        QCOMPARE(counted.describe(), "every day x1");
    }
    void everyRuleSurvivesARoundTripThroughItsOwnDescription() {
        Recurrence bang = Recurrence::every(10, Unit::Day);
        bang.fromCompletion = true;
        Recurrence until = Recurrence::every(1, Unit::Week);
        until.end = End::onDate(d(2027, 3, 4));
        Recurrence counted = Recurrence::everyWeekday();
        counted.end = End::after(4);
        Recurrence both = Recurrence::weeklyOn(2, {Weekday::Tue});
        both.fromCompletion = true;
        both.end = End::after(1);
        for (const Recurrence &rule : {Recurrence::every(1, Unit::Day), Recurrence::every(2, Unit::Week), Recurrence::every(3, Unit::Month),
                                       Recurrence::every(5, Unit::Year), Recurrence::everyWeekday(), Recurrence::weeklyOn(1, {Weekday::Sat, Weekday::Sun}),
                                       Recurrence::weeklyOn(2, {Weekday::Mon, Weekday::Wed, Weekday::Fri}), bang, until, counted, both}) {
            const QString phrase = rule.describe();
            const auto parsed = parseRecurrence(phrase, d(2026, 7, 30));
            QVERIFY2(parsed && *parsed == rule, qPrintable(phrase + " did not round-trip"));
        }
    }
    void jsonRoundTrip() {
        QCOMPARE(QString::fromUtf8(QJsonDocument(Recurrence::every(1, Unit::Day).toJson()).toJson(QJsonDocument::Compact)), R"({"interval":1,"unit":"day"})");
        Recurrence rule = Recurrence::weeklyOn(2, {Weekday::Mon, Weekday::Thu});
        rule.fromCompletion = true;
        rule.end = End::after(5);
        QVERIFY(Recurrence::fromJson(rule.toJson()) == rule);
        QCOMPARE(rule.toJson().value("weekdays").toArray().at(0).toString(), "Mon");
    }
    void phrases() {
        QCOMPARE(parse("every day"), Recurrence::every(1, Unit::Day));
        QCOMPARE(parse("every 3 days"), Recurrence::every(3, Unit::Day));
        QCOMPARE(parse("every week"), Recurrence::every(1, Unit::Week));
        QCOMPARE(parse("every month"), Recurrence::every(1, Unit::Month));
        QCOMPARE(parse("every year"), Recurrence::every(1, Unit::Year));
        QCOMPARE(parse("every other day"), Recurrence::every(2, Unit::Day));
        QCOMPARE(parse("daily"), parse("every day"));
        QCOMPARE(parse("weekly"), parse("every week"));
        QCOMPARE(parse("monthly"), parse("every month"));
        QCOMPARE(parse("yearly"), parse("every year"));
        QCOMPARE(parse("every monday"), Recurrence::weeklyOn(1, {Weekday::Mon}));
        QCOMPARE(parse("every other tuesday"), Recurrence::weeklyOn(2, {Weekday::Tue}));
        const auto expected = Recurrence::weeklyOn(1, {Weekday::Mon, Weekday::Fri});
        QCOMPARE(parse("every mon, fri"), expected);
        QCOMPARE(parse("every mon and fri"), expected);
        QCOMPARE(parse("every monday friday"), expected);
        QCOMPARE(parse("every weekday"), Recurrence::everyWeekday());
        QCOMPARE(parse("every weekdays"), Recurrence::everyWeekday());
        const auto bang = parse("every! 10 days");
        QVERIFY(bang && bang->fromCompletion && bang->interval == 10 && bang->unit == Unit::Day);
        QVERIFY(!parse("every 10 days")->fromCompletion);
        QCOMPARE(parse("every week until 1 september")->end, End::onDate(d(2026, 9, 1)));
        QCOMPARE(parse("every day x3")->end, End::after(2));
        QCOMPARE(parse("every day for 3 times")->end, End::after(2));
        QCOMPARE(parse("every monday")->firstOccurrence(today()), d(2026, 8, 3));
        QCOMPARE(parse("every thursday")->firstOccurrence(today()), today());
        QCOMPARE(parse("every week")->firstOccurrence(today()), today());
    }
    void nonsenseIsNotARule() {
        for (const char *text : {"", "every", "every fortnight", "evening", "every day and a half", "every day until lunchtime", "every day x0"})
            QVERIFY2(!parse(text), text);
    }
};

QTEST_APPLESS_MAIN(TestRecurrence)
#include "test_recurrence.moc"
