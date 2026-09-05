#include "demo.h"

#include "quickadd.h"

namespace planner {

static TaskId add(Store &store, const char *line, const QDate &today, const QDateTime &now) {
    const QuickAdd parsed = parseQuickAdd(QString::fromUtf8(line), today, store.vocabulary());
    return store.addFromQuickAdd(parsed, inboxId(), std::nullopt, now);
}

void seedDemo(Store &store, const QDate &today, const QDateTime &now) {
    const ProjectId work = store.addProject(Project::create(QStringLiteral("Work"), Color::Blue), now);
    Project admin = Project::create(QStringLiteral("Admin"), Color::Teal);
    admin.parentId = work;
    store.addProject(admin, now);
    store.addProject(Project::create(QStringLiteral("Home"), Color::Green), now);

    for (const char *line : {"Email Sam about the lease @email p1 today 9am", "Renew the parking permit p2 today", "Water the plants every! 10 days",
                             "Book the dentist @phone", "Pay the electricity bill p3 yesterday", "Weekly review every monday"})
        add(store, line, today, now);

    // One overdue with a deadline, and one with subtasks.
    const TaskId late = add(store, "File the tax return p1 yesterday", today, now);
    store.taskMut(late)->deadline = today.addDays(-2);

    const TaskId parent = add(store, "Move house @errand @home p2 today 09:00", today, now);
    store.taskMut(parent)->description = QStringLiteral("Ring the agent before Friday.\nConfirm the van booking.");
    store.taskMut(parent)->deadline = today.addDays(9);
    for (const char *child : {"Pack the kitchen", "Book a van"}) {
        const TaskId id = add(store, child, today, now);
        store.taskMut(id)->parentId = parent;
    }
    store.completeTask(store.subtasks(parent)[0]->id, now, today);

    const TaskId done = add(store, "Cancel the old broadband", today, now);
    store.completeTask(done, now, today);

    // A project with sections, so the board has columns worth looking at.
    const SectionId doing = store.addSection(Section::create(work, QStringLiteral("In progress")), now);
    const SectionId blocked = store.addSection(Section::create(work, QStringLiteral("Blocked")), now);
    const struct { const char *line; std::optional<SectionId> section; } placed[] = {
        {"Draft the Q3 report p1 tomorrow", doing}, {"Review the contract @legal", doing}, {"Chase the supplier p2", blocked}, {"Tidy the shared drive", std::nullopt}};
    for (const auto &entry : placed) {
        const TaskId id = add(store, entry.line, today, now);
        Task *task = store.taskMut(id);
        task->projectId = work;
        task->sectionId = entry.section;
    }
}

} // namespace planner
