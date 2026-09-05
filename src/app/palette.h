#pragma once

#include <QColor>
#include <QFileSystemWatcher>
#include <QHash>
#include <QObject>
#include <QQmlEngine>
#include <QString>

// Every semantic colour role the interface uses. Each is derived from the
// foundational keys in the active Omarchy theme's colors.toml (background,
// foreground, accent, selection, muted, the 8 hues and their bright variants,
// and the dark/darker/lighter background and foreground steps), so the app
// re-tints when the theme changes and every stock theme gets a coherent
// surface stack. With the Kanagawa theme the derivations land on the design
// mock's exact Kanagawa values.
#define OMARCHY_COLOR_ROLES(X) \
    X(window) X(sidebar) X(card) X(header) X(groupRow) X(hover) X(divider) \
    X(track) X(border) X(borderStrong) \
    X(text) X(text2) X(muted) X(faint) \
    X(accent) X(accentText) X(selection) X(activeFill) X(activeText) \
    X(positive) X(positiveDim) X(negative) X(warning) X(caution) X(cautionAlt) \
    X(teal) X(blueGray) X(violet) X(pink) X(info) X(blue) \
    X(positiveBg) X(infoBg) X(warningBg) X(cautionBg) X(errorBg) X(neutralBg) \
    X(destructiveHover) X(invalidBand) X(dragBg) X(tealBorder) X(tooltipBg)

class Palette : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

    Q_PROPERTY(bool dark READ dark NOTIFY changed)
    Q_PROPERTY(QString themeName READ themeName NOTIFY changed)
    Q_PROPERTY(qreal textScale READ textScale NOTIFY textScaleChanged)
    Q_PROPERTY(qreal pointsPerPixel READ pointsPerPixel CONSTANT)
    Q_PROPERTY(QString monoFamily READ monoFamily CONSTANT)
    Q_PROPERTY(QString sansFamily READ sansFamily CONSTANT)
    Q_PROPERTY(QString qtVersion READ qtVersion CONSTANT)

#define OMARCHY_DECLARE_ROLE(name) Q_PROPERTY(QColor name READ name NOTIFY changed)
    OMARCHY_COLOR_ROLES(OMARCHY_DECLARE_ROLE)
#undef OMARCHY_DECLARE_ROLE

public:
    // No default constructor: with one, the QML engine would build its own
    // instance instead of calling create(), and main()'s scale/fonts would be lost.
    explicit Palette(QObject *parent);

    static Palette *instance();
    static Palette *create(QQmlEngine *, QJSEngine *);

    bool dark() const { return m_dark; }
    QString themeName() const { return m_themeName; }
    qreal textScale() const { return m_textScale; }
    void setTextScale(qreal scale);
    qreal pointsPerPixel() const { return m_pointsPerPixel; }
    void setPointsPerPixel(qreal value) { m_pointsPerPixel = value; }
    QString monoFamily() const { return QStringLiteral("monospace"); }
    QString sansFamily() const { return m_sansFamily; }
    // Call before the QML engine loads if you bundle a UI face; otherwise
    // the fontconfig sans-serif alias is used.
    void setSansFamily(const QString &family) { m_sansFamily = family; }
    QString qtVersion() const;

#define OMARCHY_DEFINE_ROLE(name) QColor name() const { return m_roles.value(QStringLiteral(#name)); }
    OMARCHY_COLOR_ROLES(OMARCHY_DEFINE_ROLE)
#undef OMARCHY_DEFINE_ROLE

    // Look a role up by name; used by the model layer to colour tooltip rows
    // and data series without QML having to know every mapping.
    Q_INVOKABLE QColor role(const QString &name) const;
    Q_INVOKABLE QString hex(const QString &name) const { return role(name).name(); }

    // Directory holding colors.toml. Defaults to the Omarchy current theme,
    // overridable with OMARCHY_THEME_DIR for testing a theme without switching.
    QString themeDir() const { return m_themeDir; }

signals:
    void changed();
    void textScaleChanged();

private:
    void load();
    void watch();
    void rebuild();

    static Palette *s_instance;
    QString m_themeDir;
    QString m_themeName;
    bool m_dark = true;
    qreal m_textScale = 1.0;
    qreal m_pointsPerPixel = 0.75;
    QString m_sansFamily = QStringLiteral("sans-serif");
    QHash<QString, QColor> m_raw;
    QHash<QString, QColor> m_roles;
    QFileSystemWatcher m_watcher;
};
