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

// The one answer a second process may give when a window holds the socket and
// does not reply: a refusal. Falling back to the file here would be a second
// writer under the window, overwritten on its next save.
static SingleInstance::Reply busy(const QString &why) {
    const QJsonObject envelope{{QStringLiteral("ok"), false},
                               {QStringLiteral("error"), QStringLiteral("busy")},
                               {QStringLiteral("message"), QStringLiteral("Planner is open but did not answer: %1. Nothing was changed, and the file was left alone because the window holds it.").arg(why)},
                               {QStringLiteral("hint"), QStringLiteral("Try again in a moment, or close the Planner window and run the command against the file.")}};
    return {QString::fromUtf8(QJsonDocument(envelope).toJson(QJsonDocument::Indented)).trimmed(), false};
}

std::optional<SingleInstance::Reply> SingleInstance::forward(const QStringList &args, int timeoutMs) {
    QLocalSocket socket;
    socket.connectToServer(socketPath());
    if (!socket.waitForConnected(timeoutMs)) {
        // No socket, or a stale one left by a crash: nobody is listening and the
        // file is free. Anything else is a window that holds the socket and did
        // not take the connection, which is not a licence to write underneath it.
        const QLocalSocket::LocalSocketError error = socket.error();
        if (error == QLocalSocket::ServerNotFoundError || error == QLocalSocket::ConnectionRefusedError) return std::nullopt;
        return busy(QStringLiteral("the connection was not accepted within %1 ms").arg(timeoutMs));
    }
    socket.write(QJsonDocument(QJsonObject{{QStringLiteral("args"), QJsonArray::fromStringList(args)}}).toJson(QJsonDocument::Compact) + '\n');
    socket.flush();
    QByteArray buffer;
    while (!buffer.contains('\n')) {
        if (!socket.waitForReadyRead(5000)) {
            return busy(socket.state() == QLocalSocket::ConnectedState ? QStringLiteral("no reply within 5 seconds") : QStringLiteral("the window closed the connection without replying"));
        }
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
