// Which reminders are due, and when the next one is.
//
// A reminder that has already passed when the app starts does not fire:
// launching on Tuesday and being told about Monday's meeting is noise. Only
// reminders that come due while the app is running are shown, and Schedule
// remembers which have gone off so a tick does not repeat them.
#pragma once

#include "model.h"

#include <QSet>
#include <QTimeZone>

namespace planner {

class Store;

struct Firing {
    TaskId task;
    ReminderId reminder;
    QString title;
    QDateTime at;
};

class Schedule {
public:
    // Everything due at or before `now` and not yet shown, oldest first.
    QList<Firing> takeDue(const Store &store, const QDateTime &now, const QTimeZone &zone);
    // Treat everything currently due as already shown; called once at startup.
    void catchUp(const Store &store, const QDateTime &now, const QTimeZone &zone);
    std::optional<QDateTime> nextAfter(const Store &store, const QDateTime &now, const QTimeZone &zone) const;
    // Let a task's reminders fire again, after it repeated.
    void forget(const TaskId &task);
    int firedCount() const { return static_cast<int>(m_fired.size()); }

private:
    QSet<QString> m_fired;   // task + '/' + reminder
};

} // namespace planner
