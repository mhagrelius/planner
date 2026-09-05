// The second-launch socket. What matters is the three answers `forward` can
// give: a reply from a window, nothing when no window holds the socket (so the
// file is free), and a `busy` refusal when a window holds the socket and does
// not answer — never a fallback to the file underneath a running window.
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalServer>
#include <QElapsedTimer>
#include <QLocalSocket>
#include <QTemporaryDir>
#include <QtTest>

#include "single.h"

class TestSingle : public QObject {
    Q_OBJECT
    QTemporaryDir m_dir;
    QString m_suffix;

    // `forward` blocks the thread it runs on, and the window under test lives
    // on this one, so the call goes to a helper thread and this loop keeps serving.
    static std::optional<SingleInstance::Reply> forwardOnWorker(const QStringList &args, int timeoutMs) {
        std::optional<SingleInstance::Reply> reply;
        bool done = false;
        QThread worker;
        QObject context;
        context.moveToThread(&worker);
        worker.start();
        QMetaObject::invokeMethod(&context, [&]() { reply = SingleInstance::forward(args, timeoutMs); done = true; }, Qt::QueuedConnection);
        QElapsedTimer clock; clock.start();
        while (!done && clock.elapsed() < 8000) QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
        worker.quit();
        worker.wait();
        return reply;
    }

private slots:
    void initTestCase() {
        // Sockets are named from XDG_RUNTIME_DIR and the suffix; both are
        // pointed at this test so the real window is never touched.
        qputenv("XDG_RUNTIME_DIR", m_dir.path().toUtf8());
        m_suffix = QStringLiteral("-test%1").arg(QCoreApplication::applicationPid());
        qputenv("PLANNER_SOCKET_SUFFIX", m_suffix.toUtf8());
        QVERIFY(SingleInstance::socketPath().startsWith(m_dir.path()));
    }

    void noWindowMeansNoReply() {
        QVERIFY(!QFile::exists(SingleInstance::socketPath()));
        QVERIFY(!SingleInstance::forward({QStringLiteral("overview")}, 200).has_value());
    }

    void staleSocketMeansNoReply() {
        // A crash leaves the file behind with nobody listening: the file is free.
        QLocalServer server;
        QVERIFY(server.listen(SingleInstance::socketPath()));
        const QString path = SingleInstance::socketPath();
        server.close();  // QLocalServer::close removes the file; put a dead one back
        QFile dead(path);
        QVERIFY(dead.open(QIODevice::WriteOnly));
        dead.close();
        QVERIFY(QFile::exists(path));
        QVERIFY(!SingleInstance::forward({QStringLiteral("overview")}, 200).has_value());
        QFile::remove(path);
    }

    void aWindowAnswers() {
        SingleInstance instance([](const QStringList &args) -> SingleInstance::Reply {
            return {QStringLiteral("got ") + args.join(QLatin1Char(' ')), true};
        });
        QVERIFY(instance.listen());
        const auto reply = forwardOnWorker({QStringLiteral("list"), QStringLiteral("due: today")}, 500);
        QVERIFY(reply.has_value());
        QVERIFY(reply->ok);
        QCOMPARE(reply->output, QStringLiteral("got list due: today"));
    }

    void aSilentWindowIsBusyNotAbsent() {
        // Something holds the socket and never replies. The answer is a refusal
        // the caller prints and exits 1 on — not "no window, use the file".
        QLocalServer silent;
        QLocalServer::removeServer(SingleInstance::socketPath());
        QVERIFY(silent.listen(SingleInstance::socketPath()));
        QList<QLocalSocket *> held;
        connect(&silent, &QLocalServer::newConnection, this, [&]() {
            while (QLocalSocket *socket = silent.nextPendingConnection()) held << socket;
        });
        const auto reply = forwardOnWorker({QStringLiteral("add"), QStringLiteral("x")}, 300);
        QVERIFY(reply.has_value());
        QVERIFY(!reply->ok);
        const QJsonObject json = QJsonDocument::fromJson(reply->output.toUtf8()).object();
        QCOMPARE(json.value(QStringLiteral("ok")).toBool(true), false);
        QCOMPARE(json.value(QStringLiteral("error")).toString(), QStringLiteral("busy"));
        QVERIFY(json.value(QStringLiteral("message")).toString().contains(QStringLiteral("file was left alone")));
        qDeleteAll(held);
    }
};

QTEST_MAIN(TestSingle)
#include "test_single.moc"
