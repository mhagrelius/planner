// The interface an assistant drives the planner through: `planner agent <verb>`.
//
// It adds no vocabulary. A task is created from the same quick-add line the
// prompt parses, a list is the same filter query the rail runs. What is new
// is only the shape of the answers — names where the records hold ids — and
// the rules for turning a thing the user *said* into the record they meant.
// It refuses rather than guesses, never claims more than happened, and reads
// neither the clock nor the filesystem.
#pragma once

#include "model.h"

#include <QJsonObject>
#include <QStringList>

namespace planner {

class Store;

namespace agent {

struct Argument {
    const char *name;
    bool required;
    const char *description;
};

struct Verb {
    const char *name;
    QList<const char *> aliases;
    const char *usage;
    const char *summary;
    bool mutates;
    QList<Argument> arguments;
    const char *returns;
    QList<const char *> examples;
};

const QList<Verb> &verbs();
std::optional<QString> canonicalVerb(const QString &word);
QStringList verbNames();
QString helpOverview();
std::optional<QString> helpForVerb(const QString &name);

struct Candidate {
    QString id;
    QString name;
    QString context;
};

// Why a command did not run. `kind` is a stable kebab-case word for code to
// branch on; `message` is a whole sentence for the model to read.
struct Error {
    QString kind;      // unknown-verb, missing-argument, unknown-field, bad-value, bad-query, bad-date, not-found, ambiguous, read-only, refused
    QString message;
    QList<Candidate> candidates;
    QString hint;
    QJsonObject toJson() const;
};

struct Result {
    bool ok = false;
    QString action;          // the verb that produced it
    QJsonObject body;        // the response fields, without `ok`/`action`
    QString helpText;        // help is text, not JSON
    bool changedStore = false;
    Error error;
};

// Run one argument list against a store.
Result run(Store &store, const QStringList &args, const QDateTime &now, const QDate &today);
// The text the command line prints: help as-is, everything else one JSON object carrying `ok`.
QString render(const Result &result);

// Exact id, then exact title, then any title containing the text. An open
// task wins over a completed one of the same name; two open ones are an error.
std::optional<TaskId> resolveTask(const Store &store, const QString &reference, Error *error);
std::optional<ProjectId> resolveProject(const Store &store, const QString &reference, Error *error);

// A task with every id resolved to a name.
QJsonObject taskView(const Store &store, const Task &task, const QDate &today, bool detailed = false);

} // namespace agent
} // namespace planner
