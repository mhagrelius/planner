#include "icons.h"

#include <QFile>
#include <QPainter>
#include <QRegularExpression>
#include <QSvgRenderer>

QImage IconProvider::requestImage(const QString &id, QSize *size, const QSize &requestedSize) {
    // id: "<name>/<#rrggbb>"; the colour may carry alpha as #aarrggbb.
    const int slash = id.indexOf(u'/');
    const QString name = slash < 0 ? id : id.left(slash);
    const QString colour = slash < 0 ? QStringLiteral("#ffffff") : id.mid(slash + 1);
    QFile file(QStringLiteral(":/qt/qml/Planner/src/app/icons/%1.svg").arg(name));
    const int px = requestedSize.isValid() ? std::max(requestedSize.width(), requestedSize.height()) : 32;
    QImage image(px, px, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);
    if (size) *size = image.size();
    if (!file.open(QIODevice::ReadOnly)) return image;
    QString svg = QString::fromUtf8(file.readAll());
    // Every icon in the set carries one fill on its path(s).
    svg.replace(QRegularExpression(QStringLiteral("fill=\"#[0-9a-fA-F]{3,8}\"")), QStringLiteral("fill=\"%1\"").arg(colour));
    QSvgRenderer renderer(svg.toUtf8());
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing);
    renderer.render(&painter, QRectF(0, 0, px, px));
    return image;
}
