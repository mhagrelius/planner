import QtQuick
import Planner

// The 18 px box: a 2 px ring in the priority colour (surface1 when unset),
// or filled with the state colour and a check glyph when done or selected.
Item {
    id: box
    property string ringRole: ""       // priority role, empty for unset
    property bool checked: false
    property bool selected: false
    property real px: 18
    signal clicked()
    width: T.s(px)
    height: T.s(px)
    Rectangle {
        anchors.fill: parent
        radius: T.s(5)
        color: box.selected ? T.caution : box.checked ? T.positive : "transparent"
        border.width: (box.selected || box.checked) ? 0 : Math.max(1, T.s(2))
        border.color: box.ringRole.length ? T.role(box.ringRole) : T.surface1
        Behavior on color { ColorAnimation { duration: 120 } }
        Icon { anchors.centerIn: parent; visible: box.selected || box.checked; name: "object-select"; px: box.px * 12 / 18; tint: T.onFill(box.selected ? T.caution : T.positive) }
    }
    MouseArea { anchors.fill: parent; anchors.margins: -T.s(4); cursorShape: Qt.PointingHandCursor; onClicked: box.clicked() }
}
