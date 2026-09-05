#include "backend.h"
#include "palette.h"
#include "systemtheme.h"

#include <QCommandLineParser>
#include <QFont>
#include <QGuiApplication>
#include <QIcon>
#include <QQmlApplicationEngine>
#include <QQmlError>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QScreen>
#include <QTextStream>
#include <QTimer>

int main(int argc, char *argv[]) {
    QGuiApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("planner"));
    app.setApplicationDisplayName(QStringLiteral("Planner"));
    // Do not call setDesktopFileName(): under the systemd scope the Omarchy
    // menu launches apps in, Qt's host-portal registration fails and logs a
    // warning. The Wayland app_id already equals the binary name.
    app.setWindowIcon(QIcon::fromTheme(QStringLiteral("planner")));

    QCommandLineParser parser;
    parser.addHelpOption();
    // Headless checks: --screen opens on a screen id; --grab renders one frame
    // to a PNG and exits (QT_QPA_PLATFORM=offscreen), so a screen can be
    // inspected without touching the desktop.
    const QCommandLineOption screenOption(QStringLiteral("screen"), QStringLiteral("Open on the given screen id."), QStringLiteral("id"));
    const QCommandLineOption grabOption(QStringLiteral("grab"), QStringLiteral("Render one frame to this PNG and exit."), QStringLiteral("file"));
    const QCommandLineOption infoOption(QStringLiteral("info"), QStringLiteral("Print the resolved theme, palette sample and text scale, then exit."));
    parser.addOption(screenOption);
    parser.addOption(grabOption);
    parser.addOption(infoOption);
    parser.process(app);

    // Custom-drawn controls; Basic keeps Qt Quick Controls styling out of the way.
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    Palette palette(&app);
    if (QScreen *screen = app.primaryScreen())
        palette.setPointsPerPixel(72.0 / screen->logicalDotsPerInch());

    // Desktop text scale (omarchy display text size) and dark/light, via the portal.
    SystemTheme systemTheme(&app);
    palette.setTextScale(systemTheme.textScale());
    QObject::connect(&systemTheme, &SystemTheme::textScaleChanged, &palette, &Palette::setTextScale);

    if (parser.isSet(infoOption)) {
        // Give the async portal reply a moment so the printed scale is the real one.
        QTimer::singleShot(400, &app, [&palette]() {
            QTextStream(stdout) << "theme " << palette.themeName() << (palette.dark() ? " dark" : " light")
                << " scale " << palette.textScale() << " window " << palette.window().name()
                << " accent " << palette.accent().name() << " positive " << palette.positive().name()
                << " mono " << palette.monoFamily() << " sans " << palette.sansFamily() << "\n";
            QCoreApplication::exit(0);
        });
        return app.exec();
    }

    Backend backend(&palette, &app);
    if (parser.isSet(screenOption))
        backend.go(parser.value(screenOption));

    QQmlApplicationEngine engine;
    QObject::connect(&engine, &QQmlApplicationEngine::warnings, &app, [](const QList<QQmlError> &warnings) {
        for (const QQmlError &warning : warnings)
            qWarning().noquote() << warning.toString();
    });
    engine.loadFromModule(QStringLiteral("Planner"), QStringLiteral("Main"));
    if (engine.rootObjects().isEmpty()) {
        qCritical() << "Could not load the interface";
        return -1;
    }
    if (parser.isSet(grabOption)) {
        auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().first());
        const QString file = parser.value(grabOption);
        QTimer::singleShot(400, &app, [window, file]() {
            const bool ok = window && window->grabWindow().save(file);
            if (!ok) qCritical() << "grab failed:" << file;
            QCoreApplication::exit(ok ? 0 : 2);
        });
    }
    return app.exec();
}
