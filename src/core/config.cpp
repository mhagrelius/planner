#include "config.h"

#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>

namespace planner {

QString Config::defaultPath() {
    const QByteArray xdg = qgetenv("XDG_CONFIG_HOME");
    QString base;
    if (!xdg.isEmpty()) base = QString::fromLocal8Bit(xdg);
    else if (!qgetenv("HOME").isEmpty()) base = QString::fromLocal8Bit(qgetenv("HOME")) + QStringLiteral("/.config");
    else base = QStringLiteral(".");
    return base + QStringLiteral("/planner/config.json");
}

Config Config::loadFrom(const QString &path) {
    Config config;
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) return config;
    const QJsonObject json = QJsonDocument::fromJson(file.readAll()).object();
    config.syncUrl = json.value(QStringLiteral("sync_url")).toString();
    config.syncToken = json.value(QStringLiteral("sync_token")).toString();
    return config;
}

std::optional<std::pair<QString, QString>> Config::syncTarget() const {
    if (syncUrl.isEmpty() || syncToken.isEmpty()) return std::nullopt;
    return std::make_pair(syncUrl, syncToken);
}

} // namespace planner
