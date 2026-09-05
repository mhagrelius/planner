// Finding things by typing part of their name: substring matching with a
// score, not fuzzy subsequence matching. A prefix or word-start match is what
// people actually type, and ranking those first gets the right answer to the
// top without the noise.
#pragma once

#include "model.h"

namespace planner {

class Store;

struct Hit {
    enum Kind { TaskHit, ProjectHit, LabelHit } kind = TaskHit;
    QString id;
    QString title;
    QString context;   // project name, "· completed" appended for finished tasks
    // Enough of the task to draw its row: the ring and whether it is struck.
    Priority priority = Priority::P4;
    bool completed = false;
    bool operator==(const Hit &other) const { return kind == other.kind && id == other.id && title == other.title && context == other.context; }
};

// Exact 1000, prefix 800, word start 600, anywhere 400, plus up to 100 for
// brevity. Nothing for no match.
std::optional<int> searchScore(const QString &haystack, const QString &needle);
// Everything matching `query`, best first. Completed tasks rank 500 lower.
QList<Hit> search(const Store &store, const QString &query, int limit);

} // namespace planner
