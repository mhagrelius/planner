#include "remote.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTcpSocket>
#include <QUrl>

using namespace planner;
using namespace planner::sync;

namespace {
constexpr int kTimeoutMs = 20 * 1000;
// Past the server's own fifty seconds, so the server giving up is not read as
// the network failing.
constexpr int kWaitTimeoutMs = 75 * 1000;
}

std::optional<HttpRemote> HttpRemote::parse(const QString &url, const QString &token, QString *error) {
    const QUrl parsed(url);
    if (!parsed.isValid() || parsed.scheme() != QLatin1String("http") || parsed.host().isEmpty()) {
        if (error) *error = QStringLiteral("%1 must be http://host:port").arg(url);
        return std::nullopt;
    }
    HttpRemote remote;
    remote.m_host = parsed.host();
    remote.m_port = static_cast<quint16>(parsed.port(80));
    remote.m_token = token;
    return remote;
}

std::optional<QByteArray> HttpRemote::request(const char *method, const QString &path, const QByteArray &body, int timeoutMs, Error *error) {
    QTcpSocket socket;
    socket.connectToHost(m_host, m_port);
    if (!socket.waitForConnected(kTimeoutMs)) {
        if (error) error->message = QStringLiteral("could not reach %1: %2").arg(m_host, socket.errorString());
        return std::nullopt;
    }
    const QByteArray head = QStringLiteral("%1 %2 HTTP/1.1\r\nHost: %3\r\nAuthorization: Bearer %4\r\nContent-Type: application/json\r\nContent-Length: %5\r\nConnection: close\r\n\r\n")
                                .arg(QLatin1String(method), path, m_host, m_token).arg(body.size()).toUtf8();
    socket.write(head);
    socket.write(body);
    if (!socket.waitForBytesWritten(kTimeoutMs)) {
        if (error) error->message = QStringLiteral("could not send: %1").arg(socket.errorString());
        return std::nullopt;
    }
    QByteArray reply;
    while (socket.state() != QAbstractSocket::UnconnectedState) {
        if (!socket.waitForReadyRead(timeoutMs)) {
            if (socket.state() == QAbstractSocket::UnconnectedState) break;
            if (error) error->message = QStringLiteral("could not read the reply: %1").arg(socket.errorString());
            return std::nullopt;
        }
        reply += socket.readAll();
    }
    reply += socket.readAll();

    const int separator = reply.indexOf("\r\n\r\n");
    if (separator < 0) {
        if (error) error->message = QStringLiteral("the reply had no body");
        return std::nullopt;
    }
    const QList<QByteArray> statusLine = reply.left(reply.indexOf("\r\n")).split(' ');
    const int status = statusLine.size() > 1 ? statusLine.at(1).toInt() : 0;
    const QByteArray payload = reply.mid(separator + 4);
    if (status == 200) return payload;
    // A 401 is the one failure a user can fix.
    if (error) error->message = status == 401 ? QStringLiteral("the server refused the token — check sync_token in the config")
                                              : QStringLiteral("the server said %1: %2").arg(status).arg(QString::fromUtf8(payload).trimmed());
    return std::nullopt;
}

std::optional<Snapshot> HttpRemote::snapshot(Error *error) {
    const auto body = request("GET", QStringLiteral("/snapshot"), {}, kTimeoutMs, error);
    if (!body) return std::nullopt;
    Snapshot snapshot;
    for (const QJsonValue &value : QJsonDocument::fromJson(*body).array()) {
        const QJsonObject entry = value.toObject();
        const auto kind = recordKindFromSerial(entry.value(QStringLiteral("kind")).toString());
        const auto updated = instantFromSerial(entry.value(QStringLiteral("updated_at")).toString());
        if (!kind || !updated) continue;
        const Key key{*kind, entry.value(QStringLiteral("id")).toString()};
        const auto deleted = instantFromSerial(entry.value(QStringLiteral("deleted_at")).toString());
        snapshot.insert(key, deleted ? Version::gone(*deleted) : Version::live(*updated));
    }
    return snapshot;
}

std::optional<QList<Record>> HttpRemote::fetch(const QList<Key> &keys, Error *error) {
    QJsonArray wanted;
    for (const Key &key : keys) wanted.append(QJsonObject{{QStringLiteral("kind"), recordKindSerial(key.kind)}, {QStringLiteral("id"), key.id}});
    const auto body = request("POST", QStringLiteral("/fetch"), QJsonDocument(wanted).toJson(QJsonDocument::Compact), kTimeoutMs, error);
    if (!body) return std::nullopt;
    QList<Record> records;
    for (const QJsonValue &value : QJsonDocument::fromJson(*body).array())
        if (const auto record = Record::fromJson(value.toObject())) records.append(*record);
    return records;
}

static QByteArray encode(const QList<Record> &records) {
    QJsonArray array;
    for (const Record &record : records) array.append(record.toJson());
    return QJsonDocument(array).toJson(QJsonDocument::Compact);
}

bool HttpRemote::push(const QList<Record> &records, Error *error) {
    return request("POST", QStringLiteral("/records"), encode(records), kTimeoutMs, error).has_value();
}

bool HttpRemote::remove(const QList<Record> &records, Error *error) {
    return request("POST", QStringLiteral("/deletions"), encode(records), kTimeoutMs, error).has_value();
}

std::optional<std::pair<bool, QDateTime>> HttpRemote::waitForChange(const QDateTime &since, Error *error) {
    // RFC 3339 in UTC with a `Z`: a `+` in an offset would read as a space in
    // a query string.
    const QString cursor = since.toUTC().toString(QStringLiteral("yyyy-MM-ddTHH:mm:ss.zzz")) + QStringLiteral("000Z");
    const auto body = request("GET", QStringLiteral("/changes?since=%1").arg(cursor), {}, kWaitTimeoutMs, error);
    if (!body) return std::nullopt;
    const QJsonObject answer = QJsonDocument::fromJson(*body).object();
    const auto now = instantFromSerial(answer.value(QStringLiteral("now")).toString());
    return std::make_pair(answer.value(QStringLiteral("changed")).toBool(false), now.value_or(since));
}
