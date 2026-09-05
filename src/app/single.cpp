#include "single.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>

QString SingleInstance::socketPath() {
    const QString runtime = qEnvironmentVariable("XDG_RUNTIME_DIR");
    const QString dir = runtime.isEmpty() ? QDir::tempPath() : runtime;
    // Tests and scratch stores must not collide with the real window.
    const QString suffix = qEnvironmentVariable("PLANNER_SOCKET_SUFFIX");
    return dir + QStringLiteral("/planner%1.sock").arg(suffix);
}

std::optional<SingleInstance::Reply> SingleInstance::forward(const QStringList &args, int timeoutMs) {
    QLocalSocket socket;
    socket.connectToServer(socketPath());
    if (!socket.waitForConnected(timeoutMs)) return std::nullopt;
    socket.write(QJsonDocument(QJsonObject{{QStringLiteral("args"), QJsonArray::fromStringList(args)}}).toJson(QJsonDocument::Compact) + '\n');
    socket.flush();
    QByteArray buffer;
    while (!buffer.contains('\n')) {
        if (!socket.waitForReadyRead(5000)) return std::nullopt;
        buffer += socket.readAll();
    }
    const QJsonObject reply = QJsonDocument::fromJson(buffer.left(buffer.indexOf('\n'))).object();
    return Reply{reply.value(QStringLiteral("output")).toString(), reply.value(QStringLiteral("ok")).toBool()};
}

SingleInstance::SingleInstance(std::function<Reply(const QStringList &)> handler, QObject *parent)
    : QObject(parent), m_handler(std::move(handler)) {
    m_server.setSocketOptions(QLocalServer::UserAccessOption);
    connect(&m_server, &QLocalServer::newConnection, this, [this]() {
        while (QLocalSocket *socket = m_server.nextPendingConnection()) {
            auto *buffer = new QByteArray;
            connect(socket, &QLocalSocket::readyRead, this, [this, socket, buffer]() {
                *buffer += socket->readAll();
                const int end = buffer->indexOf('\n');
                if (end < 0) return;
                const QJsonObject request = QJsonDocument::fromJson(buffer->left(end)).object();
                QStringList args;
                for (const QJsonValue &value : request.value(QStringLiteral("args")).toArray()) args << value.toString();
                const Reply reply = m_handler(args);
                socket->write(QJsonDocument(QJsonObject{{QStringLiteral("output"), reply.output}, {QStringLiteral("ok"), reply.ok}}).toJson(QJsonDocument::Compact) + '\n');
                socket->flush();
                socket->disconnectFromServer();
            });
            connect(socket, &QLocalSocket::disconnected, socket, [socket, buffer]() { delete buffer; socket->deleteLater(); });
        }
    });
}

bool SingleInstance::listen() {
    // A stale socket from a crashed instance would otherwise block every launch.
    QLocalServer::removeServer(socketPath());
    return m_server.listen(socketPath());
}
