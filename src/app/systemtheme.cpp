#include "systemtheme.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCall>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusVariant>
#include <QGuiApplication>
#include <QStyleHints>
#include <QVariant>

namespace {

QVariant unwrap(QVariant value) {
    while (value.canConvert<QDBusVariant>())
        value = value.value<QDBusVariant>().variant();
    return value;
}

// org.freedesktop.appearance color-scheme: 0 = no preference, 1 = dark, 2 = light.
bool schemeIsDark(const QVariant &value, bool *known) {
    bool ok = false;
    const uint scheme = unwrap(value).toUInt(&ok);
    if (!ok)
        return false;
    if (scheme == 1) { *known = true; return true; }
    if (scheme == 2) { *known = true; return false; }
    return false;
}

// text-scaling-factor is anchored so the shell's default 12px is 1.0; clamp
// to the range GNOME itself accepts so a stray value cannot wreck layout.
qreal sanitizedScale(const QVariant &value, bool *known) {
    bool ok = false;
    const qreal scale = unwrap(value).toDouble(&ok);
    if (!ok || scale <= 0)
        return 1.0;
    *known = true;
    return qBound(0.5, scale, 3.0);
}

} // namespace

SystemTheme::SystemTheme(QObject *parent) : QObject(parent) {
    // OMARCHY_TEXT_SCALE=1.25 pins the scale for headless checks (no portal).
    bool pinned = false;
    const double pinnedScale = qEnvironmentVariable("OMARCHY_TEXT_SCALE").toDouble(&pinned);
    if (pinned && pinnedScale > 0) {
        m_textScale = qBound(0.5, pinnedScale, 3.0);
        return;
    }
    if (QGuiApplication::styleHints()) {
        const Qt::ColorScheme scheme = QGuiApplication::styleHints()->colorScheme();
        if (scheme == Qt::ColorScheme::Light)
            m_darkMode = false;
        connect(QGuiApplication::styleHints(), &QStyleHints::colorSchemeChanged,
                this, &SystemTheme::refresh);
    }

    QDBusConnection::sessionBus().connect(
        QString(),
        QStringLiteral("/org/freedesktop/portal/desktop"),
        QStringLiteral("org.freedesktop.portal.Settings"),
        QStringLiteral("SettingChanged"),
        this,
        SLOT(handlePortalSettingChanged(QString,QString,QDBusVariant)));

    requestPortalDarkMode();
    requestPortalTextScale();
}

void SystemTheme::refresh() {
    requestPortalDarkMode();
    requestPortalTextScale();
}

void SystemTheme::requestPortalSetting(const QString &nameSpace, const QString &key,
                                       std::function<void(const QVariant &)> handler) {
    const QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected())
        return;

    QDBusMessage request = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.portal.Desktop"),
        QStringLiteral("/org/freedesktop/portal/desktop"),
        QStringLiteral("org.freedesktop.portal.Settings"),
        QStringLiteral("Read"));
    request << nameSpace << key;

    auto *watcher = new QDBusPendingCallWatcher(bus.asyncCall(request), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this,
            [handler = std::move(handler)](QDBusPendingCallWatcher *finished) {
        const QDBusPendingReply<QDBusVariant> reply(*finished);
        finished->deleteLater();
        if (reply.isValid())
            handler(reply.value().variant());
    });
}

void SystemTheme::requestPortalDarkMode() {
    requestPortalSetting(QStringLiteral("org.freedesktop.appearance"),
                         QStringLiteral("color-scheme"),
                         [this](const QVariant &value) {
        bool known = false;
        const bool dark = schemeIsDark(value, &known);
        if (known)
            setDarkMode(dark);
    });
}

void SystemTheme::requestPortalTextScale() {
    requestPortalSetting(QStringLiteral("org.gnome.desktop.interface"),
                         QStringLiteral("text-scaling-factor"),
                         [this](const QVariant &value) {
        bool known = false;
        const qreal scale = sanitizedScale(value, &known);
        if (known)
            setTextScale(scale);
    });
}

void SystemTheme::handlePortalSettingChanged(const QString &nameSpace, const QString &key,
                                             const QDBusVariant &value) {
    if (key == QStringLiteral("text-scaling-factor")) {
        if (nameSpace != QStringLiteral("org.gnome.desktop.interface"))
            return;
        bool known = false;
        const qreal scale = sanitizedScale(value.variant(), &known);
        if (known)
            setTextScale(scale);
        return;
    }

    if (key == QStringLiteral("color-scheme")
            && nameSpace == QStringLiteral("org.freedesktop.appearance")) {
        bool known = false;
        const bool dark = schemeIsDark(value.variant(), &known);
        if (known)
            setDarkMode(dark);
    }
}

void SystemTheme::setDarkMode(bool darkMode) {
    if (m_darkMode == darkMode)
        return;
    m_darkMode = darkMode;
    emit darkModeChanged(m_darkMode);
}

void SystemTheme::setTextScale(qreal textScale) {
    if (qFuzzyCompare(m_textScale, textScale))
        return;
    m_textScale = textScale;
    emit textScaleChanged(m_textScale);
}
