// One running Planner at a time.
//
// The store lives whole in the running process's memory and is flushed on a
// tick, so a second process writing the file would be overwritten within two
// seconds. A second launch therefore hands its arguments to the first over a
// local socket: `planner agent …` is answered by the window, which updates as
// the commands run, and a bare second launch just raises it.
#pragma once

#include <QLocalServer>
#include <QObject>
#include <QStringList>

#include <functional>
#include <optional>

class SingleInstance : public QObject {
public:
    // Where the socket lives: $XDG_RUNTIME_DIR/planner.sock, else the temp dir.
    static QString socketPath();
    // Try to hand `args` to a running instance. Returns the reply if one
    // answered, nothing if there is no instance. A window that holds the socket
    // but does not answer is a reply too: `ok` false with a `busy` error, so
    // the caller never writes the file underneath a running window.
    struct Reply { QString output; bool ok; };
    static std::optional<Reply> forward(const QStringList &args, int timeoutMs = 700);

    // Become the instance: `handler` runs each forwarded argument list.
    explicit SingleInstance(std::function<Reply(const QStringList &)> handler, QObject *parent = nullptr);
    bool listen();

private:
    std::function<Reply(const QStringList &)> m_handler;
    QLocalServer m_server;
};
