#include "schedule.h"

#include "store.h"

#include <algorithm>

namespace planner {

static QString key(const TaskId &task, const ReminderId &reminder) { return task + QLatin1Char('/') + reminder; }

// Every reminder on an open task, resolved to an instant. A BeforeDue
// reminder on a task with a date but no time is skipped: "30 minutes before
// some time on Friday" has no answer.
static QList<Firing> pending(const Store &store, const QTimeZone &zone) {
    QList<Firing> firings;
    for (const Task &task : store.tasks()) {
        if (task.checked) continue;
        for (const Reminder &reminder : task.reminders) {
            std::optional<QDateTime> at;
            if (reminder.trigger.kind == Trigger::Absolute) {
                at = reminder.trigger.at;
            } else if (task.due && task.due->time) {
                // Qt resolves a skipped or ambiguous local time to the nearest
                // valid one rather than dropping it: a reminder an hour out is
                // better than one that never arrives.
                const QDateTime local(task.due->date, *task.due->time, zone);
                if (local.isValid()) at = local.toUTC().addSecs(-60 * reminder.trigger.minutes);
            }
            if (at) firings.append({task.id, reminder.id, task.content, *at});
        }
    }
    return firings;
}

QList<Firing> Schedule::takeDue(const Store &store, const QDateTime &now, const QTimeZone &zone) {
    QList<Firing> due;
    for (const Firing &firing : pending(store, zone))
        if (firing.at <= now && !m_fired.contains(key(firing.task, firing.reminder))) due.append(firing);
    std::stable_sort(due.begin(), due.end(), [](const Firing &a, const Firing &b) { return a.at < b.at; });
    for (const Firing &firing : due) m_fired.insert(key(firing.task, firing.reminder));
    return due;
}

void Schedule::catchUp(const Store &store, const QDateTime &now, const QTimeZone &zone) {
    for (const Firing &firing : pending(store, zone))
        if (firing.at <= now) m_fired.insert(key(firing.task, firing.reminder));
}

std::optional<QDateTime> Schedule::nextAfter(const Store &store, const QDateTime &now, const QTimeZone &zone) const {
    std::optional<QDateTime> next;
    for (const Firing &firing : pending(store, zone))
        if (firing.at > now && (!next || firing.at < *next)) next = firing.at;
    return next;
}

void Schedule::forget(const TaskId &task) {
    const QString prefix = task + QLatin1Char('/');
    QSet<QString> kept;
    for (const QString &entry : m_fired)
        if (!entry.startsWith(prefix)) kept.insert(entry);
    m_fired = kept;
}

} // namespace planner
