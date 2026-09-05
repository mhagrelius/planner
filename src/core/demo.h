// A store with enough in it to show every part of a row: the seeded content
// the design handoff was drawn from. Only behind `--demo`; a first run is an
// empty, structured book.
#pragma once

#include "store.h"

namespace planner {

// Fill an (empty) store. `today` fixes the relative dates so a grab on any
// day matches the design, which is dated Saturday 5 September 2026.
void seedDemo(Store &store, const QDate &today, const QDateTime &now);

} // namespace planner
