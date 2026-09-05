// English dates, times and repeat rules, parsed from what a person types.
//
// Shared deliberately between quick-add and the filter query language: if
// `due: friday` in a filter meant something different from `friday` typed
// into the entry, the app would be lying about its own vocabulary. Every
// function takes `today`; nothing here reads the clock.
#pragma once

#include "model.h"

#include <QStringList>

namespace planner {

// A complete date phrase: `today`, `fri`, `next friday`, `27th`, `3 august`,
// `in 3 days`, `end of month`, `2026-08-03`. The whole string must parse.
// Ambiguous numeric forms like `03/07` are refused rather than guessed.
std::optional<QDate> parseDate(const QString &text, const QDate &today);
// `9am`, `17:30`, `noon`, `at 5pm`, `tonight`.
std::optional<QTime> parseTime(const QString &text);
// `every other monday`, `every 3 days until 1 september`, `every! 10 days`,
// `daily`, `every day x3`.
std::optional<Recurrence> parseRecurrence(const QString &text, const QDate &today);

// Lower-case words with meaningless punctuation dropped. Shared by the parsers.
QStringList normaliseWords(const QString &text);
std::optional<Weekday> weekdayFromName(const QString &word);
QString singular(const QString &word);

} // namespace planner
