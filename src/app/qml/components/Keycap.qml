import QtQuick
import Planner

// A key name on a square-ish chip, with an optional word after it: `ctrl+n add`.
Row {
    id: cap
    property string key: ""
    property string text: ""
    property color fill: T.surface0
    property color keyColor: T.text
    property color textColor: T.hintText
    property real px: 11
    spacing: T.s(5)
    Rectangle {
        color: cap.fill
        radius: T.s(4)
        width: label.implicitWidth + T.s(12)
        height: label.implicitHeight + T.s(2)
        anchors.verticalCenter: parent.verticalCenter
        Mono { id: label; anchors.centerIn: parent; text: cap.key; px: cap.px; color: cap.keyColor }
    }
    Mono { visible: cap.text.length > 0; text: cap.text; px: cap.px; color: cap.textColor; anchors.verticalCenter: parent.verticalCenter }
}
