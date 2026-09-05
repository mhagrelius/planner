import QtQuick
import Planner

// A label or a small mono value on a square-ish chip: `@email`.
Rectangle {
    id: chip
    property string text: ""
    property color fg: T.text2
    property real px: 12
    property string icon: ""
    signal clicked()
    color: T.surface0
    radius: T.s(4)
    width: row.implicitWidth + T.s(12)
    height: label.implicitHeight + T.s(2)
    Row {
        id: row
        anchors.centerIn: parent
        spacing: T.s(6)
        Icon { visible: chip.icon.length > 0; name: chip.icon; px: 12; tint: T.hintText; anchors.verticalCenter: parent.verticalCenter }
        Mono { id: label; text: chip.text; px: chip.px; color: chip.fg }
    }
    MouseArea { anchors.fill: parent; enabled: false; onClicked: chip.clicked() }
}
