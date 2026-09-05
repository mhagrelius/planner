// The planner's records and every rule that operates on them.
//
// Plain C++ over QtCore: no Qt Quick, no display, no main loop, so every
// rule here is exercised by the ctest suite with nothing but a temporary
// directory. Nothing here reads the clock either: a function whose answer
// depends on today's date takes it as an argument, which is the difference
// between a test and a bug report filed next February.
//
// The on-disk shape is the one the original GTK/Rust app wrote — the same
// keys, the same enum spellings — so a planner.json written by either reads
// in the other.
#pragma once

#include <QDate>
#include <QDateTime>
#include <QJsonObject>
#include <QList>
#include <QString>
#include <QTime>

#include <optional>

namespace planner {

// Identifiers are opaque strings, generated locally and never reused. Each
// record kind gets its own alias so a signature says which it wants.
using TaskId = QString;
using ProjectId = QString;
using SectionId = QString;
using LabelId = QString;
using ReminderId = QString;
using FilterId = QString;

// Mint a fresh id: a nanosecond timestamp plus a process-wide counter, which
// is unique for a single writer and sorts roughly by age when read by hand.
QString newId();
// The Inbox has a reserved id so a task can name it before the store is read.
ProjectId inboxId();

// A day of the week, Monday first, so a weekday set sorts the way people
// list one. Matches QDate::dayOfWeek() - 1.
enum class Weekday { Mon, Tue, Wed, Thu, Fri, Sat, Sun };
int daysFromMonday(Weekday day);
Weekday weekdayOf(const QDate &date);
QString weekdaySerial(Weekday day);          // "Mon", as chrono wrote it
std::optional<Weekday> weekdayFromSerial(const QString &text);

// --- colour and priority -----------------------------------------------------

// A fixed named set rather than a free colour picker: names survive a theme
// switch where a stored hex value does not.
enum class Color { Blue, Teal, Green, Yellow, Orange, Red, Pink, Purple, Brown, Slate };
extern const Color kAllColors[10];
QString colorId(Color color);                // "blue", the stored form
std::optional<Color> colorFromId(const QString &id);
QString colorLabel(Color color);             // "Blue"
// The palette role a colour maps to on Omarchy, so a project dot follows
// the active theme rather than shipping its own hex.
QString colorRole(Color color);
// The least-used colour, so fresh ones are spent before any repeats.
Color leastUsedColor(const QList<Color> &existing);

// Four levels, named as Todoist names them. P4 is the absence of a priority.
enum class Priority { P1, P2, P3, P4 };
extern const Priority kAllPriorities[4];
std::optional<Priority> priorityFromToken(const QString &token);
QString priorityToken(Priority priority);    // "p1"
QString prioritySerial(Priority priority);   // "P1", the stored form
std::optional<Priority> priorityFromSerial(const QString &text);
QString priorityLabel(Priority priority);    // "Urgent"
// The palette role the ring takes; empty for P4, which draws no ring.
QString priorityRole(Priority priority);
bool priorityIsSet(Priority priority);
int priorityRank(Priority priority);         // most urgent first

// --- recurrence --------------------------------------------------------------

enum class Unit { Day, Week, Month, Year };

// When a recurrence stops. `remaining` counts occurrences still to come, not
// counting the one currently due, so it decrements as the rule advances.
struct End {
    enum Kind { Never, OnDate, After } kind = Never;
    QDate date;
    int remaining = 0;
    bool operator==(const End &other) const;
    bool operator!=(const End &other) const { return !(*this == other); }
    static End never() { return {}; }
    static End onDate(const QDate &date) { End e; e.kind = OnDate; e.date = date; return e; }
    static End after(int remaining) { End e; e.kind = After; e.remaining = remaining; return e; }
};

// A repeat rule. `every` steps from the due date; `every!` (fromCompletion)
// steps from the day the task was actually ticked.
struct Recurrence {
    int interval = 1;
    Unit unit = Unit::Day;
    QList<Weekday> weekdays;                 // sorted, unique; Week only
    bool fromCompletion = false;
    End end;

    static Recurrence every(int interval, Unit unit);
    static Recurrence weeklyOn(int interval, const QList<Weekday> &weekdays);
    static Recurrence everyWeekday();
    void setWeekdays(const QList<Weekday> &weekdays);

    // Strictly after `anchor`, ignoring the end condition.
    std::optional<QDate> nextAfter(const QDate &anchor) const;
    // Where a task given only a rule should land: the next named weekday,
    // else today.
    QDate firstOccurrence(const QDate &today) const;
    // The rule written back as the phrase that would produce it.
    QString describe() const;
    // Step past one completion. Returns the next due date and the rule as it
    // now stands, or nothing when the recurrence has run out.
    std::optional<std::pair<QDate, Recurrence>> advance(const QDate &due, const QDate &completedOn) const;

    QJsonObject toJson() const;
    static Recurrence fromJson(const QJsonObject &json);
    bool operator==(const Recurrence &other) const;
    bool operator!=(const Recurrence &other) const { return !(*this == other); }

private:
    std::optional<QDate> nextWeekdayAfter(const QDate &anchor, int interval) const;
    QString body(bool plural) const;
};

// --- due date ----------------------------------------------------------------

// A date, optionally a clock time, optionally repeating. Never an instant:
// "Friday at 09:00" means nine o'clock wherever you are on Friday.
struct Due {
    QDate date;
    std::optional<QTime> time;
    std::optional<Recurrence> recurrence;

    static Due on(const QDate &date) { Due d; d.date = date; return d; }
    static Due at(const QDate &date, const QTime &time) { Due d; d.date = date; d.time = time; return d; }
    Due repeating(const Recurrence &rule) const { Due d = *this; d.recurrence = rule; return d; }
    bool isRecurring() const { return recurrence.has_value(); }
    // Measured in whole days: a task due at 09:00 is not overdue at 09:01.
    bool isOverdue(const QDate &today) const { return date < today; }
    std::optional<Due> advance(const QDate &completedOn) const;

    QJsonObject toJson() const;
    static Due fromJson(const QJsonObject &json);
    bool operator==(const Due &other) const;
    bool operator!=(const Due &other) const { return !(*this == other); }
};

// --- task --------------------------------------------------------------------

struct Trigger {
    enum Kind { Absolute, BeforeDue } kind = BeforeDue;
    QDateTime at;          // Absolute
    qint64 minutes = 0;    // BeforeDue
    bool operator==(const Trigger &other) const;
};

struct Reminder {
    ReminderId id;
    Trigger trigger;
    static Reminder absolute(const QDateTime &at);
    static Reminder beforeDue(qint64 minutes);
    QJsonObject toJson() const;
    static Reminder fromJson(const QJsonObject &json);
    bool operator==(const Reminder &other) const { return id == other.id && trigger == other.trigger; }
};

// What completing a task did. Ticking a recurring task moves it on rather
// than finishing it, and the caller needs to know which happened.
struct Completion {
    enum Kind { Done, Rescheduled, AlreadyDone } kind = Done;
    std::optional<Due> due;   // Rescheduled
};

struct Task {
    TaskId id;
    QString content;
    QString description;
    ProjectId projectId;
    std::optional<SectionId> sectionId;
    std::optional<TaskId> parentId;
    std::optional<Due> due;
    std::optional<QDate> deadline;
    Priority priority = Priority::P4;
    QList<LabelId> labels;
    QList<Reminder> reminders;
    bool pinned = false;
    bool checked = false;
    std::optional<QDateTime> completedAt;
    QDateTime addedAt;
    QDateTime updatedAt;
    int order = 0;

    static Task create(const ProjectId &project, const QString &content, const QDateTime &now);
    void touch(const QDateTime &now) { updatedAt = now; }
    bool isSubtask() const { return parentId.has_value(); }
    bool isOverdue(const QDate &today) const { return !checked && due && due->isOverdue(today); }
    bool isPastDeadline(const QDate &today) const { return !checked && deadline && *deadline < today; }
    bool hasLabel(const LabelId &label) const { return labels.contains(label); }
    void addLabel(const LabelId &label);
    void removeLabel(const LabelId &label);
    Completion complete(const QDateTime &now, const QDate &today);
    void uncomplete(const QDateTime &now);

    QJsonObject toJson() const;
    static Task fromJson(const QJsonObject &json);
    bool operator==(const Task &other) const;
    bool operator!=(const Task &other) const { return !(*this == other); }
};

// --- projects, sections, labels, filters ------------------------------------

enum class ViewStyle { List, Board };
enum class SortBy { Manual, DueDate, Priority, Name, AddedAt };
QString viewStyleSerial(ViewStyle style);
ViewStyle viewStyleFromSerial(const QString &text);
QString sortBySerial(SortBy sort);
SortBy sortByFromSerial(const QString &text);

struct Section {
    SectionId id;
    QString name;
    bool collapsed = false;
    int order = 0;
    static Section create(const QString &name);
    QJsonObject toJson() const;
    static Section fromJson(const QJsonObject &json);
    bool operator==(const Section &other) const;
};

// Sections are stored inside their project: a section has no meaning apart
// from it, and deleting a project must take them with it.
struct Project {
    ProjectId id;
    QString name;
    Color color = Color::Blue;
    QString description;
    std::optional<ProjectId> parentId;
    QList<Section> sections;
    bool isFavorite = false;
    bool isArchived = false;
    bool collapsed = false;
    ViewStyle viewStyle = ViewStyle::List;
    SortBy sortBy = SortBy::Manual;
    bool showCompleted = false;
    int order = 0;

    static Project create(const QString &name, Color color);
    static Project inbox();
    bool isInbox() const { return id == inboxId(); }
    const Section *section(const SectionId &id) const;
    Section *sectionMut(const SectionId &id);
    SectionId addSection(Section section);
    std::optional<Section> removeSection(const SectionId &id);
    void restoreSection(const Section &section) { sections.append(section); }
    QList<const Section *> sectionsOrdered() const;

    QJsonObject toJson() const;
    static Project fromJson(const QJsonObject &json);
    bool operator==(const Project &other) const;
};

struct Label {
    LabelId id;
    QString name;   // unique, case-insensitively
    Color color = Color::Blue;
    bool isFavorite = false;
    int order = 0;
    static Label create(const QString &name, Color color);
    bool matchesName(const QString &name) const { return this->name.compare(name, Qt::CaseInsensitive) == 0; }
    QJsonObject toJson() const;
    static Label fromJson(const QJsonObject &json);
    bool operator==(const Label &other) const;
};

// A query the user saved and named. Stored as text, not a parsed tree: it has
// to survive a schema that learns new terms.
struct SavedFilter {
    FilterId id;
    QString name;
    QString query;
    Color color = Color::Blue;
    int order = 0;
    static SavedFilter create(const QString &name, const QString &query, Color color);
    QJsonObject toJson() const;
    static SavedFilter fromJson(const QJsonObject &json);
    bool operator==(const SavedFilter &other) const;
};

// --- JSON helpers shared by the records ---------------------------------------

QString dateSerial(const QDate &date);                        // 2026-07-30
std::optional<QDate> dateFromSerial(const QString &text);
QString timeSerial(const QTime &time);                        // 09:00:00
std::optional<QTime> timeFromSerial(const QString &text);
QString instantSerial(const QDateTime &instant);              // RFC 3339, UTC
std::optional<QDateTime> instantFromSerial(const QString &text);

} // namespace planner
