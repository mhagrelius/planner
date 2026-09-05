// Where to sync to, if anywhere.
//
// Sync is off until a URL and a token are written here on purpose: there is
// nothing sensible for the app to guess at. A missing or unreadable file is
// "no sync", not an error. `$XDG_CONFIG_HOME/planner/config.json`, shared with
// the GTK client on `main`.
#pragma once

#include <QString>

#include <optional>

namespace planner {

struct Config {
    QString syncUrl;    // http://host:port of a planner-server
    QString syncToken;  // the bearer token it was started with

    static QString defaultPath();
    static Config load() { return loadFrom(defaultPath()); }
    static Config loadFrom(const QString &path);
    // Both or neither: a URL with no token cannot authenticate.
    std::optional<std::pair<QString, QString>> syncTarget() const;
};

} // namespace planner
