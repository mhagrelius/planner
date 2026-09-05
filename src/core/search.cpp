#include "search.h"

#include "store.h"

#include <algorithm>

namespace planner {

std::optional<int> searchScore(const QString &haystack, const QString &needle) {
    if (needle.isEmpty()) return 0;
    const QString hay = haystack.toLower();
    const QString want = needle.toLower();
    const int position = hay.indexOf(want);
    if (position < 0) return std::nullopt;
    int base;
    if (hay == want) base = 1000;
    else if (position == 0) base = 800;
    else if (!hay.at(position - 1).isLetterOrNumber()) base = 600;
    else base = 400;
    const int brevity = std::max(0, 100 - std::min(100, static_cast<int>(haystack.size())));
    return base + brevity;
}

QList<Hit> search(const Store &store, const QString &rawQuery, int limit) {
    const QString query = rawQuery.trimmed();
    if (query.isEmpty()) return {};
    QList<std::pair<int, Hit>> scored;

    for (const Project &project : store.projects()) {
        if (const auto score = searchScore(project.name, query)) {
            Hit hit;
            hit.kind = Hit::ProjectHit;
            hit.id = project.id;
            hit.title = project.name;
            hit.context = QStringLiteral("Project");
            scored.append({*score, hit});
        }
    }
    for (const Label &label : store.labels()) {
        if (const auto score = searchScore(label.name, query)) {
            Hit hit;
            hit.kind = Hit::LabelHit;
            hit.id = label.id;
            hit.title = label.name;
            hit.context = QStringLiteral("Label");
            scored.append({*score, hit});
        }
    }
    for (const Task &task : store.tasks()) {
        // The description and the notes match too, but never as well as the title.
        const auto title = searchScore(task.content, query);
        std::optional<int> body = searchScore(task.description, query);
        for (const Note &note : task.notes)
            if (const auto s = searchScore(note.text, query); s && (!body || *s > *body)) body = s;
        std::optional<int> best = title;
        if (!best && body) best = *body / 3;
        if (!best) continue;
        int score = *best;
        if (task.checked) score = std::max(0, score - 500);
        const Project *project = store.project(task.projectId);
        Hit hit;
        hit.kind = Hit::TaskHit;
        hit.id = task.id;
        hit.title = task.content;
        hit.context = project ? project->name : QString();
        if (task.checked) hit.context += QStringLiteral(" · completed");
        hit.priority = task.priority;
        hit.completed = task.checked;
        scored.append({score, hit});
    }

    std::stable_sort(scored.begin(), scored.end(), [](const auto &a, const auto &b) {
        if (a.first != b.first) return a.first > b.first;
        return a.second.title.toLower() < b.second.title.toLower();
    });
    QList<Hit> hits;
    for (const auto &entry : scored) {
        if (hits.size() >= limit) break;
        hits.append(entry.second);
    }
    return hits;
}

} // namespace planner
