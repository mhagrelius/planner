// planner-server over HTTP.
//
// Four routes with JSON bodies and no TLS, on a tailnet: request line, headers,
// body, read the reply, close. Every call blocks, and every one of them runs
// on a worker thread — the App hands it a snapshot and gets records back, and
// nothing in here touches the store.
#pragma once

#include "sync.h"

#include <QByteArray>
#include <QString>

class HttpRemote : public planner::sync::Remote {
public:
    // Deliberately narrow: `http://host[:port]`, nothing else.
    static std::optional<HttpRemote> parse(const QString &url, const QString &token, QString *error);

    std::optional<planner::sync::Snapshot> snapshot(planner::sync::Error *error) override;
    std::optional<QList<planner::sync::Record>> fetch(const QList<planner::sync::Key> &keys, planner::sync::Error *error) override;
    bool push(const QList<planner::sync::Record> &records, planner::sync::Error *error) override;
    bool remove(const QList<planner::sync::Record> &records, planner::sync::Error *error) override;
    // Block until the server says something changed after `since`, or gives
    // up waiting (about fifty seconds). Returns whether anything moved and the
    // server's new cursor.
    std::optional<std::pair<bool, QDateTime>> waitForChange(const QDateTime &since, planner::sync::Error *error);

    QString host() const { return m_host; }

private:
    std::optional<QByteArray> request(const char *method, const QString &path, const QByteArray &body, int timeoutMs, planner::sync::Error *error);
    QString m_host;
    quint16 m_port = 80;
    QString m_token;
};
