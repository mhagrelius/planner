#include "order.h"

namespace planner::order {

static const char *const kDigits = "0123456789abcdefghijklmnopqrstuvwxyz";
static constexpr int kBase = 36;

static int indexOf(QChar digit) {
    for (int i = 0; i < kBase; ++i)
        if (QLatin1Char(kDigits[i]) == digit) return i;
    // A key not in the alphabet can only come from a hand-edited file; the
    // lowest digit sorts it to the top, which is visible and harmless.
    return 0;
}

// A key strictly between `a` and `b`, where a < b and `b` may be empty (open).
// Terminates because no generated key ends in the lowest digit.
static QString midpoint(const QString &a, const QString &bIn) {
    // An exhausted upper bound has no room below it; treat it as open.
    const QString b = bIn;
    if (!b.isEmpty()) {
        // A shared prefix contributes nothing; solve the smaller problem.
        int shared = 0;
        while (shared < b.size()) {
            const QChar fromA = shared < a.size() ? a.at(shared) : QLatin1Char(kDigits[0]);
            if (fromA != b.at(shared)) break;
            ++shared;
        }
        if (shared > 0) return b.left(shared) + midpoint(a.mid(shared), b.mid(shared));
    }
    const int low = a.isEmpty() ? 0 : indexOf(a.at(0));
    const int high = b.isEmpty() ? kBase : indexOf(b.at(0));
    if (high - low > 1) return QString(QLatin1Char(kDigits[(low + high) / 2]));
    // Adjacent digits: the key has to get longer.
    if (!b.isEmpty() && b.size() > 1) return b.left(1);
    return QString(QLatin1Char(kDigits[low])) + midpoint(a.mid(1), QString());
}

QString start() { return between(QString(), QString()); }

QString between(const QString &before, const QString &after) {
    // Neighbours arriving out of order means a caller sorted one way and read
    // another; appending after the larger keeps the list usable.
    if (!after.isEmpty() && compare(before, after) >= 0) return midpoint(compare(after, before) > 0 ? after : before, QString());
    return midpoint(before, after);
}

QString fromLegacyPosition(qint64 position) {
    constexpr int width = 6;
    const quint64 base = kBase - 1;
    quint64 ceiling = 1;
    for (int i = 0; i < width; ++i) ceiling *= base;
    ceiling -= 1;
    quint64 value = std::min<quint64>(static_cast<quint64>(std::max<qint64>(0, position)), ceiling);
    QString digits(width, QLatin1Char(kDigits[1]));
    for (int slot = width - 1; slot >= 0; --slot) {
        digits[slot] = QLatin1Char(kDigits[1 + static_cast<int>(value % base)]);
        value /= base;
    }
    return digits;
}

int compare(const QString &a, const QString &b) { return QString::compare(a, b); }

} // namespace planner::order
