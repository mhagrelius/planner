#include "dates.h"

#include <QRegularExpression>

namespace planner {

QStringList normaliseWords(const QString &text) {
    QStringList words;
    for (const QString &raw : text.split(QRegularExpression(QStringLiteral("\\s+")), Qt::SkipEmptyParts)) {
        QString word = raw;
        while (!word.isEmpty() && (word.front() == u',' || word.front() == u'.' || word.front() == u';')) word.remove(0, 1);
        while (!word.isEmpty() && (word.back() == u',' || word.back() == u'.' || word.back() == u';')) word.chop(1);
        word = word.toLower();
        if (!word.isEmpty()) words << word;
    }
    return words;
}

std::optional<Weekday> weekdayFromName(const QString &word) {
    if (word == u"monday" || word == u"mon") return Weekday::Mon;
    if (word == u"tuesday" || word == u"tue" || word == u"tues") return Weekday::Tue;
    if (word == u"wednesday" || word == u"wed") return Weekday::Wed;
    if (word == u"thursday" || word == u"thu" || word == u"thur" || word == u"thurs") return Weekday::Thu;
    if (word == u"friday" || word == u"fri") return Weekday::Fri;
    if (word == u"saturday" || word == u"sat") return Weekday::Sat;
    if (word == u"sunday" || word == u"sun") return Weekday::Sun;
    return std::nullopt;
}

QString singular(const QString &word) {
    return word.endsWith(u's') ? word.chopped(1) : word;
}

static std::optional<QDate> valid(const QDate &date) {
    if (!date.isValid()) return std::nullopt;
    return date;
}

// The next `weekday` on or after `today`, or a week later again.
static std::optional<QDate> nextWeekday(const QDate &today, Weekday weekday, bool skipAWeek) {
    const int current = daysFromMonday(weekdayOf(today));
    const int target = daysFromMonday(weekday);
    int ahead = ((target - current) % 7 + 7) % 7;
    // "next friday" on a Thursday is eight days off, not tomorrow; on a
    // Friday it is seven, not today.
    if (skipAWeek) ahead += 7;
    return valid(today.addDays(ahead));
}

// `friday` -> (Fri, false); `next friday` -> (Fri, true).
static std::optional<std::pair<Weekday, bool>> weekdayFromWords(const QStringList &words) {
    if (words.size() == 1) {
        if (const auto day = weekdayFromName(words[0])) return std::make_pair(*day, false);
    } else if (words.size() == 2 && words[0] == u"next") {
        if (const auto day = weekdayFromName(words[1])) return std::make_pair(*day, true);
    } else if (words.size() == 3 && words[0] == u"this" && words[1] == u"coming") {
        if (const auto day = weekdayFromName(words[2])) return std::make_pair(*day, false);
    }
    return std::nullopt;
}

static std::optional<int> positiveInt(const QString &text) {
    bool ok = false;
    const int value = text.toInt(&ok);
    if (!ok || value < 0) return std::nullopt;
    return value;
}

// "in 3 days", "in 2 weeks", "in a month".
static std::optional<QDate> relative(const QStringList &words, const QDate &today) {
    if (words.size() != 3 || words[0] != u"in") return std::nullopt;
    int count = 1;
    if (words[1] != u"a" && words[1] != u"an") {
        const auto parsed = positiveInt(words[1]);
        if (!parsed) return std::nullopt;
        count = *parsed;
    }
    const QString unit = singular(words[2]);
    if (unit == u"day") return valid(today.addDays(count));
    if (unit == u"week") return valid(today.addDays(qint64(count) * 7));
    if (unit == u"month") return valid(today.addMonths(count));
    if (unit == u"year") return valid(today.addMonths(count * 12));
    return std::nullopt;
}

// `27`, `27th`, `3rd`, `1st`, `2nd`.
static std::optional<int> dayNumber(const QString &word) {
    QString digits = word;
    for (const char *suffix : {"st", "nd", "rd", "th"}) {
        if (digits.endsWith(QLatin1String(suffix))) {
            digits.chop(2);
            break;
        }
    }
    const auto day = positiveInt(digits);
    if (!day || *day < 1 || *day > 31) return std::nullopt;
    return day;
}

static std::optional<int> monthFromName(const QString &word) {
    static const struct { const char *name; int month; } table[] = {
        {"january", 1}, {"jan", 1}, {"february", 2}, {"feb", 2}, {"march", 3}, {"mar", 3}, {"april", 4}, {"apr", 4},
        {"may", 5}, {"june", 6}, {"jun", 6}, {"july", 7}, {"jul", 7}, {"august", 8}, {"aug", 8}, {"september", 9},
        {"sep", 9}, {"sept", 9}, {"october", 10}, {"oct", 10}, {"november", 11}, {"nov", 11}, {"december", 12}, {"dec", 12}};
    for (const auto &entry : table)
        if (word == QLatin1String(entry.name)) return entry.month;
    return std::nullopt;
}

// "27th", "3 july", "july 3", "27 jul 2027". A bare day number means the next
// time that day comes round.
static std::optional<QDate> dayAndMonth(const QStringList &words, const QDate &today) {
    int day = 0;
    std::optional<int> month;
    std::optional<int> year;

    if (words.size() == 1) {
        const auto d = dayNumber(words[0]);
        if (!d) return std::nullopt;
        day = *d;
    } else if (words.size() == 2 || words.size() == 3) {
        if (words.size() == 3) {
            const auto y = positiveInt(words[2]);
            if (!y) return std::nullopt;
            year = *y;
        }
        const auto d = dayNumber(words[0]);
        const auto m = monthFromName(words[1]);
        if (d && m) {
            day = *d;
            month = *m;
        } else {
            const auto m2 = monthFromName(words[0]);
            const auto d2 = dayNumber(words[1]);
            if (!m2 || !d2) return std::nullopt;
            day = *d2;
            month = *m2;
        }
    } else {
        return std::nullopt;
    }

    if (month && year) return valid(QDate(*year, *month, day));
    if (month) {
        const QDate candidate(today.year(), *month, day);
        if (!candidate.isValid()) return std::nullopt;
        if (candidate < today) return valid(QDate(today.year() + 1, *month, day));
        return candidate;
    }
    const QDate candidate(today.year(), today.month(), day);
    if (candidate.isValid() && candidate >= today) return candidate;
    // Either the day has passed this month or this month is too short for
    // it. Both mean "next month".
    const QDate next = today.addMonths(1);
    return valid(QDate(next.year(), next.month(), day));
}

static std::optional<QDate> endOfWeek(const QDate &today) {
    return valid(today.addDays(6 - daysFromMonday(weekdayOf(today))));
}

static std::optional<QDate> endOfMonth(const QDate &today) {
    return valid(QDate(today.year(), today.month(), today.daysInMonth()));
}

std::optional<QDate> parseDate(const QString &text, const QDate &today) {
    const QStringList words = normaliseWords(text);
    if (words.isEmpty()) return std::nullopt;
    const QString joined = words.join(u' ');

    if (joined == u"today" || joined == u"tod") return today;
    if (joined == u"tomorrow" || joined == u"tom" || joined == u"tmr") return valid(today.addDays(1));
    if (joined == u"yesterday") return valid(today.addDays(-1));
    if (joined == u"next week") return valid(today.addDays(7));
    if (joined == u"next month") return valid(today.addMonths(1));
    if (joined == u"next year") return valid(today.addMonths(12));
    if (joined == u"end of week") return endOfWeek(today);
    if (joined == u"end of month") return endOfMonth(today);
    if (joined == u"end of year") return valid(QDate(today.year(), 12, 31));

    // ISO, the one unambiguous numeric form.
    if (joined.size() == 10 && joined[4] == u'-' && joined[7] == u'-') {
        const QDate iso = QDate::fromString(joined, Qt::ISODate);
        if (iso.isValid()) return iso;
    }

    if (const auto weekday = weekdayFromWords(words)) return nextWeekday(today, weekday->first, weekday->second);
    if (const auto date = relative(words, today)) return date;
    return dayAndMonth(words, today);
}

// `9`, `9am`, `9pm`, `9:30`, `9.30pm`, `21:00`.
static std::optional<QTime> clock(const QString &text) {
    QString digits = text;
    std::optional<bool> pm;
    if (digits.endsWith(u"am")) {
        digits.chop(2);
        pm = false;
    } else if (digits.endsWith(u"pm")) {
        digits.chop(2);
        pm = true;
    }
    digits = digits.trimmed();

    int hour = 0, minute = 0;
    int split = digits.indexOf(u':');
    if (split < 0) split = digits.indexOf(u'.');
    if (split >= 0) {
        const auto h = positiveInt(digits.left(split));
        const auto m = positiveInt(digits.mid(split + 1));
        if (!h || !m) return std::nullopt;
        hour = *h;
        minute = *m;
    } else {
        const auto h = positiveInt(digits);
        if (!h) return std::nullopt;
        hour = *h;
    }
    if (minute > 59) return std::nullopt;

    if (pm.has_value()) {
        // 12am is midnight and 12pm is noon; every other hour just shifts.
        if (*pm) {
            if (hour == 12) hour = 12;
            else if (hour < 12) hour += 12;
            else return std::nullopt;
        } else {
            if (hour == 12) hour = 0;
            else if (hour >= 12) return std::nullopt;
        }
    } else if (hour >= 24) {
        // `27` is a day of the month, not 27 o'clock.
        return std::nullopt;
    }
    const QTime time(hour, minute);
    if (!time.isValid()) return std::nullopt;
    return time;
}

std::optional<QTime> parseTime(const QString &text) {
    const QStringList words = normaliseWords(text);
    if (words.isEmpty()) return std::nullopt;
    const QString joined = words.join(u' ');

    if (joined == u"noon" || joined == u"midday") return QTime(12, 0);
    if (joined == u"midnight") return QTime(0, 0);
    if (joined == u"morning" || joined == u"in the morning") return QTime(9, 0);
    if (joined == u"afternoon" || joined == u"in the afternoon") return QTime(14, 0);
    if (joined == u"evening" || joined == u"in the evening") return QTime(18, 0);
    if (joined == u"night" || joined == u"tonight" || joined == u"at night") return QTime(20, 0);

    if (words.size() == 2 && (words[0] == u"at" || words[0] == u"@")) return clock(words[1]);
    if (words.size() == 1) return clock(words[0]);
    return std::nullopt;
}

// --- recurrence phrases ------------------------------------------------------

// Apply a trailing end clause, if there is one. Anything left over that is
// not an end clause means the phrase was not a repeat rule after all.
static std::optional<Recurrence> finished(Recurrence rule, const QStringList &rest, const QDate &today) {
    if (rest.isEmpty()) return rule;
    const QString first = rest[0];
    const QStringList tail = rest.mid(1);

    // "x3"
    if (first.startsWith(u'x')) {
        if (const auto count = positiveInt(first.mid(1))) {
            if (!tail.isEmpty() || *count == 0) return std::nullopt;
            rule.end = End::after(*count - 1);
            return rule;
        }
    }

    if (first == u"until" || first == u"till" || first == u"ending" || first == u"ends") {
        const auto date = parseDate(tail.join(u' '), today);
        if (!date) return std::nullopt;
        rule.end = End::onDate(*date);
        return rule;
    }
    if (first == u"for") {
        if (tail.size() == 2 && singular(tail[1]) == u"time") {
            const auto count = positiveInt(tail[0]);
            if (!count || *count == 0) return std::nullopt;
            rule.end = End::after(*count - 1);
            return rule;
        }
        return std::nullopt;
    }
    return std::nullopt;
}

// `monday`, `mon and fri`, `mon, tue, fri`.
static std::optional<std::pair<Recurrence, QStringList>> weekdayList(int interval, const QStringList &words) {
    QList<Weekday> days;
    int index = 0;
    while (index < words.size()) {
        // "and" only joins; it never starts or ends a list.
        if (words[index] == u"and" && !days.isEmpty()) {
            ++index;
            continue;
        }
        const auto day = weekdayFromName(words[index]);
        if (!day) break;
        days.append(*day);
        ++index;
    }
    if (days.isEmpty()) return std::nullopt;
    return std::make_pair(Recurrence::weeklyOn(interval, days), words.mid(index));
}

// The unit or weekday list at the heart of the rule.
static std::optional<std::pair<Recurrence, QStringList>> body(int interval, const QStringList &words) {
    if (words.isEmpty()) return std::nullopt;
    const QString first = words[0];
    const QStringList rest = words.mid(1);

    if (singular(first) == u"weekday") {
        Recurrence rule = Recurrence::everyWeekday();
        rule.interval = interval;
        return std::make_pair(rule, rest);
    }
    if (weekdayFromName(first)) return weekdayList(interval, words);

    const QString unit = singular(first);
    if (unit == u"day") return std::make_pair(Recurrence::every(interval, Unit::Day), rest);
    if (unit == u"week") return std::make_pair(Recurrence::every(interval, Unit::Week), rest);
    if (unit == u"month") return std::make_pair(Recurrence::every(interval, Unit::Month), rest);
    if (unit == u"year") return std::make_pair(Recurrence::every(interval, Unit::Year), rest);
    return std::nullopt;
}

std::optional<Recurrence> parseRecurrence(const QString &text, const QDate &today) {
    const QStringList words = normaliseWords(text);
    if (words.isEmpty()) return std::nullopt;
    const QString lead = words[0];
    QStringList rest = words.mid(1);

    bool fromCompletion = false;
    if (lead == u"every") fromCompletion = false;
    else if (lead == u"every!") fromCompletion = true;
    // "daily", "weekly" and friends are the same rules said differently.
    else if (lead == u"daily") return finished(Recurrence::every(1, Unit::Day), rest, today);
    else if (lead == u"weekly") return finished(Recurrence::every(1, Unit::Week), rest, today);
    else if (lead == u"monthly") return finished(Recurrence::every(1, Unit::Month), rest, today);
    else if (lead == u"yearly" || lead == u"annually") return finished(Recurrence::every(1, Unit::Year), rest, today);
    else return std::nullopt;

    // `other` means two; a bare number means itself; anything else means one.
    int interval = 1;
    if (!rest.isEmpty()) {
        if (rest[0] == u"other") {
            interval = 2;
            rest.removeFirst();
        } else if (const auto count = positiveInt(rest[0]); count && *count >= 1) {
            interval = *count;
            rest.removeFirst();
        }
    }

    auto parsed = body(interval, rest);
    if (!parsed) return std::nullopt;
    parsed->first.fromCompletion = fromCompletion;
    return finished(parsed->first, parsed->second, today);
}

} // namespace planner
