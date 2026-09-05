#pragma once

#include <QObject>

#include <functional>

class QDBusVariant;

// Desktop-wide appearance knobs, read from the xdg-desktop-portal Settings
// interface the way omacalc does: the colour scheme from
// org.freedesktop.appearance and GNOME's text-scaling-factor, which
// `omarchy display text size` drives. Both arrive asynchronously and update
// live through the SettingChanged signal.
class SystemTheme : public QObject {
    Q_OBJECT

public:
    explicit SystemTheme(QObject *parent = nullptr);

    bool darkMode() const { return m_darkMode; }
    qreal textScale() const { return m_textScale; }

signals:
    void darkModeChanged(bool darkMode);
    void textScaleChanged(qreal textScale);

public slots:
    void refresh();

private slots:
    void handlePortalSettingChanged(const QString &nameSpace, const QString &key,
                                    const QDBusVariant &value);

private:
    void requestPortalSetting(const QString &nameSpace, const QString &key,
                              std::function<void(const QVariant &)> handler);
    void requestPortalDarkMode();
    void requestPortalTextScale();
    void setDarkMode(bool darkMode);
    void setTextScale(qreal textScale);

    bool m_darkMode = true;
    qreal m_textScale = 1.0;
};
