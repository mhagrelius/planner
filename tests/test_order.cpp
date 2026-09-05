#include <QtTest>

#include "order.h"

using namespace planner::order;

class TestOrder : public QObject {
    Q_OBJECT
private slots:
    void aFirstKeySitsInTheMiddleOfTheAlphabet() {
        QVERIFY(start() > "0");
        QVERIFY(start() < "z");
        QCOMPARE(start(), "i");
    }
    void aKeyBetweenTwoOthersSortsBetweenThem() {
        const QString first = start();
        const QString last = between(first, QString());
        const QString middle = between(first, last);
        QVERIFY(compare(first, middle) < 0);
        QVERIFY(compare(middle, last) < 0);
    }
    void appendingAndPrependingKeepGoing() {
        QStringList keys{start()};
        for (int i = 0; i < 200; ++i) {
            const QString next = between(keys.last(), QString());
            QVERIFY(compare(keys.last(), next) < 0);
            keys << next;
        }
        QStringList down{start()};
        for (int i = 0; i < 200; ++i) {
            const QString next = between(QString(), down.first());
            QVERIFY2(compare(next, down.first()) < 0, qPrintable(next + " < " + down.first()));
            QVERIFY(!next.endsWith('0'));
            down.prepend(next);
        }
    }
    void splittingTheSameGapTwoHundredTimesStillFits() {
        const QString low = start();
        QString high = between(low, QString());
        for (int i = 0; i < 200; ++i) {
            const QString next = between(low, high);
            QVERIFY2(compare(low, next) < 0, qPrintable(low + " < " + next));
            QVERIFY2(compare(next, high) < 0, qPrintable(next + " < " + high));
            high = next;
        }
    }
    void aV1PositionKeepsItsPlaceAndNewKeysLandAfterIt() {
        QStringList positions;
        for (int i = 0; i < 50; ++i) positions << fromLegacyPosition(i);
        QStringList sorted = positions;
        std::sort(sorted.begin(), sorted.end());
        QCOMPARE(positions, sorted);
        const QString lastOfTheOld = fromLegacyPosition(41);
        QVERIFY(compare(lastOfTheOld, between(lastOfTheOld, QString())) < 0);
        QVERIFY(compare(lastOfTheOld, start()) < 0);
        QVERIFY(!lastOfTheOld.endsWith('0'));
        QCOMPARE(fromLegacyPosition(0).size(), 6);
    }
};

QTEST_APPLESS_MAIN(TestOrder)
#include "test_order.moc"
