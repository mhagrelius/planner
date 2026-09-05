#include <QtTest>

#include "dates.h"

using namespace planner;

// Thursday, 30 July 2026.
static QDate today() { return QDate(2026, 7, 30); }
static std::optional<QDate> parse(const char *text) { return parseDate(QString::fromUtf8(text), today()); }
static QDate d(int y, int m, int day) { return QDate(y, m, day); }

class TestDates : public QObject {
    Q_OBJECT
private slots:
    void theObviousWords() {
        QCOMPARE(parse("today"), d(2026, 7, 30));
        QCOMPARE(parse("tod"), d(2026, 7, 30));
        QCOMPARE(parse("tomorrow"), d(2026, 7, 31));
        QCOMPARE(parse("tom"), d(2026, 7, 31));
        QCOMPARE(parse("yesterday"), d(2026, 7, 29));
    }
    void parsingIgnoresCaseAndTrailingPunctuation() {
        QCOMPARE(parse("Tomorrow,"), d(2026, 7, 31));
        QCOMPARE(parse("  FRIDAY  "), parse("friday"));
    }
    void aBareWeekdayMeansTheComingOne() {
        QCOMPARE(parse("friday"), d(2026, 7, 31));
        QCOMPARE(parse("fri"), d(2026, 7, 31));
        QCOMPARE(parse("monday"), d(2026, 8, 3));
        QCOMPARE(parse("wednesday"), d(2026, 8, 5));
        QCOMPARE(parse("thursday"), d(2026, 7, 30));
    }
    void nextWeekdayAlwaysSkipsAWeek() {
        QCOMPARE(parse("next friday"), d(2026, 8, 7));
        QCOMPARE(parse("next thursday"), d(2026, 8, 6));
    }
    void relativeOffsets() {
        QCOMPARE(parse("in 3 days"), d(2026, 8, 2));
        QCOMPARE(parse("in 2 weeks"), d(2026, 8, 13));
        QCOMPARE(parse("in 1 month"), d(2026, 8, 30));
        QCOMPARE(parse("in a month"), d(2026, 8, 30));
        QCOMPARE(parse("in 1 year"), d(2027, 7, 30));
        QCOMPARE(parse("next week"), d(2026, 8, 6));
        QCOMPARE(parse("next month"), d(2026, 8, 30));
        QCOMPARE(parse("next year"), d(2027, 7, 30));
    }
    void endOfPeriod() {
        QCOMPARE(parse("end of week"), d(2026, 8, 2));
        QCOMPARE(parse("end of month"), d(2026, 7, 31));
        QCOMPARE(parse("end of year"), d(2026, 12, 31));
        QCOMPARE(parseDate("end of month", d(2026, 2, 10)), d(2026, 2, 28));
        QCOMPARE(parseDate("end of month", d(2028, 2, 10)), d(2028, 2, 29));
    }
    void aBareDayNumberNeverLandsInThePast() {
        QCOMPARE(parse("27th"), d(2026, 8, 27));
        QCOMPARE(parse("31st"), d(2026, 7, 31));
        QCOMPARE(parse("30th"), d(2026, 7, 30));
        QCOMPARE(parseDate("31st", d(2026, 9, 15)), d(2026, 10, 31));
    }
    void dayAndMonthInEitherOrder() {
        QCOMPARE(parse("3 august"), d(2026, 8, 3));
        QCOMPARE(parse("august 3"), d(2026, 8, 3));
        QCOMPARE(parse("3rd aug"), d(2026, 8, 3));
        QCOMPARE(parse("aug 3rd"), d(2026, 8, 3));
        QCOMPARE(parse("3 january"), d(2027, 1, 3));
        QCOMPARE(parse("3 january 2026"), d(2026, 1, 3));
        QCOMPARE(parse("2026-01-03"), d(2026, 1, 3));
    }
    void ambiguousNumericDatesAreRefused() {
        QVERIFY(!parse("03/07/2026"));
        QVERIFY(!parse("3/7"));
    }
    void nonsenseIsNotADate() {
        QVERIFY(!parse(""));
        QVERIFY(!parse("lunch"));
        QVERIFY(!parse("in 3 fortnights"));
        QVERIFY(!parse("32nd"));
        QVERIFY(!parse("next lunchtime"));
    }
    void timesOfDay() {
        QCOMPARE(parseTime("9am"), QTime(9, 0));
        QCOMPARE(parseTime("9pm"), QTime(21, 0));
        QCOMPARE(parseTime("at 5pm"), QTime(17, 0));
        QCOMPARE(parseTime("9:30"), QTime(9, 30));
        QCOMPARE(parseTime("9.30pm"), QTime(21, 30));
        QCOMPARE(parseTime("17:00"), QTime(17, 0));
        QCOMPARE(parseTime("12am"), QTime(0, 0));
        QCOMPARE(parseTime("12pm"), QTime(12, 0));
        QCOMPARE(parseTime("noon"), QTime(12, 0));
        QCOMPARE(parseTime("midnight"), QTime(0, 0));
        QCOMPARE(parseTime("morning"), QTime(9, 0));
        QCOMPARE(parseTime("afternoon"), QTime(14, 0));
        QCOMPARE(parseTime("evening"), QTime(18, 0));
        QCOMPARE(parseTime("tonight"), QTime(20, 0));
    }
    void nonsenseIsNotATime() {
        QVERIFY(!parseTime("27"));
        QVERIFY(!parseTime("25:00"));
        QVERIFY(!parseTime("9:75"));
        QVERIFY(!parseTime("13pm"));
        QVERIFY(!parseTime("lunch"));
    }
};

QTEST_APPLESS_MAIN(TestDates)
#include "test_dates.moc"
