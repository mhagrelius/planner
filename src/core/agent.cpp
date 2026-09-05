#include "agent.h"

#include "dates.h"
#include "query.h"
#include "quickadd.h"
#include "search.h"
#include "store.h"

#include <QJsonArray>
#include <QJsonDocument>

#include <algorithm>

namespace planner::agent {

// --- what the surface says about itself ---------------------------------------

static const Argument kTaskReference{"task", true,
    "A task, named by its id or by any part of its title. A title that matches more than one open task comes back as an "
    "`ambiguous` error listing the candidates and their ids."};

const QList<Verb> &verbs() {
    static const QList<Verb> table = {
        {"help", {}, "planner agent help [verb]", "This text, or everything about one verb.", false,
         {{"verb", false, "The verb to explain. Omit it for the whole surface."}}, "Plain text, not JSON.", {"planner agent help update"}},
        {"describe", {}, "planner agent describe", "Every verb as JSON, for generating tool definitions.", false, {},
         "`{verbs: [{name, usage, summary, mutates, arguments, returns}]}`.", {"planner agent describe"}},
        {"overview", {"projects"}, "planner agent overview", "Projects, sections, labels, filters, and what is outstanding.", false, {},
         "`{projects, labels, filters, counts}`. Start here — it is the only call that shows what project and label names exist, which "
         "the other verbs expect you to use.", {"planner agent overview"}},
        {"list", {"tasks"}, "planner agent list [query] [limit=N]", "The tasks matching a filter query.", false,
         {{"query", false, "A filter query (see FILTER QUERIES). Omitted, it lists every open task."},
          {"limit", false, "How many to return. Defaults to 50. The response always says how many matched, so a truncated list is visible rather than silent."}},
         "`{tasks, count, matched, truncated}`. Completed tasks are excluded unless the query says `completed`.",
         {"planner agent list due: today | overdue", "planner agent list '#Work & p1'", "planner agent list 'no date' limit=10"}},
        {"show", {"task"}, "planner agent show <task>", "One task in full, with its description and subtasks.", false, {kTaskReference},
         "`{task}`, including `description`, `reminders` and nested `subtasks` — none of which `list` returns.", {"planner agent show 'Email Sam'"}},
        {"search", {"find"}, "planner agent search <text> [limit=N]", "Tasks, projects and labels whose names contain some text.", false,
         {{"text", true, "What to look for. Substring, case-insensitive."}, {"limit", false, "How many to return. Defaults to 50."}},
         "`{hits}`, each with a `kind` of task, project or label, and an id. Use this to turn a vague reference from the user into an id "
         "before calling a verb that changes something.", {"planner agent search lease"}},
        {"add", {"new"}, "planner agent add <line>", "Create a task from a natural-language line.", true,
         {{"line", true, "A quick-add line (see QUICK-ADD LINES). Everything — project, labels, priority, date, repeat — goes in the one line rather than in separate fields."}},
         "`{task}`, the task as it was actually understood. Check it rather than assuming: the date and project came from parsing prose, so this is where a misread shows up.",
         {"planner agent add Email Sam about the lease #Work @email p2 friday 9am", "planner agent add Water the plants every other monday"}},
        {"subtask", {}, "planner agent subtask <parent> <line>", "Create a task underneath another one.", true,
         {{"parent", true, "The task to nest under, as ONE argument. Quote it if it has spaces — unlike the other verbs, there is a second argument after it."},
          {"line", true, "A quick-add line for the subtask."}},
         "`{task}`. A subtask always shares its parent's project, whatever the line says.", {"planner agent subtask 'Move house' Pack the books p1"}},
        {"complete", {"done", "check"}, "planner agent complete <task>", "Tick a task off.", true, {kTaskReference},
         "`{task, outcome}`. The outcome is `done`, `already-done`, or `completed-and-repeats` with a `next_due` date. The last one means the "
         "completion worked and the task repeats, so it is open again later — report it as done *and* say when it comes back, not as a "
         "reschedule. Completing a task completes its subtasks.", {"planner agent complete 'Email Sam'"}},
        {"reopen", {"uncomplete", "uncheck"}, "planner agent reopen <task>", "Put a completed task back.", true, {kTaskReference},
         "`{task, reopened}`. `reopened` is false when the task was not ticked off to begin with, so a no-op does not read as an undo. "
         "Reopening a subtask reopens the parents above it, since an open task cannot sit inside a finished one.", {"planner agent reopen 'Email Sam'"}},
        {"delete", {"remove"}, "planner agent delete <task>", "Delete a task and its subtasks.", true, {kTaskReference},
         "`{removed, count}`, every task that went, so you can report the subtasks that went with it. There is no undo from here — prefer "
         "`complete` when the user meant they had finished it.", {"planner agent delete 'Email Sam'"}},
        {"update", {"edit", "set"}, "planner agent update <task> <field=value> [field=value ...]", "Change fields of an existing task.", true,
         {kTaskReference,
          {"title", false, "The title, as plain text. Quick-add tokens are not parsed here — set the date with `due=`."},
          {"description", false, "The long text under the title. Markdown."},
          {"due", false, "A date phrase: `friday`, `next monday 9am`, `in 3 days`, `every other week`. `due=none` clears it, which also stops it repeating."},
          {"deadline", false, "The hard date, as distinct from the day to work on it. A date phrase, or `none`."},
          {"priority", false, "`p1` to `p4`. p1 is most urgent; p4 means unset."},
          {"project", false, "Move it to another project, by name or id. Its subtasks go with it."},
          {"section", false, "File it under a section of its project, by name. `section=none` takes it out of one. Sections are created in the app, not from here."},
          {"pinned", false, "`true` or `false`."},
          {"add-label", false, "Put a label on it, creating the label if it is new."},
          {"remove-label", false, "Take a label off it. The label itself stays."}},
         "`{task, applied}`. `applied` lists what actually changed, so a value that was already set is visibly a no-op rather than a silent one.",
         {"planner agent update 'Email Sam' due=next friday priority=p1", "planner agent update 'Email Sam' project=Work add-label=urgent"}},
        {"add-project", {"new-project"}, "planner agent add-project <name> [parent=<project>]", "Create a project.", true,
         {{"name", true, "What to call it."}, {"parent", false, "An existing project to nest it under."}},
         "`{project}`, with the colour it was given.", {"planner agent add-project Loft conversion parent=Home"}},
        {"rename-project", {}, "planner agent rename-project <project> <name>", "Rename a project.", true,
         {{"project", true, "The project to rename, as ONE argument. Quote it if it has spaces."}, {"name", true, "The new name."}},
         "`{project}`.", {"planner agent rename-project Work 'Work — 2026'"}},
        {"remove-project", {"delete-project"}, "planner agent remove-project <project>", "Delete a project, its subprojects, and every task in them.", true,
         {{"project", true, "The project to delete. The Inbox cannot be deleted."}},
         "`{name, projects, tasks}` — how much went. This deletes tasks the user may not have had in mind; confirm before calling it.",
         {"planner agent remove-project 'Loft conversion'"}},
    };
    return table;
}

static const Verb *findVerb(const QString &word) {
    for (const Verb &verb : verbs()) {
        if (word == QLatin1String(verb.name)) return &verb;
        for (const char *alias : verb.aliases)
            if (word == QLatin1String(alias)) return &verb;
    }
    return nullptr;
}

std::optional<QString> canonicalVerb(const QString &word) {
    const Verb *verb = findVerb(word);
    if (!verb) return std::nullopt;
    return QString::fromLatin1(verb->name);
}

QStringList verbNames() {
    QStringList names;
    for (const Verb &verb : verbs()) names << QLatin1String(verb.name);
    return names;
}

static const char *const kQuickAddHelp =
    "QUICK-ADD LINES\n"
    "  `add` and `subtask` take one line of prose. Tokens are recognised anywhere in\n"
    "  it and removed from the title, so what is left is the title.\n"
    "\n"
    "    #Project    file it in a project, by name. A #project that does not exist\n"
    "                is NOT created — the task lands in the Inbox instead, where it\n"
    "                is visible, rather than a misspelling becoming a new project.\n"
    "    /Section    file it under a section of that project\n"
    "    @label      put a label on it. A @label that does not exist IS created.\n"
    "    p1 p2 p3 p4 priority. p1 is most urgent, p4 is none\n"
    "    !30m        remind that long before it is due\n"
    "    dates       today, tomorrow, fri, next friday, 27th, in 3 days,\n"
    "                end of month, 9am, friday 9am\n"
    "    repeats     every day, every other monday, every 3 weeks, every month,\n"
    "                every weekday, every year until 1 Jan 2027, every day x5\n"
    "                `every!` repeats from the day you complete it, not from the\n"
    "                due date — use it for \"water the plants every 3 days\".\n"
    "\n"
    "  Example: Email Sam about the lease #Work /Admin @email p2 friday 9am !30m\n"
    "           becomes the task \"Email Sam about the lease\", in Work's Admin\n"
    "           section, labelled email, priority 2, due 09:00 on Friday, with a\n"
    "           reminder half an hour before.";

static const char *const kQueryHelp =
    "FILTER QUERIES\n"
    "  `list` takes the same query language the app's own views are built from.\n"
    "\n"
    "    due: today            deadline: friday      overdue\n"
    "    due before: friday    due after: monday     no date\n"
    "    p1                    @label                no labels\n"
    "    #Project              ##Project (with its subprojects)     /Section\n"
    "    pinned                recurring             subtask\n"
    "    completed             search: some text     no deadline\n"
    "\n"
    "  Combine with `&` (and), `|` (or), `!` (not) and parentheses.\n"
    "\n"
    "    #Work & p1                      due: today | overdue\n"
    "    ##Home & !p4 & @errand          (p1 | p2) & no date\n"
    "\n"
    "  Completed tasks are left out unless the query says `completed`.";

static QString collapse(const QString &text) { return text.simplified(); }

static QString firstSentence(const QString &summary) {
    const QString collapsed = collapse(summary);
    const int end = collapsed.indexOf(QStringLiteral(". "));
    return end < 0 ? collapsed : collapsed.left(end + 1);
}

static QString wrap(const QString &text, const QString &indent) {
    constexpr int width = 78;
    QStringList lines;
    QString line = indent;
    for (const QString &word : collapse(text).split(u' ')) {
        if (line.size() > indent.size() && line.size() + 1 + word.size() > width) {
            lines << line;
            line = indent;
        } else if (line.size() > indent.size()) {
            line += u' ';
        }
        line += word;
    }
    lines << line;
    return lines.join(u'\n');
}

QString helpOverview() {
    QString text = QStringLiteral(
        "planner agent — read and change tasks from a script or an assistant.\n"
        "\n"
        "USAGE\n"
        "  planner agent <verb> [arguments]\n"
        "\n"
        "  Every verb prints one JSON object on stdout and exits 0, or prints\n"
        "  {\"ok\": false, \"error\": ...} and exits 1. `help` prints text instead.\n"
        "\n"
        "  Arguments are positional words and `key=value` pairs. There are no `--flags`.\n"
        "\n"
        "  If Planner is running, the command is handed to it, so the window updates as\n"
        "  you go and there is no second copy of the file to fall out of step. If it is\n"
        "  not, the command reads and writes the file itself.\n"
        "\n"
        "VERBS\n");
    int width = 0;
    for (const Verb &verb : verbs()) width = std::max(width, static_cast<int>(strlen(verb.name)));
    for (const Verb &verb : verbs())
        text += QStringLiteral("  %1 %2  %3\n").arg(verb.mutates ? u'*' : u' ').arg(QLatin1String(verb.name), -width).arg(firstSentence(QLatin1String(verb.summary)));
    text += QStringLiteral("\n  * changes something. Everything else only reads.\n\n"
                           "Run `planner agent help <verb>` for arguments and examples, or\n"
                           "`planner agent describe` for the same thing as JSON.\n\n");
    text += QLatin1String(kQuickAddHelp);
    text += QStringLiteral("\n\n");
    text += QLatin1String(kQueryHelp);
    text += QStringLiteral("\n\nREFERRING TO A TASK\n  By id, or by any part of its title. A title matching several open\n  tasks is an `ambiguous` "
                           "error listing them with their ids, rather\n  than a guess — use `search` first when the user was vague, and pass\n  the id when you already have it.\n");
    return text;
}

std::optional<QString> helpForVerb(const QString &name) {
    const Verb *verb = findVerb(name);
    if (!verb) return std::nullopt;
    QString text = QStringLiteral("%1\n\n%2\n\nUSAGE\n  %3\n").arg(QLatin1String(verb->name), wrap(QLatin1String(verb->summary), QStringLiteral("  ")), QLatin1String(verb->usage));
    if (!verb->aliases.isEmpty()) {
        QStringList aliases;
        for (const char *alias : verb->aliases) aliases << QLatin1String(alias);
        text += QStringLiteral("\nALSO CALLED\n  %1\n").arg(aliases.join(QStringLiteral(", ")));
    }
    if (!verb->arguments.isEmpty()) {
        text += QStringLiteral("\nARGUMENTS\n");
        for (const Argument &argument : verb->arguments)
            text += QStringLiteral("  %1 (%2)\n%3\n").arg(QLatin1String(argument.name), argument.required ? QStringLiteral("required") : QStringLiteral("optional"),
                                                         wrap(QLatin1String(argument.description), QStringLiteral("      ")));
    }
    text += QStringLiteral("\nRETURNS\n%1\n").arg(wrap(QLatin1String(verb->returns), QStringLiteral("  ")));
    if (verb->mutates) text += QStringLiteral("\n  This changes the store.\n");
    if (!verb->examples.isEmpty()) {
        text += QStringLiteral("\nEXAMPLES\n");
        for (const char *example : verb->examples) text += QStringLiteral("  %1\n").arg(QLatin1String(example));
    }
    const QLatin1String verbName(verb->name);
    if (verbName == QLatin1String("add") || verbName == QLatin1String("subtask")) text += QLatin1Char('\n') + QLatin1String(kQuickAddHelp) + QLatin1Char('\n');
    if (verbName == QLatin1String("list")) text += QLatin1Char('\n') + QLatin1String(kQueryHelp) + QLatin1Char('\n');
    return text;
}

// --- errors --------------------------------------------------------------------

QJsonObject Error::toJson() const {
    QJsonObject json{{QStringLiteral("error"), kind}, {QStringLiteral("message"), message}};
    if (!candidates.isEmpty()) {
        QJsonArray array;
        for (const Candidate &candidate : candidates) {
            QJsonObject entry{{QStringLiteral("id"), candidate.id}, {QStringLiteral("name"), candidate.name}};
            if (!candidate.context.isEmpty()) entry.insert(QStringLiteral("context"), candidate.context);
            array.append(entry);
        }
        json.insert(QStringLiteral("candidates"), array);
    }
    if (!hint.isEmpty()) json.insert(QStringLiteral("hint"), hint);
    return json;
}

static Result fail(const QString &kind, const QString &message, const QString &hint = QString()) {
    Result result;
    result.ok = false;
    result.error.kind = kind;
    result.error.message = message;
    result.error.hint = hint;
    return result;
}

static Result failWith(const Error &error) {
    Result result;
    result.ok = false;
    result.error = error;
    return result;
}

static Error missing(const QString &verb, const QString &wanted) {
    return {QStringLiteral("missing-argument"), QStringLiteral("`%1` needs %2.").arg(verb, wanted), {},
            QStringLiteral("Run `planner agent help %1` for the arguments.").arg(verb)};
}

static Error unknownField(const QString &key, const QString &verb, const QStringList &allowed) {
    return {QStringLiteral("unknown-field"), QStringLiteral("`%1` has no `%2` field. It takes: %3.").arg(verb, key, allowed.join(QStringLiteral(", "))), {},
            QStringLiteral("Run `planner agent help %1`.").arg(verb)};
}

// --- views ---------------------------------------------------------------------

// A due date as one string: the date, and the time if there is one. Not an
// instant: a due date is a date in the user's own day.
static QString duePhrase(const Due &due) {
    return due.time ? QStringLiteral("%1 %2").arg(dateSerial(due.date), due.time->toString(QStringLiteral("HH:mm"))) : dateSerial(due.date);
}

static QString reminderPhrase(const Reminder &reminder) {
    if (reminder.trigger.kind == Trigger::Absolute) return reminder.trigger.at.toUTC().toString(QStringLiteral("yyyy-MM-dd HH:mm 'UTC'"));
    const qint64 m = reminder.trigger.minutes;
    if (m % 1440 == 0) return QStringLiteral("%1 days before").arg(m / 1440);
    if (m % 60 == 0) return QStringLiteral("%1 hours before").arg(m / 60);
    return QStringLiteral("%1 minutes before").arg(m);
}

QJsonObject taskView(const Store &store, const Task &task, const QDate &today, bool detailed) {
    QJsonObject json;
    json.insert(QStringLiteral("id"), task.id);
    json.insert(QStringLiteral("content"), task.content);
    if (detailed && !task.description.isEmpty()) json.insert(QStringLiteral("description"), task.description);
    const Project *project = store.project(task.projectId);
    // A task whose project has gone is a hand-edited file, not a state the
    // app produces. Saying so is more use than an id.
    json.insert(QStringLiteral("project"), project ? project->name : QStringLiteral("(unknown project)"));
    if (task.sectionId) {
        const auto found = store.section(*task.sectionId);
        if (found.second) json.insert(QStringLiteral("section"), found.second->name);
    }
    if (task.due) {
        json.insert(QStringLiteral("due"), duePhrase(*task.due));
        if (task.due->recurrence) json.insert(QStringLiteral("repeats"), task.due->recurrence->describe());
    }
    if (task.deadline) json.insert(QStringLiteral("deadline"), dateSerial(*task.deadline));
    json.insert(QStringLiteral("priority"), priorityToken(task.priority));
    QJsonArray labels;
    for (const LabelId &id : task.labels)
        if (const Label *label = store.label(id)) labels.append(label->name);
    if (!labels.isEmpty()) json.insert(QStringLiteral("labels"), labels);
    if (task.pinned) json.insert(QStringLiteral("pinned"), true);
    if (task.isOverdue(today)) json.insert(QStringLiteral("overdue"), true);
    if (task.checked) json.insert(QStringLiteral("completed"), true);
    if (task.parentId) json.insert(QStringLiteral("parent"), *task.parentId);
    const QList<const Task *> children = store.subtasks(task.id);
    if (!children.isEmpty()) {
        int done = 0;
        for (const Task *child : children) done += child->checked ? 1 : 0;
        json.insert(QStringLiteral("subtask_count"), QJsonObject{{QStringLiteral("done"), done}, {QStringLiteral("total"), static_cast<int>(children.size())}});
    }
    if (detailed) {
        QJsonArray reminders;
        for (const Reminder &reminder : task.reminders) reminders.append(reminderPhrase(reminder));
        if (!reminders.isEmpty()) json.insert(QStringLiteral("reminders"), reminders);
        QJsonArray subtasks;
        for (const Task *child : children) subtasks.append(taskView(store, *child, today, true));
        if (!subtasks.isEmpty()) json.insert(QStringLiteral("subtasks"), subtasks);
    }
    return json;
}

static QJsonObject projectView(const Store &store, const Project &project) {
    QJsonObject json{{QStringLiteral("id"), project.id}, {QStringLiteral("name"), project.name}};
    if (project.parentId) {
        if (const Project *parent = store.project(*project.parentId)) json.insert(QStringLiteral("parent"), parent->name);
    }
    QJsonArray sections;
    for (const Section *section : project.sectionsOrdered()) sections.append(section->name);
    if (!sections.isEmpty()) json.insert(QStringLiteral("sections"), sections);
    const auto [done, total] = store.progress(project.id);
    json.insert(QStringLiteral("open"), total - done);
    json.insert(QStringLiteral("done"), done);
    if (project.isInbox()) json.insert(QStringLiteral("inbox"), true);
    return json;
}

// --- resolving references -------------------------------------------------------

static QString taskContext(const Store &store, const Task &task) {
    const Project *project = store.project(task.projectId);
    QString text = project ? project->name : QStringLiteral("unknown project");
    if (task.due) text += QStringLiteral(", due ") + duePhrase(*task.due);
    if (task.checked) text += QStringLiteral(", completed");
    return text;
}

static std::optional<TaskId> pickTask(const Store &store, const QString &wanted, const QList<const Task *> &matches, Error *error) {
    if (matches.size() == 1) return matches[0]->id;
    QList<const Task *> open;
    for (const Task *task : matches)
        if (!task->checked) open.append(task);
    if (open.size() == 1) return open[0]->id;
    if (error) {
        error->kind = QStringLiteral("ambiguous");
        error->message = QStringLiteral("`%1` matches %2 tasks. Name one by its id.").arg(wanted).arg(matches.size());
        for (const Task *task : matches) error->candidates.append({task->id, task->content, taskContext(store, *task)});
    }
    return std::nullopt;
}

std::optional<TaskId> resolveTask(const Store &store, const QString &reference, Error *error) {
    const QString wanted = reference.trimmed();
    for (const Task &task : store.tasks())
        if (task.id == wanted) return task.id;
    QList<const Task *> exact;
    for (const Task &task : store.tasks())
        if (task.content.trimmed().compare(wanted, Qt::CaseInsensitive) == 0) exact.append(&task);
    if (!exact.isEmpty()) return pickTask(store, wanted, exact, error);
    QList<const Task *> partial;
    for (const Task &task : store.tasks())
        if (task.content.toLower().contains(wanted.toLower())) partial.append(&task);
    if (!partial.isEmpty()) return pickTask(store, wanted, partial, error);
    if (error) *error = {QStringLiteral("not-found"), QStringLiteral("No task matches `%1`.").arg(wanted), {},
                         QStringLiteral("`planner agent search %1` looks across projects and labels too.").arg(wanted)};
    return std::nullopt;
}

std::optional<ProjectId> resolveProject(const Store &store, const QString &reference, Error *error) {
    const QString wanted = reference.trimmed();
    for (const Project &project : store.projects())
        if (project.id == wanted) return project.id;
    if (const Project *named = store.projectByName(wanted)) return named->id;
    QList<const Project *> partial;
    for (const Project &project : store.projects())
        if (project.name.toLower().contains(wanted.toLower())) partial.append(&project);
    if (partial.size() == 1) return partial[0]->id;
    if (error) {
        if (partial.isEmpty()) {
            *error = {QStringLiteral("not-found"), QStringLiteral("There is no project called `%1`.").arg(wanted), {},
                      QStringLiteral("`planner agent overview` lists the projects that exist. A project is not created by naming one that does not.")};
        } else {
            error->kind = QStringLiteral("ambiguous");
            error->message = QStringLiteral("`%1` matches %2 projects. Name one exactly, or by its id.").arg(wanted).arg(partial.size());
            for (const Project *project : partial) error->candidates.append({project->id, project->name, {}});
        }
    }
    return std::nullopt;
}

static std::optional<SectionId> resolveSection(const Store &store, const ProjectId &projectId, const QString &reference, Error *error) {
    const QString wanted = reference.trimmed();
    const Project *owner = store.project(projectId);
    if (!owner) {
        if (error) *error = {QStringLiteral("not-found"), QStringLiteral("There is no project `%1` to look for `%2` in.").arg(projectId, wanted), {}, {}};
        return std::nullopt;
    }
    const auto sections = owner->sectionsOrdered();
    for (const Section *section : sections)
        if (section->name.compare(wanted, Qt::CaseInsensitive) == 0) return section->id;
    for (const Section *section : sections)
        if (section->name.toLower().contains(wanted.toLower())) return section->id;
    if (error) {
        if (sections.isEmpty()) {
            *error = {QStringLiteral("not-found"), QStringLiteral("`%1` has no sections.").arg(owner->name), {}, QStringLiteral("Sections are created in the app, not from here.")};
        } else {
            QStringList names;
            for (const Section *section : sections) names << section->name;
            *error = {QStringLiteral("not-found"), QStringLiteral("`%1` has no section called `%2`.").arg(owner->name, wanted), {},
                      QStringLiteral("Its sections are: %1.").arg(names.join(QStringLiteral(", ")))};
        }
    }
    return std::nullopt;
}

// --- reading arguments -----------------------------------------------------------

// The key a `key=value` token opens: lower-case ASCII, so an `=` inside prose
// does not turn the rest of the line into a field.
static std::optional<std::pair<QString, QString>> keyOf(const QString &argument) {
    const int eq = argument.indexOf(u'=');
    if (eq <= 0) return std::nullopt;
    const QString key = argument.left(eq);
    if (!key.at(0).isLower() || !key.at(0).isLetter()) return std::nullopt;
    for (const QChar c : key)
        if (!((c.isLower() && c.isLetter() && c.unicode() < 128) || c.isDigit() || c == u'-')) return std::nullopt;
    return std::make_pair(key, argument.mid(eq + 1));
}

// Leading words, then `key=value` pairs whose values run until the next key.
static std::pair<QStringList, QList<std::pair<QString, QString>>> splitPairs(const QStringList &args) {
    QStringList leading;
    QList<std::pair<QString, QString>> pairs;
    for (const QString &argument : args) {
        if (const auto pair = keyOf(argument)) {
            pairs.append(*pair);
        } else if (!pairs.isEmpty()) {
            QString &value = pairs.last().second;
            if (!value.isEmpty()) value += u' ';
            value += argument;
        } else {
            leading << argument;
        }
    }
    return {leading, pairs};
}

static QString join(const QStringList &words) { return words.join(u' ').trimmed(); }

static std::optional<int> takeLimit(const QList<std::pair<QString, QString>> &pairs, const QString &verb, Error *error) {
    int limit = 50;
    for (const auto &[key, value] : pairs) {
        if (key != QLatin1String("limit")) {
            *error = unknownField(key, verb, {QStringLiteral("limit")});
            return std::nullopt;
        }
        bool ok = false;
        limit = value.trimmed().toInt(&ok);
        if (!ok) {
            *error = {QStringLiteral("bad-value"), QStringLiteral("`limit=%1` is not a whole number.").arg(value), {}, {}};
            return std::nullopt;
        }
    }
    return limit;
}

static std::optional<bool> boolean(const QString &value) {
    const QString lowered = value.toLower();
    if (lowered == u"true" || lowered == u"yes" || lowered == u"on" || lowered == u"1") return true;
    if (lowered == u"false" || lowered == u"no" || lowered == u"off" || lowered == u"0") return false;
    return std::nullopt;
}

// A date phrase, understood exactly as quick-add understands it, so `due=`
// accepts everything the entry box does: a time, a repeat, or both.
static std::optional<Due> parseDue(const QString &phrase, const QDate &today, Error *error) {
    const QuickAdd parsed = parseQuickAdd(phrase, today, Vocabulary());
    const QString hint = QStringLiteral("Dates look like: friday, next monday 9am, 27th, in 3 days, every other week.");
    if (parsed.due && !parsed.title.trimmed().isEmpty()) {
        // Words the date parser did not account for mean it read something
        // other than what was intended.
        *error = {QStringLiteral("bad-date"), QStringLiteral("I did not understand `%1` in the date `%2`.").arg(parsed.title.trimmed(), phrase), {}, hint};
        return std::nullopt;
    }
    if (!parsed.due) {
        *error = {QStringLiteral("bad-date"), QStringLiteral("`%1` is not a date I understand.").arg(phrase), {}, hint};
        return std::nullopt;
    }
    return parsed.due;
}

// --- the verbs --------------------------------------------------------------------

static Result respond(const QString &action, const QJsonObject &body, bool changed) {
    Result result;
    result.ok = true;
    result.action = action;
    result.body = body;
    result.changedStore = changed;
    return result;
}

static Result overview(const Store &store, const QDate &today) {
    QJsonArray projects, labels, filters;
    for (const Project *project : store.projectsOrdered()) projects.append(projectView(store, *project));
    const QHash<LabelId, int> counts = store.labelCounts();
    for (const Label &label : store.labels()) labels.append(QJsonObject{{QStringLiteral("name"), label.name}, {QStringLiteral("open"), counts.value(label.id, 0)}});
    for (const SavedFilter *filter : store.filtersOrdered()) filters.append(QJsonObject{{QStringLiteral("name"), filter->name}, {QStringLiteral("query"), filter->query}});
    int open = 0, completed = 0, dueToday = 0, overdue = 0, inbox = 0;
    for (const Task &task : store.tasks()) {
        if (task.checked) { ++completed; continue; }
        ++open;
        if (task.due && task.due->date == today) ++dueToday;
        if (task.isOverdue(today)) ++overdue;
        if (task.projectId == inboxId()) ++inbox;
    }
    return respond(QStringLiteral("overview"),
                   {{QStringLiteral("projects"), projects}, {QStringLiteral("labels"), labels}, {QStringLiteral("filters"), filters},
                    {QStringLiteral("counts"), QJsonObject{{QStringLiteral("open"), open}, {QStringLiteral("completed"), completed}, {QStringLiteral("due_today"), dueToday},
                                                           {QStringLiteral("overdue"), overdue}, {QStringLiteral("inbox"), inbox}}}},
                   false);
}

// Soonest first, then most urgent, then by title: the order a reader with no
// screen would work in, the same every time so two calls can be compared.
static void sortForReading(QList<const Task *> &tasks) {
    std::stable_sort(tasks.begin(), tasks.end(), [](const Task *a, const Task *b) {
        if (a->due.has_value() != b->due.has_value()) return a->due.has_value();
        if (a->due && b->due && a->due->date != b->due->date) return a->due->date < b->due->date;
        if (priorityRank(a->priority) != priorityRank(b->priority)) return priorityRank(a->priority) < priorityRank(b->priority);
        return a->content < b->content;
    });
}

static Result list(const Store &store, const QString &source, int limit, const QDate &today) {
    Query query = Query::all();
    if (!source.isEmpty()) {
        QueryError error;
        const auto parsed = Query::parse(source, &error);
        if (!parsed) return fail(QStringLiteral("bad-query"), QStringLiteral("`%1` is not a filter query: %2").arg(source, error.message),
                                 QStringLiteral("Run `planner agent help list` for the syntax."));
        query = *parsed;
    }
    QList<const Task *> matches = query.run(store, today);
    sortForReading(matches);
    QJsonArray tasks;
    for (const Task *task : matches) {
        if (tasks.size() >= limit) break;
        tasks.append(taskView(store, *task, today));
    }
    QJsonObject body{{QStringLiteral("count"), tasks.size()}, {QStringLiteral("matched"), static_cast<int>(matches.size())}, {QStringLiteral("tasks"), tasks}};
    if (!source.isEmpty()) body.insert(QStringLiteral("query"), source);
    if (matches.size() > tasks.size()) body.insert(QStringLiteral("truncated"), true);
    return respond(QStringLiteral("list"), body, false);
}

static Result update(Store &store, const QString &reference, const QList<std::pair<QString, QString>> &pairs, const QDateTime &now, const QDate &today) {
    Error error;
    const auto resolved = resolveTask(store, reference, &error);
    if (!resolved) return failWith(error);
    const TaskId id = *resolved;
    QStringList applied;
    std::optional<ProjectId> destination;
    std::optional<std::optional<QString>> section;   // outer: mentioned; inner: a name, or none

    for (const auto &[key, raw] : pairs) {
        const QString value = raw.trimmed();
        // A field that can be unset is cleared by naming nothing, or by the word itself.
        const bool cleared = value.isEmpty() || value.compare(QStringLiteral("none"), Qt::CaseInsensitive) == 0;
        Task *task = store.taskMut(id);
        if (key == u"title" || key == u"content") {
            if (task->content != value) { task->content = value; task->touch(now); applied << QStringLiteral("title → %1").arg(value); }
        } else if (key == u"description" || key == u"notes") {
            if (task->description != value) { task->description = value; task->touch(now); applied << QStringLiteral("description changed"); }
        } else if (key == u"due" || key == u"date") {
            std::optional<Due> due;
            if (!cleared) {
                due = parseDue(value, today, &error);
                if (!due) return failWith(error);
            }
            if (task->due != due) {
                task->due = due;
                task->touch(now);
                applied << QStringLiteral("due → %1").arg(due ? duePhrase(*due) : QStringLiteral("none"));
            }
        } else if (key == u"deadline") {
            std::optional<QDate> deadline;
            if (!cleared) {
                deadline = parseDate(value, today);
                if (!deadline) return fail(QStringLiteral("bad-date"), QStringLiteral("`%1` is not a date I understand.").arg(value),
                                           QStringLiteral("A deadline is a plain day: friday, 27th, next monday, in 3 days."));
            }
            if (task->deadline != deadline) {
                task->deadline = deadline;
                task->touch(now);
                applied << QStringLiteral("deadline → %1").arg(deadline ? dateSerial(*deadline) : QStringLiteral("none"));
            }
        } else if (key == u"priority") {
            auto priority = priorityFromToken(value);
            if (!priority && cleared) priority = Priority::P4;
            if (!priority) return fail(QStringLiteral("bad-value"), QStringLiteral("`priority=%1` is not a priority. Use p1, p2, p3 or p4.").arg(value),
                                       QStringLiteral("p1 is the most urgent; p4 means no priority."));
            if (task->priority != *priority) { task->priority = *priority; task->touch(now); applied << QStringLiteral("priority → %1").arg(priorityToken(*priority)); }
        } else if (key == u"project") {
            if (cleared) return fail(QStringLiteral("bad-value"), QStringLiteral("A task is always in a project. Move it to Inbox rather than clearing it."), QStringLiteral("project=Inbox"));
            destination = resolveProject(store, value, &error);
            if (!destination) return failWith(error);
        } else if (key == u"section") {
            section = cleared ? std::optional<QString>() : std::optional<QString>(value);
        } else if (key == u"pinned" || key == u"pin") {
            const auto pinned = boolean(value);
            if (!pinned) return fail(QStringLiteral("bad-value"), QStringLiteral("`pinned=%1` is not true or false.").arg(value));
            if (task->pinned != *pinned) { task->pinned = *pinned; task->touch(now); applied << QStringLiteral("pinned → %1").arg(*pinned ? QStringLiteral("true") : QStringLiteral("false")); }
        } else if (key == u"add-label" || key == u"label") {
            const LabelId label = store.labelForName(value);
            task = store.taskMut(id);
            if (!task->hasLabel(label)) { task->addLabel(label); task->touch(now); applied << QStringLiteral("label + %1").arg(value); }
        } else if (key == u"remove-label" || key == u"unlabel") {
            if (const Label *label = store.labelByName(value)) {
                if (task->hasLabel(label->id)) { task->removeLabel(label->id); task->touch(now); applied << QStringLiteral("label − %1").arg(value); }
            }
        } else {
            return failWith(unknownField(key, QStringLiteral("update"),
                                         {QStringLiteral("title"), QStringLiteral("description"), QStringLiteral("due"), QStringLiteral("deadline"), QStringLiteral("priority"),
                                          QStringLiteral("project"), QStringLiteral("section"), QStringLiteral("pinned"), QStringLiteral("add-label"), QStringLiteral("remove-label")}));
        }
    }

    // Moves happen last and together, because a section only means anything
    // inside a project: `project=Home section=Doing` looks for Doing in Home.
    if (destination || section) {
        const Task *task = store.task(id);
        const ProjectId currentProject = task->projectId;
        const std::optional<SectionId> currentSection = task->sectionId;
        const ProjectId project = destination.value_or(currentProject);
        std::optional<SectionId> target;
        if (section && *section) {
            target = resolveSection(store, project, **section, &error);
            if (!target) return failWith(error);
        }
        if (project != currentProject || target != currentSection) {
            if (!store.moveTask(id, project, target, 1 << 30, now)) return fail(QStringLiteral("refused"), QStringLiteral("That task could not be moved there."));
            if (destination) applied << QStringLiteral("project → %1").arg(store.project(*destination)->name);
            if (target) applied << QStringLiteral("section → %1").arg(store.section(*target).second->name);
            else if (currentSection) applied << QStringLiteral("section → none");
        }
    }

    return respond(QStringLiteral("updated"), {{QStringLiteral("task"), taskView(store, *store.task(id), today)}, {QStringLiteral("applied"), QJsonArray::fromStringList(applied)}},
                   !applied.isEmpty());
}

Result run(Store &store, const QStringList &args, const QDateTime &now, const QDate &today) {
    if (args.isEmpty()) {
        Result result;
        result.ok = true;
        result.action = QStringLiteral("help");
        result.helpText = helpOverview();
        return result;
    }
    const QString verbWord = args[0].toLower();
    QStringList rest = args.mid(1);
    // `help` after a verb reads the same way round as before it.
    if (!rest.isEmpty() && rest[0] == u"help" && verbWord != u"help") {
        Result result;
        result.ok = true;
        result.action = QStringLiteral("help");
        const auto text = helpForVerb(verbWord);
        if (!text) return fail(QStringLiteral("unknown-verb"), QStringLiteral("`%1` is not a verb. The verbs are: %2.").arg(verbWord, verbNames().join(QStringLiteral(", "))),
                               QStringLiteral("Run `planner agent help` for what each one does."));
        result.helpText = *text;
        return result;
    }
    const auto verb = canonicalVerb(verbWord);
    if (!verb) return fail(QStringLiteral("unknown-verb"), QStringLiteral("`%1` is not a verb. The verbs are: %2.").arg(verbWord, verbNames().join(QStringLiteral(", "))),
                           QStringLiteral("Run `planner agent help` for what each one does."));

    const bool mutates = findVerb(*verb)->mutates;
    // Checked once, here: a store that will not be saved must not be edited in
    // memory either, or the caller reports a success that quietly evaporates.
    if (mutates && store.isReadOnly())
        return fail(QStringLiteral("read-only"), QStringLiteral("This planner file was written by a newer version of Planner, so it will not be changed."),
                    QStringLiteral("Reading still works. Upgrade Planner to write to it."));

    Error error;
    if (*verb == u"help") {
        Result result;
        result.ok = true;
        result.action = QStringLiteral("help");
        if (rest.isEmpty()) {
            result.helpText = helpOverview();
        } else {
            const auto text = helpForVerb(rest[0].toLower());
            if (!text) return fail(QStringLiteral("unknown-verb"), QStringLiteral("`%1` is not a verb. The verbs are: %2.").arg(rest[0], verbNames().join(QStringLiteral(", "))),
                                   QStringLiteral("Run `planner agent help` for what each one does."));
            result.helpText = *text;
        }
        return result;
    }
    if (*verb == u"describe") {
        QJsonArray array;
        for (const Verb &v : verbs()) {
            QJsonObject entry{{QStringLiteral("name"), QLatin1String(v.name)}, {QStringLiteral("usage"), QLatin1String(v.usage)},
                              {QStringLiteral("summary"), QLatin1String(v.summary)}, {QStringLiteral("mutates"), v.mutates},
                              {QStringLiteral("returns"), QLatin1String(v.returns)}};
            QJsonArray arguments;
            for (const Argument &a : v.arguments)
                arguments.append(QJsonObject{{QStringLiteral("name"), QLatin1String(a.name)}, {QStringLiteral("required"), a.required}, {QStringLiteral("description"), QLatin1String(a.description)}});
            entry.insert(QStringLiteral("arguments"), arguments);
            if (!v.aliases.isEmpty()) {
                QJsonArray aliases;
                for (const char *alias : v.aliases) aliases.append(QLatin1String(alias));
                entry.insert(QStringLiteral("aliases"), aliases);
            }
            if (!v.examples.isEmpty()) {
                QJsonArray examples;
                for (const char *example : v.examples) examples.append(QLatin1String(example));
                entry.insert(QStringLiteral("examples"), examples);
            }
            array.append(entry);
        }
        return respond(QStringLiteral("describe"), {{QStringLiteral("verbs"), array}}, false);
    }
    if (*verb == u"overview") return overview(store, today);
    if (*verb == u"list") {
        const auto [words, pairs] = splitPairs(rest);
        const auto limit = takeLimit(pairs, QStringLiteral("list"), &error);
        if (!limit) return failWith(error);
        return list(store, join(words), *limit, today);
    }
    if (*verb == u"search") {
        const auto [words, pairs] = splitPairs(rest);
        const auto limit = takeLimit(pairs, QStringLiteral("search"), &error);
        if (!limit) return failWith(error);
        const QString text = join(words);
        if (text.isEmpty()) return failWith(missing(QStringLiteral("search"), QStringLiteral("some text to look for")));
        QJsonArray hits;
        for (const Hit &hit : search(store, text, *limit)) {
            switch (hit.kind) {
            case Hit::TaskHit: hits.append(QJsonObject{{QStringLiteral("kind"), QStringLiteral("task")}, {QStringLiteral("id"), hit.id}, {QStringLiteral("title"), hit.title}, {QStringLiteral("context"), hit.context}}); break;
            case Hit::ProjectHit: hits.append(QJsonObject{{QStringLiteral("kind"), QStringLiteral("project")}, {QStringLiteral("id"), hit.id}, {QStringLiteral("name"), hit.title}}); break;
            // A label has an id, but nothing takes one: every verb that deals in labels takes the name.
            case Hit::LabelHit: hits.append(QJsonObject{{QStringLiteral("kind"), QStringLiteral("label")}, {QStringLiteral("name"), hit.title}}); break;
            }
        }
        return respond(QStringLiteral("search"), {{QStringLiteral("count"), hits.size()}, {QStringLiteral("hits"), hits}}, false);
    }
    if (*verb == u"show") {
        const QString reference = join(rest);
        if (reference.isEmpty()) return failWith(missing(QStringLiteral("show"), QStringLiteral("a task")));
        const auto id = resolveTask(store, reference, &error);
        if (!id) return failWith(error);
        return respond(QStringLiteral("show"), {{QStringLiteral("task"), taskView(store, *store.task(*id), today, true)}}, false);
    }
    if (*verb == u"add") {
        const QString line = join(rest);
        if (line.isEmpty()) return failWith(missing(QStringLiteral("add"), QStringLiteral("a quick-add line")));
        const QuickAdd parsed = parseQuickAdd(line, today, store.vocabulary());
        if (parsed.title.trimmed().isEmpty())
            return fail(QStringLiteral("bad-value"), QStringLiteral("`%1` is all tokens and no title, so there is nothing to call the task.").arg(line),
                        QStringLiteral("Put the title first: `Email Sam #Work friday`."));
        const TaskId id = store.addFromQuickAdd(parsed, inboxId(), std::nullopt, now);
        return respond(QStringLiteral("added"), {{QStringLiteral("task"), taskView(store, *store.task(id), today)}}, true);
    }
    if (*verb == u"subtask") {
        if (rest.isEmpty()) return failWith(missing(QStringLiteral("subtask"), QStringLiteral("a parent task")));
        const QString parentRef = rest[0];
        const QString line = join(rest.mid(1));
        if (line.isEmpty()) return failWith(missing(QStringLiteral("subtask"), QStringLiteral("a quick-add line for the subtask")));
        const auto parentId = resolveTask(store, parentRef, &error);
        if (!parentId) return failWith(error);
        const QuickAdd parsed = parseQuickAdd(line, today, store.vocabulary());
        if (parsed.title.trimmed().isEmpty())
            return fail(QStringLiteral("bad-value"), QStringLiteral("`%1` is all tokens and no title, so there is nothing to call the subtask.").arg(line));
        const Task *parent = store.task(*parentId);
        const ProjectId project = parent->projectId;
        const std::optional<SectionId> section = parent->sectionId;
        // A subtask shares its parent's project whatever the line said.
        const TaskId id = store.addFromQuickAdd(parsed, project, section, now);
        Task *child = store.taskMut(id);
        child->parentId = *parentId;
        child->projectId = project;
        return respond(QStringLiteral("added"), {{QStringLiteral("task"), taskView(store, *store.task(id), today)}}, true);
    }
    if (*verb == u"complete") {
        const QString reference = join(rest);
        if (reference.isEmpty()) return failWith(missing(QStringLiteral("complete"), QStringLiteral("a task")));
        const auto id = resolveTask(store, reference, &error);
        if (!id) return failWith(error);
        const Completion outcome = *store.completeTask(*id, now, today);
        QJsonObject body{{QStringLiteral("task"), taskView(store, *store.task(*id), today)}};
        switch (outcome.kind) {
        case Completion::Done: body.insert(QStringLiteral("outcome"), QStringLiteral("done")); break;
        // Named for the completion rather than the reschedule: the task is
        // done for today *and* comes back, and the reader must say both.
        case Completion::Rescheduled:
            body.insert(QStringLiteral("outcome"), QStringLiteral("completed-and-repeats"));
            body.insert(QStringLiteral("next_due"), duePhrase(*outcome.due));
            break;
        case Completion::AlreadyDone: body.insert(QStringLiteral("outcome"), QStringLiteral("already-done")); break;
        }
        return respond(QStringLiteral("completed"), body, outcome.kind != Completion::AlreadyDone);
    }
    if (*verb == u"reopen") {
        const QString reference = join(rest);
        if (reference.isEmpty()) return failWith(missing(QStringLiteral("reopen"), QStringLiteral("a task")));
        const auto id = resolveTask(store, reference, &error);
        if (!id) return failWith(error);
        const bool reopened = store.task(*id)->checked;
        store.uncompleteTask(*id, now);
        return respond(QStringLiteral("reopened"), {{QStringLiteral("task"), taskView(store, *store.task(*id), today)}, {QStringLiteral("reopened"), reopened}}, reopened);
    }
    if (*verb == u"delete") {
        const QString reference = join(rest);
        if (reference.isEmpty()) return failWith(missing(QStringLiteral("delete"), QStringLiteral("a task")));
        const auto id = resolveTask(store, reference, &error);
        if (!id) return failWith(error);
        QJsonArray removed;
        const QList<Task> gone = store.removeTask(*id);
        for (const Task &task : gone) removed.append(QJsonObject{{QStringLiteral("id"), task.id}, {QStringLiteral("content"), task.content}});
        return respond(QStringLiteral("deleted"), {{QStringLiteral("count"), static_cast<int>(gone.size())}, {QStringLiteral("removed"), removed}}, true);
    }
    if (*verb == u"update") {
        const auto [words, pairs] = splitPairs(rest);
        const QString reference = join(words);
        if (reference.isEmpty()) return failWith(missing(QStringLiteral("update"), QStringLiteral("a task to change")));
        if (pairs.isEmpty()) return failWith(missing(QStringLiteral("update"), QStringLiteral("at least one `field=value`")));
        return update(store, reference, pairs, now, today);
    }
    if (*verb == u"add-project") {
        const auto [words, pairs] = splitPairs(rest);
        const QString name = join(words);
        if (name.isEmpty()) return failWith(missing(QStringLiteral("add-project"), QStringLiteral("a name")));
        std::optional<ProjectId> parent;
        for (const auto &[key, value] : pairs) {
            if (key != u"parent") return failWith(unknownField(key, QStringLiteral("add-project"), {QStringLiteral("parent")}));
            parent = resolveProject(store, value, &error);
            if (!parent) return failWith(error);
        }
        Project project = Project::create(name.trimmed(), store.nextProjectColor());
        project.parentId = parent;
        const ProjectId id = store.addProject(project);
        return respond(QStringLiteral("project-added"), {{QStringLiteral("project"), projectView(store, *store.project(id))}}, true);
    }
    if (*verb == u"rename-project") {
        if (rest.isEmpty()) return failWith(missing(QStringLiteral("rename-project"), QStringLiteral("a project")));
        const QString name = join(rest.mid(1));
        if (name.isEmpty()) return failWith(missing(QStringLiteral("rename-project"), QStringLiteral("a new name")));
        const auto id = resolveProject(store, rest[0], &error);
        if (!id) return failWith(error);
        if (*id == inboxId()) return fail(QStringLiteral("refused"), QStringLiteral("The Inbox cannot be renamed. It is where anything without a project goes, and every other part of the app names it."));
        store.projectMut(*id)->name = name.trimmed();
        return respond(QStringLiteral("project-renamed"), {{QStringLiteral("project"), projectView(store, *store.project(*id))}}, true);
    }
    if (*verb == u"remove-project") {
        const QString reference = join(rest);
        if (reference.isEmpty()) return failWith(missing(QStringLiteral("remove-project"), QStringLiteral("a project")));
        const auto id = resolveProject(store, reference, &error);
        if (!id) return failWith(error);
        const QString name = store.project(*id)->name;
        const auto removed = store.removeProject(*id);
        if (!removed) return fail(QStringLiteral("refused"), QStringLiteral("The Inbox cannot be deleted. It is where a task with no project of its own lives."),
                                  QStringLiteral("Delete the tasks in it instead, or move them somewhere else."));
        return respond(QStringLiteral("project-removed"), {{QStringLiteral("name"), name}, {QStringLiteral("projects"), static_cast<int>(removed->projects.size())},
                                                           {QStringLiteral("tasks"), static_cast<int>(removed->tasks.size())}}, true);
    }
    return fail(QStringLiteral("unknown-verb"), QStringLiteral("`%1` is not a verb.").arg(verbWord));
}

QString render(const Result &result) {
    if (result.ok && !result.helpText.isEmpty()) return result.helpText;
    QJsonObject envelope;
    if (result.ok) {
        envelope = result.body;
        envelope.insert(QStringLiteral("ok"), true);
        envelope.insert(QStringLiteral("action"), result.action);
    } else {
        envelope = result.error.toJson();
        envelope.insert(QStringLiteral("ok"), false);
    }
    return QString::fromUtf8(QJsonDocument(envelope).toJson(QJsonDocument::Indented)).trimmed();
}

} // namespace planner::agent
