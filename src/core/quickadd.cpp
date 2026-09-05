#include "quickadd.h"

#include "dates.h"

#include <algorithm>
#include <functional>

namespace planner {

namespace {

struct Word {
    QString text;
    QString lowered;
    int start = 0;
    int end = 0;
    bool taken = false;
};

QList<Word> split(const QString &text) {
    QList<Word> words;
    int start = -1;
    for (int i = 0; i < text.size(); ++i) {
        const bool space = text.at(i).isSpace();
        if (!space && start < 0) start = i;
        if (space && start >= 0) {
            words.append({text.mid(start, i - start), text.mid(start, i - start).toLower(), start, i, false});
            start = -1;
        }
    }
    if (start >= 0) words.append({text.mid(start), text.mid(start).toLower(), start, text.size(), false});
    return words;
}

// `!30m`, `!2h`, `!1d`, `!90`.
std::optional<qint64> reminder(const QString &token) {
    if (!token.startsWith(u'!')) return std::nullopt;
    QString body = token.mid(1);
    if (body.endsWith(u"before")) body.chop(6);
    body = body.trimmed();
    if (body.isEmpty()) return std::nullopt;
    qint64 multiplier = 1;
    const QChar last = body.back();
    if (last == u'm') { body.chop(1); multiplier = 1; }
    else if (last == u'h') { body.chop(1); multiplier = 60; }
    else if (last == u'd') { body.chop(1); multiplier = 60 * 24; }
    bool ok = false;
    const qint64 count = body.toLongLong(&ok);
    if (!ok || count <= 0) return std::nullopt;
    return count * multiplier;
}

// Read a `#`/`/`/`@` token, preferring the longest known name. Returns the
// name, the span, and how many words it used. A bare prefix is not a token.
struct Named { QString name; int start; int end; int consumed; };

std::optional<Named> named(const QList<Word> &words, int index, QChar prefix, const QStringList &known) {
    const QString first = words[index].text.mid(1);
    Q_UNUSED(prefix);
    std::optional<std::pair<QString, int>> best;
    for (int length = 1; index + length <= words.size(); ++length) {
        bool blocked = false;
        for (int i = index + 1; i < index + length; ++i)
            if (words[i].taken) blocked = true;
        if (blocked) break;
        QString candidate = first;
        for (int i = index + 1; i < index + length; ++i) {
            candidate += u' ';
            candidate += words[i].text;
        }
        for (const QString &name : known)
            if (name.compare(candidate, Qt::CaseInsensitive) == 0) best = std::make_pair(candidate, length);
    }
    if (best) return Named{best->first, words[index].start, words[index + best->second - 1].end, best->second};
    if (first.isEmpty()) return std::nullopt;
    return Named{first, words[index].start, words[index].end, 1};
}

void takePrefixed(QList<Word> &words, QuickAdd &result, const Vocabulary &vocabulary) {
    int index = 0;
    while (index < words.size()) {
        if (words[index].taken) { ++index; continue; }
        const QChar head = words[index].text.isEmpty() ? QChar() : words[index].text.front();
        SpanKind kind;
        const QStringList *known = nullptr;
        if (head == u'#') { kind = SpanKind::Project; known = &vocabulary.projects; }
        else if (head == u'/') { kind = SpanKind::Section; known = &vocabulary.sections; }
        else if (head == u'@') { kind = SpanKind::Label; known = &vocabulary.labels; }
        else if (head == u'!') {
            if (const auto minutes = reminder(words[index].lowered)) {
                result.reminders.append(*minutes);
                result.spans.append({words[index].start, words[index].end, SpanKind::Reminder});
                words[index].taken = true;
            }
            ++index;
            continue;
        } else {
            if (const auto priority = priorityFromToken(words[index].lowered)) {
                result.priority = *priority;
                result.spans.append({words[index].start, words[index].end, SpanKind::Priority});
                words[index].taken = true;
            }
            ++index;
            continue;
        }

        const auto found = named(words, index, head, *known);
        if (!found) { ++index; continue; }
        switch (kind) {
        case SpanKind::Project: result.project = found->name; break;
        case SpanKind::Section: result.section = found->name; break;
        case SpanKind::Label: {
            bool duplicate = false;
            for (const QString &label : result.labels)
                if (label.compare(found->name, Qt::CaseInsensitive) == 0) duplicate = true;
            if (!duplicate) result.labels.append(found->name);
            break;
        }
        default: break;
        }
        result.spans.append({found->start, found->end, kind});
        for (int i = index; i < index + found->consumed; ++i) words[i].taken = true;
        index += found->consumed;
    }
}

// Try phrases of decreasing length from `start`, returning the longest that
// parses. Longest-first is what makes `next friday` beat `friday`.
template <typename T>
std::optional<std::pair<T, int>> longest(const QList<Word> &words, int start, const std::function<std::optional<T>(const QString &)> &parse) {
    int limit = 0;
    while (start + limit < words.size() && !words[start + limit].taken && limit < 5) ++limit;
    for (int length = limit; length >= 1; --length) {
        QStringList parts;
        for (int i = start; i < start + length; ++i) parts << words[i].text;
        if (const auto value = parse(parts.join(u' '))) return std::make_pair(*value, length);
    }
    return std::nullopt;
}

void mark(QList<Word> &words, int start, int length, QuickAdd &result, SpanKind kind) {
    result.spans.append({words[start].start, words[start + length - 1].end, kind});
    for (int i = start; i < start + length; ++i) words[i].taken = true;
}

// Only `every` starts a repeat here: "Weekly review" is a title.
void takeRecurrence(QList<Word> &words, QuickAdd &result, const QDate &today) {
    for (int start = 0; start < words.size(); ++start) {
        if (words[start].taken || (words[start].lowered != u"every" && words[start].lowered != u"every!")) continue;
        const auto found = longest<Recurrence>(words, start, [&](const QString &phrase) { return parseRecurrence(phrase, today); });
        if (!found) continue;
        if (!result.due) result.due = Due::on(found->first.firstOccurrence(today));
        result.due->recurrence = found->first;
        mark(words, start, found->second, result, SpanKind::Recurrence);
        return;
    }
}

bool allDigits(const QString &text) {
    if (text.isEmpty()) return false;
    for (const QChar c : text)
        if (!c.isDigit()) return false;
    return true;
}

void takeDate(QList<Word> &words, QuickAdd &result, const QDate &today) {
    for (int start = 0; start < words.size(); ++start) {
        if (words[start].taken) continue;
        // "Buy 3 apples" is not due on the 3rd.
        const auto found = longest<QDate>(words, start, [&](const QString &phrase) -> std::optional<QDate> {
            if (allDigits(phrase)) return std::nullopt;
            return parseDate(phrase, today);
        });
        if (!found) continue;
        mark(words, start, found->second, result, SpanKind::Date);
        // A repeat phrase already set a first occurrence; an explicit date is
        // more specific, so it wins.
        if (result.due) result.due->date = found->first;
        else result.due = Due::on(found->first);
        break;
    }

    // A time is only meaningful once something is dated. `9am` on its own
    // means today at nine. "Chapter 9" is not nine o'clock.
    for (int start = 0; start < words.size(); ++start) {
        if (words[start].taken) continue;
        const auto found = longest<QTime>(words, start, [&](const QString &phrase) -> std::optional<QTime> {
            if (allDigits(phrase)) return std::nullopt;
            return parseTime(phrase);
        });
        if (!found) continue;
        mark(words, start, found->second, result, SpanKind::Date);
        if (!result.due) result.due = Due::on(today);
        result.due->time = found->first;
        return;
    }
}

} // namespace

QuickAdd parseQuickAdd(const QString &text, const QDate &today, const Vocabulary &vocabulary) {
    QList<Word> words = split(text);
    QuickAdd result;
    takePrefixed(words, result, vocabulary);
    takeRecurrence(words, result, today);
    takeDate(words, result, today);

    QStringList remaining;
    for (const Word &word : words)
        if (!word.taken) remaining << word.text;
    result.title = remaining.join(u' ');
    std::sort(result.spans.begin(), result.spans.end(), [](const Span &a, const Span &b) { return a.start < b.start; });
    return result;
}

} // namespace planner
