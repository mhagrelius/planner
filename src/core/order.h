// Where a record sits in a hand-sorted list.
//
// A key that sorts lexicographically and can always be generated *between*
// two neighbours, so moving one task rewrites one record — which is what lets
// a reorder sync as a single changed record instead of a renumbered list. Two
// machines dropping into the same gap can mint the same key; the list then
// sorts by key and id, so nothing is lost. Base 36, ASCII, the same comparison
// Postgres makes on a text column.
#pragma once

#include <QString>

#include <optional>

namespace planner::order {

// The key for the first record in an empty list.
QString start();
// A key strictly after `before` and before `after`; either may be empty for
// "the end of the list in that direction".
QString between(const QString &before, const QString &after);
// The key for a position written by schema v1, which numbered lists 0, 1, 2…
// Fixed width, so v1 order survives and new keys land after it.
QString fromLegacyPosition(qint64 position);
// Compare two keys the way the list sorts: by bytes.
int compare(const QString &a, const QString &b);

} // namespace planner::order
