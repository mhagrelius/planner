// Symbolic icons recoloured at render time.
//
// The Adwaita symbolic set the design names is shipped as single-fill SVGs;
// `image://icon/<name>/<#rrggbb>` renders one in a palette colour, so a row
// never ships per-colour copies and re-tints when the theme changes.
#pragma once

#include <QQuickImageProvider>

class IconProvider : public QQuickImageProvider {
public:
    IconProvider() : QQuickImageProvider(QQuickImageProvider::Image) {}
    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;
};
