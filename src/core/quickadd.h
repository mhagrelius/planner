// Quick add: one line of text becomes a task.
//
//   Email Sam about the lease #Work /Admin @email p2 friday 9am !30m
//
// The parse is reported as spans over the original string as well as a
// finished result, because the entry highlights each token as you type. The
// spans are UTF-16 offsets into the QString, which is what a text field
// wants back.
#pragma once

#include "model.h"

#include <QStringList>

namespace planner {

// The names already in the store, so multi-word ones can be recognised.
// `#My Big Project` is three words, and only the store knows that.
struct Vocabulary {
    QStringList projects;
    QStringList sections;
    QStringList labels;
};

enum class SpanKind { Project, Section, Label, Priority, Date, Recurrence, Reminder };

struct Span {
    int start = 0;
    int end = 0;
    SpanKind kind = SpanKind::Date;
};

struct QuickAdd {
    QString title;                 // what is left once every token is removed
    std::optional<QString> project;
    std::optional<QString> section;
    QStringList labels;
    std::optional<Priority> priority;
    std::optional<Due> due;
    QList<qint64> reminders;       // minutes before the due time, one per `!`
    QList<Span> spans;             // sorted by start
};

QuickAdd parseQuickAdd(const QString &text, const QDate &today, const Vocabulary &vocabulary);

} // namespace planner
