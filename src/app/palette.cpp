#include "palette.h"

#include <QDir>
#include <QFile>
#include <QTextStream>
#include <QtGlobal>

Palette *Palette::s_instance = nullptr;

namespace {

QColor mix(const QColor &a, const QColor &b, qreal t) {
    return QColor::fromRgbF(a.redF() + (b.redF() - a.redF()) * t,
                            a.greenF() + (b.greenF() - a.greenF()) * t,
                            a.blueF() + (b.blueF() - a.blueF()) * t, 1.0);
}

// Push a hue to a legible lightness on the current background: the theme
// palettes carry terminal reds and oranges that read fine on a prompt but
// sink into a dark card, so semantic colours are lifted toward the mock's
// weight (or darkened on light themes).
QColor lift(const QColor &c, qreal target, bool dark) {
    const qreal l = c.lightnessF();
    const qreal next = dark ? qMax(l, target) : qMin(l, 1.0 - target);
    return QColor::fromHslF(qMax(0.0, c.hslHueF()), c.hslSaturationF(), next);
}

qreal luminance(const QColor &c) {
    return 0.299 * c.redF() + 0.587 * c.greenF() + 0.114 * c.blueF();
}

QString defaultThemeDir() {
    const QByteArray override = qgetenv("OMARCHY_THEME_DIR");
    if (!override.isEmpty())
        return QString::fromLocal8Bit(override);
    const QByteArray stateHome = qgetenv("XDG_STATE_HOME");
    const QString state = stateHome.isEmpty() ? QDir::homePath() + QStringLiteral("/.local/state")
                                              : QString::fromLocal8Bit(stateHome);
    return state + QStringLiteral("/omarchy/current/theme");
}

} // namespace

Palette::Palette(QObject *parent) : QObject(parent), m_themeDir(defaultThemeDir()) {
    s_instance = this;
    load();
    watch();
    const auto reload = [this]() { load(); watch(); };
    connect(&m_watcher, &QFileSystemWatcher::fileChanged, this, reload);
    connect(&m_watcher, &QFileSystemWatcher::directoryChanged, this, reload);
}

Palette *Palette::instance() { return s_instance; }

QString Palette::qtVersion() const {
    const QStringList parts = QString::fromLatin1(qVersion()).split(QLatin1Char('.'));
    return parts.size() >= 2 ? parts.at(0) + QLatin1Char('.') + parts.at(1) : QString::fromLatin1(qVersion());
}

Palette *Palette::create(QQmlEngine *, QJSEngine *engine) {
    Q_ASSERT(s_instance);
    QJSEngine::setObjectOwnership(s_instance, QJSEngine::CppOwnership);
    Q_UNUSED(engine);
    return s_instance;
}

void Palette::setTextScale(qreal scale) {
    if (qFuzzyCompare(m_textScale, scale))
        return;
    m_textScale = scale;
    emit textScaleChanged();
}

QColor Palette::role(const QString &name) const {
    const auto it = m_roles.constFind(name);
    if (it != m_roles.constEnd())
        return *it;
    const QColor direct(name);
    return direct.isValid() ? direct : m_roles.value(QStringLiteral("text"));
}

// colors.toml is flat `key = "#rrggbb"` lines plus `mode = "dark"|"light"`.
void Palette::load() {
    QHash<QString, QColor> raw;
    QString mode;
    QFile file(m_themeDir + QStringLiteral("/colors.toml"));
    if (file.open(QIODevice::ReadOnly | QIODevice::Text)) {
        QTextStream in(&file);
        while (!in.atEnd()) {
            const QString line = in.readLine().trimmed();
            if (line.isEmpty() || line.startsWith(QLatin1Char('#')))
                continue;
            const int eq = line.indexOf(QLatin1Char('='));
            if (eq < 0)
                continue;
            const QString key = line.left(eq).trimmed();
            QString value = line.mid(eq + 1).trimmed();
            const int hash = value.indexOf(QLatin1Char('#'), 1);
            if (hash > 0 && !value.startsWith(QLatin1Char('"')) && !value.startsWith(QLatin1Char('\'')))
                value = value.left(hash).trimmed();
            if (value.size() >= 2 && (value.front() == QLatin1Char('"') || value.front() == QLatin1Char('\'')))
                value = value.mid(1, value.size() - 2);
            if (key == QStringLiteral("mode")) {
                mode = value;
                continue;
            }
            const QColor color(value);
            if (color.isValid())
                raw.insert(key, color);
        }
    }

    // Fall back to Kanagawa so the app still renders when no theme is present.
    static const QHash<QString, QColor> kanagawa = {
        {"background", QColor("#1f1f28")}, {"dark_background", QColor("#17171e")},
        {"darker_background", QColor("#111116")}, {"lighter_background", QColor("#223249")},
        {"foreground", QColor("#dcd7ba")}, {"dark_foreground", QColor("#727169")},
        {"light_foreground", QColor("#c8c093")}, {"bright_foreground", QColor("#dcd7ba")},
        {"accent", QColor("#7e9cd8")}, {"selection", QColor("#363646")}, {"muted", QColor("#54546d")},
        {"red", QColor("#c34043")}, {"yellow", QColor("#c0a36e")}, {"orange", QColor("#c17158")},
        {"green", QColor("#76946a")}, {"cyan", QColor("#6a9589")}, {"blue", QColor("#7e9cd8")},
        {"magenta", QColor("#957fb8")}, {"bright_red", QColor("#e82424")},
        {"bright_yellow", QColor("#e6c384")}, {"bright_green", QColor("#98bb6c")},
        {"bright_cyan", QColor("#7aa89f")}, {"bright_blue", QColor("#7fb4ca")},
        {"bright_magenta", QColor("#938aa9")}
    };
    for (auto it = kanagawa.constBegin(); it != kanagawa.constEnd(); ++it) {
        if (!raw.contains(it.key()))
            raw.insert(it.key(), it.value());
    }
    // Derived steps for themes that only define the basics.
    if (!raw.contains("bright_foreground")) raw["bright_foreground"] = raw["foreground"];
    if (!raw.contains("light_foreground")) raw["light_foreground"] = mix(raw["foreground"], raw["background"], 0.12);
    if (!raw.contains("dark_foreground")) raw["dark_foreground"] = mix(raw["foreground"], raw["background"], 0.5);

    m_raw = raw;
    if (mode == QStringLiteral("dark"))
        m_dark = true;
    else if (mode == QStringLiteral("light"))
        m_dark = false;
    else
        m_dark = luminance(raw["background"]) < 0.5;

    QFile nameFile(m_themeDir + QStringLiteral("/../theme.name"));
    m_themeName = nameFile.open(QIODevice::ReadOnly | QIODevice::Text)
        ? QString::fromUtf8(nameFile.readAll()).trimmed()
        : QDir(m_themeDir).dirName();

    rebuild();
    emit changed();
}

void Palette::watch() {
    const QStringList watched = m_watcher.files() + m_watcher.directories();
    if (!watched.isEmpty())
        m_watcher.removePaths(watched);
    const QString colors = m_themeDir + QStringLiteral("/colors.toml");
    const QString parent = QDir(m_themeDir).absolutePath() + QStringLiteral("/..");
    if (QDir(parent).exists())
        m_watcher.addPath(QDir(parent).absolutePath());
    if (QDir(m_themeDir).exists())
        m_watcher.addPath(m_themeDir);
    if (QFile::exists(colors))
        m_watcher.addPath(colors);
}

void Palette::rebuild() {
    const auto c = [this](const char *key) { return m_raw.value(QLatin1String(key)); };
    const QColor bg = c("background"), dbg = c("dark_background"), ddbg = c("darker_background"),
        lbg = c("lighter_background"), fg = c("foreground"), dfg = c("dark_foreground"),
        lfg = c("light_foreground"), bfg = c("bright_foreground"), accent = c("accent"),
        sel = c("selection"), muted = c("muted"), red = c("red"), yellow = c("yellow"),
        orange = c("orange"), blue = c("blue"), magenta = c("magenta"),
        bgreen = c("bright_green"), bcyan = c("bright_cyan"), bblue = c("bright_blue"),
        byellow = c("bright_yellow");

    QHash<QString, QColor> r;
    // Surfaces: the card sits one step below the window, the sidebar and the
    // table headers between the two.
    r["window"] = bg;
    r["card"] = dbg;
    r["sidebar"] = mix(dbg, bg, 0.15);
    r["header"] = mix(dbg, bg, 0.40);
    r["groupRow"] = mix(dbg, bg, 0.60);
    r["hover"] = r["header"];
    r["divider"] = r["groupRow"];
    r["track"] = mix(bg, sel, 0.15);
    r["border"] = mix(bg, sel, 0.50);
    r["borderStrong"] = sel;
    r["tooltipBg"] = dbg;

    r["text"] = fg;
    r["text2"] = lfg;
    r["muted"] = dfg;
    r["faint"] = muted;

    r["accent"] = accent;
    r["accentText"] = luminance(accent) > 0.45 ? ddbg : bfg;
    r["selection"] = lbg;
    r["activeFill"] = mix(lbg, blue, 0.20);
    r["activeText"] = mix(bblue, bfg, 0.45);

    r["positive"] = bgreen;
    r["positiveDim"] = mix(bgreen, bg, 0.45);
    r["negative"] = lift(red, 0.64, m_dark);
    r["warning"] = lift(mix(orange, byellow, 0.45), 0.68, m_dark);
    r["caution"] = yellow;
    r["cautionAlt"] = mix(byellow, orange, 0.30);
    r["teal"] = bcyan;
    r["blueGray"] = mix(blue, fg, 0.30);
    r["violet"] = magenta;
    r["pink"] = lift(mix(red, magenta, 0.5), 0.64, m_dark);
    r["info"] = bblue;
    r["blue"] = blue;

    r["positiveBg"] = mix(dbg, bgreen, 0.16);
    r["infoBg"] = lbg;
    r["warningBg"] = mix(dbg, orange, 0.14);
    r["cautionBg"] = mix(dbg, yellow, 0.22);
    r["errorBg"] = mix(dbg, red, 0.22);
    r["neutralBg"] = r["track"];
    r["destructiveHover"] = mix(dbg, red, 0.07);
    r["invalidBand"] = mix(dbg, red, 0.04);
    r["dragBg"] = mix(dbg, blue, 0.12);
    r["tealBorder"] = mix(dbg, bcyan, 0.25);

    m_roles = r;
}
