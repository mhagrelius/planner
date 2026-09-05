#include "query.h"

#include "dates.h"
#include "store.h"

#include <QRegularExpression>

#include <cstring>

namespace planner {

bool Term::operator==(const Term &other) const {
    return kind == other.kind && date == other.date && priority == other.priority && name == other.name
        && includeSubprojects == other.includeSubprojects;
}

Filter Filter::negate(const Filter &inner) {
    Filter f;
    f.kind = Not;
    f.left = std::make_shared<Filter>(inner);
    return f;
}

Filter Filter::both(const Filter &a, const Filter &b) {
    Filter f;
    f.kind = And;
    f.left = std::make_shared<Filter>(a);
    f.right = std::make_shared<Filter>(b);
    return f;
}

Filter Filter::either(const Filter &a, const Filter &b) {
    Filter f;
    f.kind = Or;
    f.left = std::make_shared<Filter>(a);
    f.right = std::make_shared<Filter>(b);
    return f;
}

bool Filter::operator==(const Filter &other) const {
    if (kind != other.kind) return false;
    switch (kind) {
    case All: return true;
    case TermIs: return term == other.term;
    case Not: return *left == *other.left;
    case And:
    case Or: return *left == *other.left && *right == *other.right;
    }
    return false;
}

static bool dateMatches(const DateFilter &filter, const QDate &date, const QDate &today) {
    const auto target = parseDate(filter.phrase, today);
    // An unparseable date matches nothing. The parser rejects these up
    // front; this is the belt to that braces.
    if (!target) return false;
    switch (filter.kind) {
    case DateFilter::On: return date == *target;
    case DateFilter::Before: return date < *target;
    case DateFilter::After: return date > *target;
    }
    return false;
}

static bool termMatches(const Term &term, const Task &task, const Store &store, const QDate &today) {
    switch (term.kind) {
    case Term::Due: return task.due && dateMatches(term.date, task.due->date, today);
    case Term::Deadline: return task.deadline && dateMatches(term.date, *task.deadline, today);
    case Term::Overdue: return task.isOverdue(today);
    case Term::NoDate: return !task.due;
    case Term::NoDeadline: return !task.deadline;
    case Term::Recurring: return task.due && task.due->isRecurring();
    case Term::Subtask: return task.isSubtask();
    case Term::Pinned: return task.pinned;
    case Term::Completed: return task.checked;
    case Term::PriorityIs: return task.priority == term.priority;
    case Term::NoLabels: return task.labels.isEmpty();
    case Term::LabelIs: {
        const Label *label = store.labelByName(term.name);
        return label && task.hasLabel(label->id);
    }
    case Term::ProjectIs: {
        // An unknown project matches nothing rather than everything.
        const Project *project = store.projectByName(term.name);
        if (!project) return false;
        if (!term.includeSubprojects) return task.projectId == project->id;
        return store.projectAndDescendants(project->id).contains(task.projectId);
    }
    case Term::SectionIs: {
        if (!task.sectionId) return false;
        const auto found = store.section(*task.sectionId);
        return found.second && found.second->name.compare(term.name, Qt::CaseInsensitive) == 0;
    }
    case Term::Search: {
        const QString needle = term.name.toLower();
        return task.content.toLower().contains(needle) || task.description.toLower().contains(needle);
    }
    }
    return false;
}

bool Filter::matches(const Task &task, const Store &store, const QDate &today) const {
    switch (kind) {
    case All: return true;
    case TermIs: return termMatches(term, task, store, today);
    case Not: return !left->matches(task, store, today);
    case And: return left->matches(task, store, today) && right->matches(task, store, today);
    case Or: return left->matches(task, store, today) || right->matches(task, store, today);
    }
    return false;
}

static bool mentionsCompleted(const Filter &filter) {
    switch (filter.kind) {
    case Filter::All: return false;
    case Filter::TermIs: return filter.term.kind == Term::Completed;
    case Filter::Not: return mentionsCompleted(*filter.left);
    case Filter::And:
    case Filter::Or: return mentionsCompleted(*filter.left) || mentionsCompleted(*filter.right);
    }
    return false;
}

// --- tokenizer ---------------------------------------------------------------

namespace {

struct Token {
    enum Kind { Text, And, Or, Not, Comma, Open, Close } kind;
    QString text;
    int at;
};

// Term text may contain spaces — `#My Project`, `due before: next friday` —
// so a run continues until an operator. A backslash escapes the character
// after it, which is how a project genuinely called "R&D" is written.
std::optional<QList<Token>> tokenize(const QString &source, QueryError *error) {
    QList<Token> tokens;
    QString text;
    int textStart = 0;
    auto flush = [&]() {
        const QString trimmed = text.trimmed();
        if (!trimmed.isEmpty()) tokens.append({Token::Text, trimmed, textStart});
        text.clear();
    };
    for (int i = 0; i < source.size(); ++i) {
        const QChar c = source.at(i);
        if (c == u'\\') {
            if (text.isEmpty()) textStart = i;
            if (i + 1 >= source.size()) {
                if (error) *error = {QStringLiteral("the query ends with a stray backslash"), i};
                return std::nullopt;
            }
            text += source.at(++i);
        } else if (c == u'&' || c == u'|' || c == u'!' || c == u',' || c == u'(' || c == u')') {
            flush();
            Token::Kind kind = c == u'&' ? Token::And : c == u'|' ? Token::Or : c == u'!' ? Token::Not
                             : c == u',' ? Token::Comma : c == u'(' ? Token::Open : Token::Close;
            tokens.append({kind, QString(c), i});
        } else {
            if (text.isEmpty() && !c.isSpace()) textStart = i;
            text += c;
        }
    }
    flush();
    return tokens;
}

// Build a term from a name, rejecting an empty one.
std::optional<Term> named(const QString &raw, int at, const char *kind, Term::Kind termKind, bool subprojects, QueryError *error) {
    const QString name = raw.trimmed();
    if (name.isEmpty()) {
        if (error) *error = {QStringLiteral("this needs a %1 name after it").arg(QLatin1String(kind)), at};
        return std::nullopt;
    }
    Term term;
    term.kind = termKind;
    term.name = name;
    term.includeSubprojects = subprojects;
    return term;
}

// Validate a date phrase at parse time so a typo is reported as a broken
// query rather than a filter that silently matches nothing. Any probe date
// will do: a phrase either parses on every day or on none.
std::optional<DateFilter> dateFilter(const QString &raw, int at, QueryError *error) {
    QString text = raw.trimmed();
    DateFilter filter;
    for (const auto &[prefix, kind] : {std::make_pair("before:", DateFilter::Before), std::make_pair("before ", DateFilter::Before),
                                       std::make_pair("after:", DateFilter::After), std::make_pair("after ", DateFilter::After)}) {
        if (text.startsWith(QLatin1String(prefix))) {
            filter.kind = kind;
            text = text.mid(static_cast<int>(strlen(prefix))).trimmed();
            break;
        }
    }
    if (text.isEmpty()) {
        if (error) *error = {QStringLiteral("this needs a date after it"), at};
        return std::nullopt;
    }
    if (!parseDate(text, QDate(2026, 7, 30))) {
        if (error) *error = {QStringLiteral("`%1` is not a date I understand").arg(text), at};
        return std::nullopt;
    }
    filter.phrase = text;
    return filter;
}

std::optional<Term> parseTerm(const QString &text, int at, QueryError *error) {
    const QString lowered = text.toLower();
    const QString collapsed = lowered.split(QRegularExpression(QStringLiteral("\\s+")), Qt::SkipEmptyParts).join(u' ');
    Term term;
    auto simple = [&](Term::Kind kind) { term.kind = kind; return term; };

    if (collapsed == u"overdue" || collapsed == u"od") return simple(Term::Overdue);
    if (collapsed == u"no date" || collapsed == u"nodate" || collapsed == u"no due date") return simple(Term::NoDate);
    if (collapsed == u"no deadline") return simple(Term::NoDeadline);
    if (collapsed == u"no labels" || collapsed == u"no label") return simple(Term::NoLabels);
    if (collapsed == u"recurring") return simple(Term::Recurring);
    if (collapsed == u"subtask") return simple(Term::Subtask);
    if (collapsed == u"pinned") return simple(Term::Pinned);
    if (collapsed == u"completed" || collapsed == u"done") return simple(Term::Completed);
    if (collapsed == u"all") {
        if (error) *error = {QStringLiteral("`all` is not a condition; leave the query empty instead"), at};
        return std::nullopt;
    }
    if (const auto priority = priorityFromToken(collapsed)) {
        term.kind = Term::PriorityIs;
        term.priority = *priority;
        return term;
    }
    if (text.startsWith(u"##")) return named(text.mid(2), at, "project", Term::ProjectIs, true, error);
    if (text.startsWith(u'#')) return named(text.mid(1), at, "project", Term::ProjectIs, false, error);
    if (text.startsWith(u'@')) return named(text.mid(1), at, "label", Term::LabelIs, false, error);
    if (text.startsWith(u'/')) return named(text.mid(1), at, "section", Term::SectionIs, false, error);
    if (collapsed.startsWith(u"search:")) {
        const int rest = collapsed.size() - 7;
        return named(text.right(rest), at, "search", Term::Search, false, error);
    }

    // `due: ...`, `date: ...`, `deadline: ...`, or a bare date phrase.
    QString rest = collapsed;
    bool isDeadline = false;
    for (const auto &[prefix, deadline] : {std::make_pair("deadline:", true), std::make_pair("deadline ", true), std::make_pair("due:", false),
                                           std::make_pair("due ", false), std::make_pair("date:", false), std::make_pair("date ", false)}) {
        if (collapsed.startsWith(QLatin1String(prefix))) {
            rest = collapsed.mid(static_cast<int>(strlen(prefix))).trimmed();
            isDeadline = deadline;
            break;
        }
    }
    const auto filter = dateFilter(rest, at, error);
    if (!filter) return std::nullopt;
    term.kind = isDeadline ? Term::Deadline : Term::Due;
    term.date = *filter;
    return term;
}

struct Parser {
    const QList<Token> &tokens;
    int position = 0;
    QueryError *error;

    const Token *peek() const { return position < tokens.size() ? &tokens.at(position) : nullptr; }
    int endOffset() const { return tokens.isEmpty() ? 0 : tokens.last().at; }

    std::optional<QList<Filter>> parseLists() {
        QList<Filter> lists;
        auto first = parseOr();
        if (!first) return std::nullopt;
        lists.append(*first);
        while (peek() && peek()->kind == Token::Comma) {
            ++position;
            auto next = parseOr();
            if (!next) return std::nullopt;
            lists.append(*next);
        }
        return lists;
    }

    std::optional<Filter> parseOr() {
        auto left = parseAnd();
        if (!left) return std::nullopt;
        while (peek() && peek()->kind == Token::Or) {
            ++position;
            auto right = parseAnd();
            if (!right) return std::nullopt;
            left = Filter::either(*left, *right);
        }
        return left;
    }

    std::optional<Filter> parseAnd() {
        auto left = parseUnary();
        if (!left) return std::nullopt;
        while (peek() && peek()->kind == Token::And) {
            ++position;
            auto right = parseUnary();
            if (!right) return std::nullopt;
            left = Filter::both(*left, *right);
        }
        return left;
    }

    std::optional<Filter> parseUnary() {
        const Token *token = peek();
        if (!token) {
            if (error) *error = {QStringLiteral("the query ends where a condition was expected"), endOffset()};
            return std::nullopt;
        }
        switch (token->kind) {
        case Token::Not: {
            ++position;
            auto inner = parseUnary();
            if (!inner) return std::nullopt;
            return Filter::negate(*inner);
        }
        case Token::Open: {
            const int at = token->at;
            ++position;
            auto inner = parseOr();
            if (!inner) return std::nullopt;
            if (peek() && peek()->kind == Token::Close) {
                ++position;
                return inner;
            }
            if (error) *error = {QStringLiteral("this bracket is never closed"), at};
            return std::nullopt;
        }
        case Token::Text: {
            ++position;
            auto term = parseTerm(token->text, token->at, error);
            if (!term) return std::nullopt;
            return Filter::of(*term);
        }
        default:
            if (error) *error = {QStringLiteral("expected a condition here"), token->at};
            return std::nullopt;
        }
    }

    bool expectEnd() {
        const Token *token = peek();
        if (!token) return true;
        if (token->kind == Token::Close) {
            if (error) *error = {QStringLiteral("this bracket was never opened"), token->at};
        } else if (error) {
            *error = {QStringLiteral("unexpected text after the end of the query"), token->at};
        }
        return false;
    }
};

} // namespace

std::optional<Query> Query::parse(const QString &source, QueryError *error) {
    const auto tokens = tokenize(source, error);
    if (!tokens) return std::nullopt;
    Parser parser{*tokens, 0, error};
    const auto lists = parser.parseLists();
    if (!lists) return std::nullopt;
    if (!parser.expectEnd()) return std::nullopt;
    Query query;
    query.m_lists = *lists;
    for (const Filter &filter : *lists)
        if (mentionsCompleted(filter)) query.m_includesCompleted = true;
    return query;
}

Query Query::all() {
    Query query;
    query.m_lists.append(Filter::all());
    return query;
}

bool Query::matches(const Task &task, const Store &store, const QDate &today) const {
    if (task.checked && !m_includesCompleted) return false;
    for (const Filter &filter : m_lists)
        if (filter.matches(task, store, today)) return true;
    return false;
}

QList<const Task *> Query::run(const Store &store, const QDate &today) const {
    QList<const Task *> matching;
    for (const Task &task : store.tasks())
        if (matches(task, store, today)) matching.append(&task);
    return matching;
}

} // namespace planner
