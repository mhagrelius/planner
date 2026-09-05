// The filter query language.
//
// Every view in the app is this: Today is `due: today | overdue`, Pinned is
// `pinned`, a project is `#Work`. One evaluator behind the built-in views and
// the user's saved filters means there is only one thing to get right.
//
//   query := or (',' or)*      a comma renders as separate lists
//   or    := and ('|' and)*
//   and   := unary ('&' unary)*
//   unary := '!' unary | '(' query ')' | term
//
// Completed tasks are excluded unless the query mentions `completed`.
#pragma once

#include "model.h"

#include <memory>

namespace planner {

class Store;

struct DateFilter {
    enum Kind { On, Before, After } kind = On;
    QString phrase;   // resolved at evaluation time, so `today` means today whenever it runs
    bool operator==(const DateFilter &other) const { return kind == other.kind && phrase == other.phrase; }
};

struct Term {
    enum Kind { Due, Deadline, Overdue, NoDate, NoDeadline, Recurring, Subtask, Pinned, Completed,
                PriorityIs, LabelIs, NoLabels, ProjectIs, SectionIs, Search } kind = Overdue;
    DateFilter date;               // Due, Deadline
    Priority priority = Priority::P4;
    QString name;                  // LabelIs, ProjectIs, SectionIs, Search
    bool includeSubprojects = false;
    bool operator==(const Term &other) const;
};

struct Filter {
    enum Kind { All, TermIs, Not, And, Or } kind = All;
    Term term;
    std::shared_ptr<Filter> left, right;   // Not uses left
    static Filter all() { return {}; }
    static Filter of(const Term &term) { Filter f; f.kind = TermIs; f.term = term; return f; }
    static Filter negate(const Filter &inner);
    static Filter both(const Filter &a, const Filter &b);
    static Filter either(const Filter &a, const Filter &b);
    bool matches(const Task &task, const Store &store, const QDate &today) const;
    bool operator==(const Filter &other) const;
};

struct QueryError {
    QString message;
    int at = 0;   // offset into the query, for underlining it
};

class Query {
public:
    static std::optional<Query> parse(const QString &source, QueryError *error = nullptr);
    static Query all();
    const QList<Filter> &lists() const { return m_lists; }
    bool includesCompleted() const { return m_includesCompleted; }
    // Any list matching is a match. A completed task never matches unless the
    // query asked for completed tasks.
    bool matches(const Task &task, const Store &store, const QDate &today) const;
    // Every task matching, in store order. Sorting is the view's business.
    QList<const Task *> run(const Store &store, const QDate &today) const;

private:
    QList<Filter> m_lists;
    bool m_includesCompleted = false;
};

} // namespace planner
