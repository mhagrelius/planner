// Turning model values into the words a row shows, and the views that always
// exist. Toolkit-free so "does 3 August render as `3 Aug`?" is a headless
// test. Nothing here reads the clock.
#pragma once

#include "model.h"
#include "quickadd.h"

namespace planner {

class Store;

// `Today`, `Tomorrow`, `Yesterday`, a weekday name 2–6 days out, `3 Aug`,
// `3 Aug 2027` in another year.
QString formatDate(const QDate &date, const QDate &today);
// The row's due string plus its class: `overdue`, `today` or empty.
std::pair<QString, QString> formatDue(const QDate &date, const std::optional<QTime> &time, const QDate &today);
// `Today at 09:00`, `Mon · every weekday`; `No date` for nothing.
QString describeDue(const std::optional<Due> &due, const QDate &today);
QString describeDeadline(const std::optional<QDate> &deadline, const QDate &today);
QString capitalise(const QString &text);
// `30 minutes`, `2 hours`, `1 day`.
QString duration(qint64 minutes);
// `1 task`, `6 tasks`.
QString countOf(int count);

// The reminder line under the quick-add entry.
extern const char *const kQuickAddHint;

struct Chip {
    QString icon;   // bare theme name: folder, x-office-calendar, …
    QString text;
    QString role;   // palette role for the text, empty for the default
};
// The chips under a quick-add entry, in parser order. The project chip shows
// even when it was not typed, because "where is this going?" is the question
// quick-add is worst at answering.
QList<Chip> describeQuickAdd(const QuickAdd &parsed, const QDate &today, const QString &defaultProject);

// One thing in the rail. Every entry is a query, including the built-in ones.
struct View {
    QString id;        // inbox, today, …, filter:<id>, project:<id>
    QString title;
    QString icon;
    QString query;
    std::optional<Color> color;
    int depth = 0;
    QString emptyTitle;
    QString emptyDescription;
    std::optional<ProjectId> projectId() const;
    std::optional<FilterId> filterId() const;
};
QList<View> builtinViews();
QList<View> filterViews(const Store &store);
QList<View> projectViews(const Store &store);
// Escape a name so it survives the query tokenizer: `#R\&D`.
QString escapeQueryName(const QString &name);

} // namespace planner
