#include "present.h"

#include "store.h"

#include <QLocale>

namespace planner {

QString formatDate(const QDate &date, const QDate &today) {
    const qint64 days = today.daysTo(date);
    if (days == 0) return QStringLiteral("Today");
    if (days == 1) return QStringLiteral("Tomorrow");
    if (days == -1) return QStringLiteral("Yesterday");
    const QLocale c = QLocale::c();
    // Within the coming week a weekday name is unambiguous and shorter.
    if (days >= 2 && days <= 6) return c.dayName(date.dayOfWeek(), QLocale::ShortFormat);
    if (date.year() == today.year()) return QStringLiteral("%1 %2").arg(date.day()).arg(c.monthName(date.month(), QLocale::ShortFormat));
    return QStringLiteral("%1 %2 %3").arg(date.day()).arg(c.monthName(date.month(), QLocale::ShortFormat)).arg(date.year());
}

std::pair<QString, QString> formatDue(const QDate &date, const std::optional<QTime> &time, const QDate &today) {
    QString cls;
    if (date < today) cls = QStringLiteral("overdue");
    else if (date == today) cls = QStringLiteral("today");
    QString label = formatDate(date, today);
    if (time) label += QLatin1Char(' ') + time->toString(QStringLiteral("HH:mm"));
    return {label, cls};
}

QString describeDue(const std::optional<Due> &due, const QDate &today) {
    if (!due) return QStringLiteral("No date");
    QString text = formatDate(due->date, today);
    if (due->time) text += QStringLiteral(" at ") + due->time->toString(QStringLiteral("HH:mm"));
    // The rule itself, not the fact that there is one.
    if (due->recurrence) text += QStringLiteral(" · ") + due->recurrence->describe();
    return text;
}

QString describeDeadline(const std::optional<QDate> &deadline, const QDate &today) {
    return deadline ? formatDate(*deadline, today) : QStringLiteral("No deadline");
}

QString capitalise(const QString &text) {
    if (text.isEmpty()) return text;
    return text.left(1).toUpper() + text.mid(1);
}

QString duration(qint64 minutes) {
    auto plural = [](qint64 count, const char *unit) {
        return count == 1 ? QStringLiteral("1 %1").arg(QLatin1String(unit)) : QStringLiteral("%1 %2s").arg(count).arg(QLatin1String(unit));
    };
    if (minutes % (60 * 24) == 0) return plural(minutes / (60 * 24), "day");
    if (minutes % 60 == 0) return plural(minutes / 60, "hour");
    return plural(minutes, "minute");
}

QString countOf(int count) { return count == 1 ? QStringLiteral("1 task") : QStringLiteral("%1 tasks").arg(count); }

const char *const kQuickAddHint =
    "#project  /section  @label  p1–p4  !30m  — and dates like “friday 9am”, “in 3 days”, “every other monday”";

QList<Chip> describeQuickAdd(const QuickAdd &parsed, const QDate &today, const QString &defaultProject) {
    QList<Chip> chips;
    if (parsed.project) chips.append({QStringLiteral("folder"), *parsed.project, {}});
    else if (!defaultProject.isEmpty()) chips.append({QStringLiteral("folder"), defaultProject, {}});
    if (parsed.section) chips.append({QStringLiteral("view-list"), *parsed.section, {}});
    if (parsed.due) {
        const auto [text, cls] = formatDue(parsed.due->date, parsed.due->time, today);
        chips.append({QStringLiteral("x-office-calendar"), text, cls == u"overdue" ? QStringLiteral("negative") : cls == u"today" ? QStringLiteral("positive") : QString()});
        if (parsed.due->isRecurring()) chips.append({QStringLiteral("media-playlist-repeat"), capitalise(parsed.due->recurrence->describe()), {}});
    }
    if (parsed.priority && priorityIsSet(*parsed.priority))
        chips.append({QStringLiteral("emblem-important"), priorityLabel(*parsed.priority), priorityRole(*parsed.priority)});
    for (const QString &label : parsed.labels) chips.append({QStringLiteral("user-bookmarks"), label, {}});
    for (qint64 minutes : parsed.reminders) chips.append({QStringLiteral("alarm"), duration(minutes) + QStringLiteral(" before"), {}});
    return chips;
}

std::optional<ProjectId> View::projectId() const {
    if (id.startsWith(u"project:")) return id.mid(8);
    return std::nullopt;
}

std::optional<FilterId> View::filterId() const {
    if (id.startsWith(u"filter:")) return id.mid(7);
    return std::nullopt;
}

QList<View> builtinViews() {
    return {
        {QStringLiteral("inbox"), QStringLiteral("Inbox"), QStringLiteral("mail-unread"), QStringLiteral("#Inbox"), std::nullopt, 0,
         QStringLiteral("Inbox zero"), QStringLiteral("New tasks with no project land here.")},
        {QStringLiteral("today"), QStringLiteral("Today"), QStringLiteral("view-continuous"), QStringLiteral("due: today | overdue"), std::nullopt, 0,
         QStringLiteral("Nothing due today"), QStringLiteral("Enjoy it.")},
        {QStringLiteral("upcoming"), QStringLiteral("Upcoming"), QStringLiteral("x-office-calendar"), QStringLiteral("due after: today"), std::nullopt, 0,
         QStringLiteral("Nothing scheduled"), QStringLiteral("Tasks with a future date will appear here.")},
        {QStringLiteral("pinned"), QStringLiteral("Pinned"), QStringLiteral("view-pin"), QStringLiteral("pinned"), std::nullopt, 0,
         QStringLiteral("Nothing pinned"), QStringLiteral("Pin a task to keep it in reach.")},
        {QStringLiteral("completed"), QStringLiteral("Completed"), QStringLiteral("object-select"), QStringLiteral("completed"), std::nullopt, 0,
         QStringLiteral("Nothing completed yet"), QStringLiteral("Finished tasks are kept here.")},
    };
}

QList<View> filterViews(const Store &store) {
    QList<View> views;
    for (const SavedFilter *filter : store.filtersOrdered())
        views.append({QStringLiteral("filter:") + filter->id, filter->name, QStringLiteral("edit-find"), filter->query, filter->color, 0,
                      QStringLiteral("Nothing matches"), QStringLiteral("No tasks match this filter right now.")});
    return views;
}

QString escapeQueryName(const QString &name) {
    QString escaped;
    for (const QChar c : name) {
        if (c == u'&' || c == u'|' || c == u'!' || c == u',' || c == u'(' || c == u')' || c == u'\\') escaped += u'\\';
        escaped += c;
    }
    return escaped;
}

static void walkProjects(const Store &store, const std::optional<ProjectId> &parent, int depth, QList<View> &out) {
    for (const Project *project : store.projectsOrdered()) {
        if (project->isInbox() || project->parentId != parent) continue;
        out.append({QStringLiteral("project:") + project->id, project->name, QStringLiteral("folder"),
                    QLatin1Char('#') + escapeQueryName(project->name), project->color, depth,
                    QStringLiteral("No tasks yet"), QStringLiteral("Add one with ctrl+n.")});
        walkProjects(store, project->id, depth + 1, out);
    }
}

QList<View> projectViews(const Store &store) {
    QList<View> views;
    walkProjects(store, std::nullopt, 0, views);
    return views;
}

} // namespace planner
