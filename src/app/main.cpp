#include "app.h"
#include "icons.h"
#include "palette.h"
#include "single.h"
#include "systemtheme.h"

#include <QCommandLineParser>
#include <QGuiApplication>
#include <QIcon>
#include <QQmlApplicationEngine>
#include <QQmlError>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QScreen>
#include <QTextStream>
#include <QTimer>

#include "agent.h"
#include "store.h"

int main(int argc, char *argv[]) {
    // `planner agent …` is a command, not a launch. It goes to the running
    // window when there is one, so the store in that process's memory is the
    // one that answers; with nothing running, this process does the work.
    QStringList raw;
    for (int i = 1; i < argc; ++i) raw << QString::fromLocal8Bit(argv[i]);
    const bool isAgent = !raw.isEmpty() && raw.first() == QStringLiteral("agent");

    QGuiApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("planner"));
    app.setApplicationDisplayName(QStringLiteral("Planner"));
    // No setDesktopFileName(): under the systemd scope the Omarchy menu
    // launches apps in, the host-portal registration fails and logs a warning.
    app.setWindowIcon(QIcon::fromTheme(QStringLiteral("planner")));

    QCommandLineParser parser;
    parser.setApplicationDescription(QStringLiteral(
        "A keyboard-driven task planner.\n\n"
        "Run with no arguments to open the window; a second launch raises the one\n"
        "already open. `planner agent <verb>` reads and changes tasks from a script\n"
        "or an assistant, printing JSON; start with `planner agent help`."));
    parser.addHelpOption();
    parser.addPositionalArgument(QStringLiteral("agent"), QStringLiteral("agent VERB [args]: drive the planner from outside the window."), QStringLiteral("[agent VERB ...]"));
    const QCommandLineOption dataOption(QStringLiteral("data"), QStringLiteral("Use this planner.json (or directory) instead of the default."), QStringLiteral("path"));
    const QCommandLineOption demoOption(QStringLiteral("demo"), QStringLiteral("Seed a throwaway store with the design's sample tasks."));
    const QCommandLineOption todayOption(QStringLiteral("today"), QStringLiteral("Pin today's date (YYYY-MM-DD), for grabs and tests."), QStringLiteral("date"));
    const QCommandLineOption screenOption(QStringLiteral("screen"), QStringLiteral("Open on a view: today, inbox, upcoming, pinned, completed, project:<name>."), QStringLiteral("id"));
    const QCommandLineOption actOption(QStringLiteral("act"), QStringLiteral("Enter a state before grabbing: detail, palette, add, find, select, board, picker, norail, cursor:N."), QStringLiteral("name"));
    const QCommandLineOption grabOption(QStringLiteral("grab"), QStringLiteral("Render one frame to this PNG and exit."), QStringLiteral("file"));
    const QCommandLineOption infoOption(QStringLiteral("info"), QStringLiteral("Print the resolved theme, palette sample and text scale, then exit."));
    parser.addOption(dataOption);
    parser.addOption(demoOption);
    parser.addOption(todayOption);
    parser.addOption(screenOption);
    parser.addOption(actOption);
    parser.addOption(grabOption);
    parser.addOption(infoOption);
    if (isAgent) {
        // Everything after `agent` belongs to the verb, `--flags` included.
        const QStringList agentArgs = raw.mid(1);
        if (const auto reply = SingleInstance::forward(agentArgs)) {
            QTextStream(stdout) << reply->output << "\n";
            return reply->ok ? 0 : 1;
        }
        planner::LoadOutcome outcome;
        planner::Store store = planner::Store::open(&outcome);
        const planner::agent::Result result = planner::agent::run(store, agentArgs, QDateTime::currentDateTimeUtc(), QDate::currentDate());
        if (result.changedStore) {
            if (const auto error = store.save()) {
                QTextStream(stderr) << error->message << "\n";
                return 1;
            }
        }
        QTextStream(stdout) << planner::agent::render(result) << "\n";
        return result.ok ? 0 : 1;
    }
    parser.process(app);

    // Custom-drawn controls; Basic keeps Qt Quick Controls styling out of the way.
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    Palette palette(&app);
    if (QScreen *screen = app.primaryScreen())
        palette.setPointsPerPixel(72.0 / screen->logicalDotsPerInch());

    SystemTheme systemTheme(&app);
    palette.setTextScale(systemTheme.textScale());
    QObject::connect(&systemTheme, &SystemTheme::textScaleChanged, &palette, &Palette::setTextScale);

    if (parser.isSet(infoOption)) {
        QTimer::singleShot(400, &app, [&palette]() {
            QTextStream(stdout) << "theme " << palette.themeName() << (palette.dark() ? " dark" : " light")
                << " scale " << palette.textScale() << " window " << palette.window().name()
                << " accent " << palette.accent().name() << " positive " << palette.positive().name()
                << " mono " << palette.monoFamily() << " sans " << palette.sansFamily() << "\n";
            QCoreApplication::exit(0);
        });
        return app.exec();
    }

    App::Options options;
    options.dataPath = parser.value(dataOption);
    options.demo = parser.isSet(demoOption);
    options.today = QDate::fromString(parser.value(todayOption), Qt::ISODate);
    options.screen = parser.value(screenOption);
    for (const QString &act : parser.values(actOption)) options.acts << act.split(u',', Qt::SkipEmptyParts);
    const bool scratch = options.demo || !options.dataPath.isEmpty() || parser.isSet(grabOption);

    // A second launch of the real app raises the first instead of opening a
    // second window over the same file. Scratch stores stay independent.
    SingleInstance single([](const QStringList &args) -> SingleInstance::Reply {
        if (args.isEmpty()) {
            emit App::instance()->windowRequested();
            return {QString(), true};
        }
        const auto [output, ok] = App::instance()->agentCommand(args);
        return {output, ok};
    });
    if (!scratch) {
        if (SingleInstance::forward({})) return 0;
        single.listen();
    }

    App backend(&palette, options, &app);

    QQmlApplicationEngine engine;
    engine.addImageProvider(QStringLiteral("icon"), new IconProvider);
    QObject::connect(&engine, &QQmlApplicationEngine::warnings, &app, [](const QList<QQmlError> &warnings) {
        for (const QQmlError &warning : warnings) qWarning().noquote() << warning.toString();
    });
    engine.loadFromModule(QStringLiteral("Planner"), QStringLiteral("Main"));
    if (engine.rootObjects().isEmpty()) {
        qCritical() << "Could not load the interface";
        return -1;
    }
    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().first());
    QObject::connect(&backend, &App::windowRequested, &app, [window]() {
        if (window) {
            window->show();
            window->requestActivate();
        }
    });
    QObject::connect(&app, &QGuiApplication::aboutToQuit, &backend, &App::saveNow);

    if (parser.isSet(grabOption)) {
        const QString file = parser.value(grabOption);
        QTimer::singleShot(500, &app, [window, file]() {
            const bool ok = window && window->grabWindow().save(file);
            if (!ok) qCritical() << "grab failed:" << file;
            QCoreApplication::exit(ok ? 0 : 2);
        });
    }
    return app.exec();
}
