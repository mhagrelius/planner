#include "model.h"

#include <QJsonArray>
#include <QLocale>

#include <algorithm>
#include <atomic>
#include <chrono>

namespace planner {

// --- ids ---------------------------------------------------------------------

QString newId() {
    static std::atomic<quint64> counter{0};
    const auto nanos = std::chrono::duration_cast<std::chrono::nanoseconds>(
                           std::chrono::system_clock::now().time_since_epoch())
                           .count();
    const quint64 seq = counter.fetch_add(1);
    return QString::number(static_cast<quint64>(nanos), 16) + QLatin1Char('-') + QString::number(seq, 16);
}

ProjectId inboxId() { return QStringLiteral("inbox"); }

// --- weekdays ----------------------------------------------------------------

int daysFromMonday(Weekday day) { return static_cast<int>(day); }
Weekday weekdayOf(const QDate &date) { return static_cast<Weekday>(date.dayOfWeek() - 1); }

QString weekdaySerial(Weekday day) {
    static const char *names[] = {"Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"};
    return QString::fromLatin1(names[daysFromMonday(day)]);
}

std::optional<Weekday> weekdayFromSerial(const QString &text) {
    const QString lowered = text.toLower();
    static const char *names[] = {"mon", "tue", "wed", "thu", "fri", "sat", "sun"};
    for (int i = 0; i < 7; ++i)
        if (lowered.startsWith(QLatin1String(names[i]))) return static_cast<Weekday>(i);
    return std::nullopt;
}

// --- colour ------------------------------------------------------------------

const Color kAllColors[10] = {Color::Blue, Color::Teal, Color::Green, Color::Yellow, Color::Orange,
                              Color::Red, Color::Pink, Color::Purple, Color::Brown, Color::Slate};

QString colorId(Color color) {
    static const char *ids[] = {"blue", "teal", "green", "yellow", "orange", "red", "pink", "purple", "brown", "slate"};
    return QString::fromLatin1(ids[static_cast<int>(color)]);
}

std::optional<Color> colorFromId(const QString &id) {
    for (Color color : kAllColors)
        if (colorId(color) == id) return color;
    return std::nullopt;
}

QString colorLabel(Color color) {
    static const char *labels[] = {"Blue", "Teal", "Green", "Yellow", "Orange", "Red", "Pink", "Purple", "Brown", "Slate"};
    return QString::fromLatin1(labels[static_cast<int>(color)]);
}

QString colorRole(Color color) {
    static const char *roles[] = {"blue", "teal", "positive", "caution", "warning", "negative", "pink", "violet", "cautionAlt", "blueGray"};
    return QString::fromLatin1(roles[static_cast<int>(color)]);
}

Color leastUsedColor(const QList<Color> &existing) {
    Color best = Color::Blue;
    int bestCount = -1;
    for (Color color : kAllColors) {
        const int count = static_cast<int>(std::count(existing.begin(), existing.end(), color));
        if (bestCount < 0 || count < bestCount) {
            best = color;
            bestCount = count;
        }
    }
    return best;
}

// --- priority ----------------------------------------------------------------

const Priority kAllPriorities[4] = {Priority::P1, Priority::P2, Priority::P3, Priority::P4};

std::optional<Priority> priorityFromToken(const QString &token) {
    const QString lowered = token.toLower();
    if (lowered == QLatin1String("p1")) return Priority::P1;
    if (lowered == QLatin1String("p2")) return Priority::P2;
    if (lowered == QLatin1String("p3")) return Priority::P3;
    if (lowered == QLatin1String("p4")) return Priority::P4;
    return std::nullopt;
}

QString priorityToken(Priority priority) { return QStringLiteral("p%1").arg(static_cast<int>(priority) + 1); }
QString prioritySerial(Priority priority) { return QStringLiteral("P%1").arg(static_cast<int>(priority) + 1); }

std::optional<Priority> priorityFromSerial(const QString &text) { return priorityFromToken(text); }

QString priorityLabel(Priority priority) {
    static const char *labels[] = {"Urgent", "High", "Medium", "None"};
    return QString::fromLatin1(labels[static_cast<int>(priority)]);
}

QString priorityRole(Priority priority) {
    switch (priority) {
    case Priority::P1: return QStringLiteral("negative");
    case Priority::P2: return QStringLiteral("warning");
    case Priority::P3: return QStringLiteral("info");
    case Priority::P4: return QString();
    }
    return QString();
}

bool priorityIsSet(Priority priority) { return priority != Priority::P4; }
int priorityRank(Priority priority) { return static_cast<int>(priority); }

// --- recurrence --------------------------------------------------------------

bool End::operator==(const End &other) const {
    if (kind != other.kind) return false;
    switch (kind) {
    case Never: return true;
    case OnDate: return date == other.date;
    case After: return remaining == other.remaining;
    }
    return false;
}

Recurrence Recurrence::every(int interval, Unit unit) {
    Recurrence rule;
    rule.interval = std::max(1, interval);
    rule.unit = unit;
    return rule;
}

Recurrence Recurrence::weeklyOn(int interval, const QList<Weekday> &weekdays) {
    Recurrence rule = every(interval, Unit::Week);
    rule.setWeekdays(weekdays);
    return rule;
}

Recurrence Recurrence::everyWeekday() {
    return weeklyOn(1, {Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri});
}

void Recurrence::setWeekdays(const QList<Weekday> &days) {
    QList<Weekday> sorted = days;
    std::sort(sorted.begin(), sorted.end(), [](Weekday a, Weekday b) { return daysFromMonday(a) < daysFromMonday(b); });
    sorted.erase(std::unique(sorted.begin(), sorted.end()), sorted.end());
    weekdays = sorted;
}

static std::optional<QDate> addMonthsClamped(const QDate &date, int months) {
    // QDate::addMonths clamps to the end of the target month, as chrono did.
    const QDate next = date.addMonths(months);
    if (!next.isValid()) return std::nullopt;
    return next;
}

std::optional<QDate> Recurrence::nextAfter(const QDate &anchor) const {
    const int step = std::max(1, interval);
    switch (unit) {
    case Unit::Day: return anchor.addDays(step);
    case Unit::Week:
        if (weekdays.isEmpty()) return anchor.addDays(qint64(step) * 7);
        return nextWeekdayAfter(anchor, step);
    case Unit::Month: return addMonthsClamped(anchor, step);
    case Unit::Year: return addMonthsClamped(anchor, step * 12);
    }
    return std::nullopt;
}

// The next listed day later in the same week, or else the first listed day
// `interval` weeks on, measured from the start of the anchor's week so a
// multi-day rule cannot creep forward as it wraps.
std::optional<QDate> Recurrence::nextWeekdayAfter(const QDate &anchor, int interval) const {
    const int anchorIndex = daysFromMonday(weekdayOf(anchor));
    for (Weekday day : weekdays) {
        const int index = daysFromMonday(day);
        if (index > anchorIndex) return anchor.addDays(index - anchorIndex);
    }
    const QDate weekStart = anchor.addDays(-anchorIndex);
    const int first = daysFromMonday(weekdays.first());
    return weekStart.addDays(qint64(interval) * 7 + first);
}

QDate Recurrence::firstOccurrence(const QDate &today) const {
    if (unit != Unit::Week || weekdays.isEmpty()) return today;
    const int current = daysFromMonday(weekdayOf(today));
    int best = 7;
    for (Weekday day : weekdays) {
        const int ahead = ((daysFromMonday(day) - current) % 7 + 7) % 7;
        best = std::min(best, ahead);
    }
    return today.addDays(best);
}

static QString weekdayName(Weekday day) {
    static const char *names[] = {"monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"};
    return QString::fromLatin1(names[daysFromMonday(day)]);
}

static QString joinDays(const QList<Weekday> &days) {
    if (days.isEmpty()) return QString();
    if (days.size() == 1) return weekdayName(days.first());
    QStringList rest;
    for (int i = 0; i < days.size() - 1; ++i) rest << weekdayName(days.at(i));
    return rest.join(QStringLiteral(", ")) + QStringLiteral(" and ") + weekdayName(days.last());
}

QString Recurrence::body(bool plural) const {
    if (unit == Unit::Week && !weekdays.isEmpty()) {
        if (weekdays == everyWeekday().weekdays) return QStringLiteral("weekday");
        return joinDays(weekdays);
    }
    QString name;
    switch (unit) {
    case Unit::Day: name = QStringLiteral("day"); break;
    case Unit::Week: name = QStringLiteral("week"); break;
    case Unit::Month: name = QStringLiteral("month"); break;
    case Unit::Year: name = QStringLiteral("year"); break;
    }
    return plural ? name + QLatin1Char('s') : name;
}

QString Recurrence::describe() const {
    QString text = fromCompletion ? QStringLiteral("every!") : QStringLiteral("every");
    const int step = std::max(1, interval);
    if (step == 2)
        text += QStringLiteral(" other");
    else if (step > 2)
        text += QStringLiteral(" %1").arg(step);
    text += QLatin1Char(' ');
    text += body(interval > 2);
    switch (end.kind) {
    case End::Never: break;
    // The year is always written so the phrase means the same thing in January.
    case End::OnDate: text += QStringLiteral(" until %1 %2 %3").arg(end.date.day()).arg(QLocale::c().monthName(end.date.month(), QLocale::ShortFormat)).arg(end.date.year()); break;
    case End::After: text += QStringLiteral(" x%1").arg(end.remaining + 1); break;
    }
    return text;
}

std::optional<std::pair<QDate, Recurrence>> Recurrence::advance(const QDate &due, const QDate &completedOn) const {
    // Never step backwards: completing early must not resurrect a past occurrence.
    const QDate anchor = fromCompletion ? std::max(completedOn, due) : due;
    const auto next = nextAfter(anchor);
    if (!next) return std::nullopt;
    switch (end.kind) {
    case End::Never: return std::make_pair(*next, *this);
    case End::OnDate:
        if (*next > end.date) return std::nullopt;
        return std::make_pair(*next, *this);
    case End::After:
        if (end.remaining == 0) return std::nullopt;
        {
            Recurrence rule = *this;
            rule.end = End::after(end.remaining - 1);
            return std::make_pair(*next, rule);
        }
    }
    return std::nullopt;
}

static QString unitSerial(Unit unit) {
    switch (unit) {
    case Unit::Day: return QStringLiteral("day");
    case Unit::Week: return QStringLiteral("week");
    case Unit::Month: return QStringLiteral("month");
    case Unit::Year: return QStringLiteral("year");
    }
    return QStringLiteral("day");
}

static Unit unitFromSerial(const QString &text) {
    if (text == QLatin1String("week")) return Unit::Week;
    if (text == QLatin1String("month")) return Unit::Month;
    if (text == QLatin1String("year")) return Unit::Year;
    return Unit::Day;
}

QJsonObject Recurrence::toJson() const {
    QJsonObject json;
    json.insert(QStringLiteral("interval"), interval);
    json.insert(QStringLiteral("unit"), unitSerial(unit));
    if (!weekdays.isEmpty()) {
        QJsonArray days;
        for (Weekday day : weekdays) days.append(weekdaySerial(day));
        json.insert(QStringLiteral("weekdays"), days);
    }
    if (fromCompletion) json.insert(QStringLiteral("from_completion"), true);
    if (end.kind == End::OnDate)
        json.insert(QStringLiteral("end"), QJsonObject{{QStringLiteral("kind"), QStringLiteral("on-date")}, {QStringLiteral("date"), dateSerial(end.date)}});
    else if (end.kind == End::After)
        json.insert(QStringLiteral("end"), QJsonObject{{QStringLiteral("kind"), QStringLiteral("after")}, {QStringLiteral("remaining"), end.remaining}});
    return json;
}

Recurrence Recurrence::fromJson(const QJsonObject &json) {
    Recurrence rule;
    rule.interval = json.value(QStringLiteral("interval")).toInt(1);
    rule.unit = unitFromSerial(json.value(QStringLiteral("unit")).toString());
    QList<Weekday> days;
    for (const QJsonValue &value : json.value(QStringLiteral("weekdays")).toArray())
        if (const auto day = weekdayFromSerial(value.toString())) days.append(*day);
    rule.setWeekdays(days);
    rule.fromCompletion = json.value(QStringLiteral("from_completion")).toBool(false);
    const QJsonObject end = json.value(QStringLiteral("end")).toObject();
    const QString kind = end.value(QStringLiteral("kind")).toString();
    if (kind == QLatin1String("on-date")) {
        if (const auto date = dateFromSerial(end.value(QStringLiteral("date")).toString())) rule.end = End::onDate(*date);
    } else if (kind == QLatin1String("after")) {
        rule.end = End::after(end.value(QStringLiteral("remaining")).toInt(0));
    }
    return rule;
}

bool Recurrence::operator==(const Recurrence &other) const {
    return interval == other.interval && unit == other.unit && weekdays == other.weekdays
        && fromCompletion == other.fromCompletion && end == other.end;
}

// --- due ---------------------------------------------------------------------

std::optional<Due> Due::advance(const QDate &completedOn) const {
    if (!recurrence) return std::nullopt;
    const auto next = recurrence->advance(date, completedOn);
    if (!next) return std::nullopt;
    Due advanced;
    advanced.date = next->first;
    advanced.time = time;   // a recurrence steps dates, never times
    advanced.recurrence = next->second;
    return advanced;
}

QJsonObject Due::toJson() const {
    QJsonObject json;
    json.insert(QStringLiteral("date"), dateSerial(date));
    if (time) json.insert(QStringLiteral("time"), timeSerial(*time));
    if (recurrence) json.insert(QStringLiteral("recurrence"), recurrence->toJson());
    return json;
}

Due Due::fromJson(const QJsonObject &json) {
    Due due;
    due.date = dateFromSerial(json.value(QStringLiteral("date")).toString()).value_or(QDate());
    if (json.contains(QStringLiteral("time"))) due.time = timeFromSerial(json.value(QStringLiteral("time")).toString());
    if (json.contains(QStringLiteral("recurrence"))) due.recurrence = Recurrence::fromJson(json.value(QStringLiteral("recurrence")).toObject());
    return due;
}

bool Due::operator==(const Due &other) const {
    return date == other.date && time == other.time && recurrence == other.recurrence;
}

// --- task --------------------------------------------------------------------

bool Trigger::operator==(const Trigger &other) const {
    if (kind != other.kind) return false;
    return kind == Absolute ? at == other.at : minutes == other.minutes;
}

Reminder Reminder::absolute(const QDateTime &at) {
    Reminder reminder;
    reminder.id = newId();
    reminder.trigger.kind = Trigger::Absolute;
    reminder.trigger.at = at.toUTC();
    return reminder;
}

Reminder Reminder::beforeDue(qint64 minutes) {
    Reminder reminder;
    reminder.id = newId();
    reminder.trigger.kind = Trigger::BeforeDue;
    reminder.trigger.minutes = minutes;
    return reminder;
}

QJsonObject Reminder::toJson() const {
    QJsonObject trigger;
    if (this->trigger.kind == Trigger::Absolute) {
        trigger.insert(QStringLiteral("kind"), QStringLiteral("absolute"));
        trigger.insert(QStringLiteral("at"), instantSerial(this->trigger.at));
    } else {
        trigger.insert(QStringLiteral("kind"), QStringLiteral("before-due"));
        trigger.insert(QStringLiteral("minutes"), static_cast<double>(this->trigger.minutes));
    }
    return QJsonObject{{QStringLiteral("id"), id}, {QStringLiteral("trigger"), trigger}};
}

Reminder Reminder::fromJson(const QJsonObject &json) {
    Reminder reminder;
    reminder.id = json.value(QStringLiteral("id")).toString();
    const QJsonObject trigger = json.value(QStringLiteral("trigger")).toObject();
    if (trigger.value(QStringLiteral("kind")).toString() == QLatin1String("absolute")) {
        reminder.trigger.kind = Trigger::Absolute;
        reminder.trigger.at = instantFromSerial(trigger.value(QStringLiteral("at")).toString()).value_or(QDateTime());
    } else {
        reminder.trigger.kind = Trigger::BeforeDue;
        reminder.trigger.minutes = static_cast<qint64>(trigger.value(QStringLiteral("minutes")).toDouble());
    }
    return reminder;
}

Task Task::create(const ProjectId &project, const QString &content, const QDateTime &now) {
    Task task;
    task.id = newId();
    task.content = content;
    task.projectId = project;
    task.addedAt = now.toUTC();
    task.updatedAt = now.toUTC();
    return task;
}

void Task::addLabel(const LabelId &label) {
    if (!hasLabel(label)) labels.append(label);
}

void Task::removeLabel(const LabelId &label) { labels.removeAll(label); }

Completion Task::complete(const QDateTime &now, const QDate &today) {
    if (checked) return Completion{Completion::AlreadyDone, std::nullopt};
    if (due) {
        if (const auto next = due->advance(today)) {
            due = *next;
            touch(now);
            return Completion{Completion::Rescheduled, *next};
        }
    }
    checked = true;
    completedAt = now.toUTC();
    // A recurrence that has run out must not sit on the completed task:
    // reopening it would otherwise resurrect a rule that is finished.
    if (due) due->recurrence.reset();
    touch(now);
    return Completion{Completion::Done, std::nullopt};
}

void Task::uncomplete(const QDateTime &now) {
    if (!checked) return;
    checked = false;
    completedAt.reset();
    touch(now);
}

QJsonObject Task::toJson() const {
    QJsonObject json;
    json.insert(QStringLiteral("id"), id);
    json.insert(QStringLiteral("content"), content);
    if (!description.isEmpty()) json.insert(QStringLiteral("description"), description);
    json.insert(QStringLiteral("project_id"), projectId);
    if (sectionId) json.insert(QStringLiteral("section_id"), *sectionId);
    if (parentId) json.insert(QStringLiteral("parent_id"), *parentId);
    if (due) json.insert(QStringLiteral("due"), due->toJson());
    if (deadline) json.insert(QStringLiteral("deadline"), dateSerial(*deadline));
    json.insert(QStringLiteral("priority"), prioritySerial(priority));
    if (!labels.isEmpty()) json.insert(QStringLiteral("labels"), QJsonArray::fromStringList(labels));
    if (!reminders.isEmpty()) {
        QJsonArray array;
        for (const Reminder &reminder : reminders) array.append(reminder.toJson());
        json.insert(QStringLiteral("reminders"), array);
    }
    if (pinned) json.insert(QStringLiteral("pinned"), true);
    if (checked) json.insert(QStringLiteral("checked"), true);
    if (completedAt) json.insert(QStringLiteral("completed_at"), instantSerial(*completedAt));
    json.insert(QStringLiteral("added_at"), instantSerial(addedAt));
    json.insert(QStringLiteral("updated_at"), instantSerial(updatedAt));
    json.insert(QStringLiteral("order"), order);
    return json;
}

Task Task::fromJson(const QJsonObject &json) {
    Task task;
    task.id = json.value(QStringLiteral("id")).toString();
    task.content = json.value(QStringLiteral("content")).toString();
    task.description = json.value(QStringLiteral("description")).toString();
    task.projectId = json.value(QStringLiteral("project_id")).toString();
    if (json.contains(QStringLiteral("section_id")) && !json.value(QStringLiteral("section_id")).isNull())
        task.sectionId = json.value(QStringLiteral("section_id")).toString();
    if (json.contains(QStringLiteral("parent_id")) && !json.value(QStringLiteral("parent_id")).isNull())
        task.parentId = json.value(QStringLiteral("parent_id")).toString();
    if (json.value(QStringLiteral("due")).isObject()) task.due = Due::fromJson(json.value(QStringLiteral("due")).toObject());
    if (json.contains(QStringLiteral("deadline"))) task.deadline = dateFromSerial(json.value(QStringLiteral("deadline")).toString());
    task.priority = priorityFromSerial(json.value(QStringLiteral("priority")).toString()).value_or(Priority::P4);
    for (const QJsonValue &value : json.value(QStringLiteral("labels")).toArray()) task.labels.append(value.toString());
    for (const QJsonValue &value : json.value(QStringLiteral("reminders")).toArray()) task.reminders.append(Reminder::fromJson(value.toObject()));
    task.pinned = json.value(QStringLiteral("pinned")).toBool(false);
    task.checked = json.value(QStringLiteral("checked")).toBool(false);
    if (json.contains(QStringLiteral("completed_at"))) task.completedAt = instantFromSerial(json.value(QStringLiteral("completed_at")).toString());
    task.addedAt = instantFromSerial(json.value(QStringLiteral("added_at")).toString()).value_or(QDateTime());
    task.updatedAt = instantFromSerial(json.value(QStringLiteral("updated_at")).toString()).value_or(task.addedAt);
    task.order = json.value(QStringLiteral("order")).toInt(0);
    return task;
}

bool Task::operator==(const Task &other) const {
    return id == other.id && content == other.content && description == other.description && projectId == other.projectId
        && sectionId == other.sectionId && parentId == other.parentId && due == other.due && deadline == other.deadline
        && priority == other.priority && labels == other.labels && reminders == other.reminders && pinned == other.pinned
        && checked == other.checked && completedAt == other.completedAt && addedAt == other.addedAt
        && updatedAt == other.updatedAt && order == other.order;
}

// --- projects ----------------------------------------------------------------

QString viewStyleSerial(ViewStyle style) { return style == ViewStyle::Board ? QStringLiteral("board") : QStringLiteral("list"); }
ViewStyle viewStyleFromSerial(const QString &text) { return text == QLatin1String("board") ? ViewStyle::Board : ViewStyle::List; }

QString sortBySerial(SortBy sort) {
    switch (sort) {
    case SortBy::Manual: return QStringLiteral("manual");
    case SortBy::DueDate: return QStringLiteral("due-date");
    case SortBy::Priority: return QStringLiteral("priority");
    case SortBy::Name: return QStringLiteral("name");
    case SortBy::AddedAt: return QStringLiteral("added-at");
    }
    return QStringLiteral("manual");
}

SortBy sortByFromSerial(const QString &text) {
    if (text == QLatin1String("due-date")) return SortBy::DueDate;
    if (text == QLatin1String("priority")) return SortBy::Priority;
    if (text == QLatin1String("name")) return SortBy::Name;
    if (text == QLatin1String("added-at")) return SortBy::AddedAt;
    return SortBy::Manual;
}

Section Section::create(const QString &name) {
    Section section;
    section.id = newId();
    section.name = name;
    return section;
}

QJsonObject Section::toJson() const {
    QJsonObject json{{QStringLiteral("id"), id}, {QStringLiteral("name"), name}, {QStringLiteral("order"), order}};
    if (collapsed) json.insert(QStringLiteral("collapsed"), true);
    return json;
}

Section Section::fromJson(const QJsonObject &json) {
    Section section;
    section.id = json.value(QStringLiteral("id")).toString();
    section.name = json.value(QStringLiteral("name")).toString();
    section.collapsed = json.value(QStringLiteral("collapsed")).toBool(false);
    section.order = json.value(QStringLiteral("order")).toInt(0);
    return section;
}

bool Section::operator==(const Section &other) const {
    return id == other.id && name == other.name && collapsed == other.collapsed && order == other.order;
}

Project Project::create(const QString &name, Color color) {
    Project project;
    project.id = newId();
    project.name = name;
    project.color = color;
    return project;
}

Project Project::inbox() {
    Project project = create(QStringLiteral("Inbox"), Color::Slate);
    project.id = inboxId();
    project.order = -1;   // always at the top of the sidebar
    return project;
}

const Section *Project::section(const SectionId &id) const {
    for (const Section &section : sections)
        if (section.id == id) return &section;
    return nullptr;
}

Section *Project::sectionMut(const SectionId &id) {
    for (Section &section : sections)
        if (section.id == id) return &section;
    return nullptr;
}

SectionId Project::addSection(Section section) {
    int max = -1;
    for (const Section &existing : sections) max = std::max(max, existing.order);
    section.order = max + 1;
    sections.append(section);
    return section.id;
}

std::optional<Section> Project::removeSection(const SectionId &id) {
    for (int i = 0; i < sections.size(); ++i) {
        if (sections.at(i).id == id) {
            Section removed = sections.at(i);
            sections.removeAt(i);
            return removed;
        }
    }
    return std::nullopt;
}

QList<const Section *> Project::sectionsOrdered() const {
    QList<const Section *> ordered;
    for (const Section &section : sections) ordered.append(&section);
    std::stable_sort(ordered.begin(), ordered.end(), [](const Section *a, const Section *b) { return a->order < b->order; });
    return ordered;
}

QJsonObject Project::toJson() const {
    QJsonObject json;
    json.insert(QStringLiteral("id"), id);
    json.insert(QStringLiteral("name"), name);
    json.insert(QStringLiteral("color"), colorId(color));
    if (!description.isEmpty()) json.insert(QStringLiteral("description"), description);
    if (parentId) json.insert(QStringLiteral("parent_id"), *parentId);
    if (!sections.isEmpty()) {
        QJsonArray array;
        for (const Section &section : sections) array.append(section.toJson());
        json.insert(QStringLiteral("sections"), array);
    }
    if (isFavorite) json.insert(QStringLiteral("is_favorite"), true);
    if (isArchived) json.insert(QStringLiteral("is_archived"), true);
    if (collapsed) json.insert(QStringLiteral("collapsed"), true);
    json.insert(QStringLiteral("view_style"), viewStyleSerial(viewStyle));
    json.insert(QStringLiteral("sort_by"), sortBySerial(sortBy));
    if (showCompleted) json.insert(QStringLiteral("show_completed"), true);
    json.insert(QStringLiteral("order"), order);
    return json;
}

Project Project::fromJson(const QJsonObject &json) {
    Project project;
    project.id = json.value(QStringLiteral("id")).toString();
    project.name = json.value(QStringLiteral("name")).toString();
    project.color = colorFromId(json.value(QStringLiteral("color")).toString()).value_or(Color::Blue);
    project.description = json.value(QStringLiteral("description")).toString();
    if (json.contains(QStringLiteral("parent_id")) && !json.value(QStringLiteral("parent_id")).isNull())
        project.parentId = json.value(QStringLiteral("parent_id")).toString();
    for (const QJsonValue &value : json.value(QStringLiteral("sections")).toArray()) project.sections.append(Section::fromJson(value.toObject()));
    project.isFavorite = json.value(QStringLiteral("is_favorite")).toBool(false);
    project.isArchived = json.value(QStringLiteral("is_archived")).toBool(false);
    project.collapsed = json.value(QStringLiteral("collapsed")).toBool(false);
    project.viewStyle = viewStyleFromSerial(json.value(QStringLiteral("view_style")).toString());
    project.sortBy = sortByFromSerial(json.value(QStringLiteral("sort_by")).toString());
    project.showCompleted = json.value(QStringLiteral("show_completed")).toBool(false);
    project.order = json.value(QStringLiteral("order")).toInt(0);
    return project;
}

bool Project::operator==(const Project &other) const {
    return id == other.id && name == other.name && color == other.color && description == other.description
        && parentId == other.parentId && sections == other.sections && isFavorite == other.isFavorite
        && isArchived == other.isArchived && collapsed == other.collapsed && viewStyle == other.viewStyle
        && sortBy == other.sortBy && showCompleted == other.showCompleted && order == other.order;
}

Label Label::create(const QString &name, Color color) {
    Label label;
    label.id = newId();
    label.name = name;
    label.color = color;
    return label;
}

QJsonObject Label::toJson() const {
    QJsonObject json{{QStringLiteral("id"), id}, {QStringLiteral("name"), name}, {QStringLiteral("color"), colorId(color)}, {QStringLiteral("order"), order}};
    if (isFavorite) json.insert(QStringLiteral("is_favorite"), true);
    return json;
}

Label Label::fromJson(const QJsonObject &json) {
    Label label;
    label.id = json.value(QStringLiteral("id")).toString();
    label.name = json.value(QStringLiteral("name")).toString();
    label.color = colorFromId(json.value(QStringLiteral("color")).toString()).value_or(Color::Blue);
    label.isFavorite = json.value(QStringLiteral("is_favorite")).toBool(false);
    label.order = json.value(QStringLiteral("order")).toInt(0);
    return label;
}

bool Label::operator==(const Label &other) const {
    return id == other.id && name == other.name && color == other.color && isFavorite == other.isFavorite && order == other.order;
}

SavedFilter SavedFilter::create(const QString &name, const QString &query, Color color) {
    SavedFilter filter;
    filter.id = newId();
    filter.name = name;
    filter.query = query;
    filter.color = color;
    return filter;
}

QJsonObject SavedFilter::toJson() const {
    return QJsonObject{{QStringLiteral("id"), id}, {QStringLiteral("name"), name}, {QStringLiteral("query"), query},
                       {QStringLiteral("color"), colorId(color)}, {QStringLiteral("order"), order}};
}

SavedFilter SavedFilter::fromJson(const QJsonObject &json) {
    SavedFilter filter;
    filter.id = json.value(QStringLiteral("id")).toString();
    filter.name = json.value(QStringLiteral("name")).toString();
    filter.query = json.value(QStringLiteral("query")).toString();
    filter.color = colorFromId(json.value(QStringLiteral("color")).toString()).value_or(Color::Blue);
    filter.order = json.value(QStringLiteral("order")).toInt(0);
    return filter;
}

bool SavedFilter::operator==(const SavedFilter &other) const {
    return id == other.id && name == other.name && query == other.query && color == other.color && order == other.order;
}

// --- serial forms ------------------------------------------------------------

QString dateSerial(const QDate &date) { return date.toString(Qt::ISODate); }

std::optional<QDate> dateFromSerial(const QString &text) {
    const QDate date = QDate::fromString(text, Qt::ISODate);
    if (!date.isValid()) return std::nullopt;
    return date;
}

QString timeSerial(const QTime &time) { return time.toString(QStringLiteral("HH:mm:ss")); }

std::optional<QTime> timeFromSerial(const QString &text) {
    // chrono writes HH:MM:SS, with a fraction only when there is one.
    const QString head = text.section(QLatin1Char('.'), 0, 0);
    QTime time = QTime::fromString(head, QStringLiteral("HH:mm:ss"));
    if (!time.isValid()) time = QTime::fromString(head, QStringLiteral("HH:mm"));
    if (!time.isValid()) return std::nullopt;
    return time;
}

QString instantSerial(const QDateTime &instant) {
    return instant.toUTC().toString(QStringLiteral("yyyy-MM-ddTHH:mm:ss.zzz")) + QLatin1Char('Z');
}

std::optional<QDateTime> instantFromSerial(const QString &text) {
    if (text.isEmpty()) return std::nullopt;
    // chrono writes up to nine fractional digits; Qt reads three. Trim the
    // fraction to milliseconds before parsing rather than rejecting the file.
    QString normalised = text;
    const int dot = normalised.indexOf(QLatin1Char('.'));
    if (dot >= 0) {
        int end = dot + 1;
        while (end < normalised.size() && normalised.at(end).isDigit()) ++end;
        const QString fraction = normalised.mid(dot + 1, end - dot - 1).left(3).leftJustified(3, QLatin1Char('0'));
        normalised = normalised.left(dot + 1) + fraction + normalised.mid(end);
    }
    QDateTime instant = QDateTime::fromString(normalised, Qt::ISODateWithMs);
    if (!instant.isValid()) return std::nullopt;
    return instant.toUTC();
}

} // namespace planner
