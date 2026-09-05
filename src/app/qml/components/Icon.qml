import QtQuick
import Planner

// A symbolic icon in a palette colour, rendered by the icon provider.
Image {
    property string name: "folder"
    property color tint: T.hintText
    property real px: 15
    width: T.s(px)
    height: T.s(px)
    sourceSize: Qt.size(T.s(px) * 2, T.s(px) * 2)
    source: name.length ? "image://icon/" + name + "/" + T.hex(tint) : ""
    smooth: true
    mipmap: true
    fillMode: Image.PreserveAspectFit
}
