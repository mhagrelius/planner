import QtQuick
import Planner

// A section name as an uppercase mono lane header with its count and a
// hairline running to the right edge. Lane headers, not boxes — boxes are
// for the board.
Item {
    id: lane
    property string title: ""
    property int count: 0
    width: parent ? parent.width : 0
    height: T.s(11) + T.s(18) + T.s(8)
    Row {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: T.s(20)
        anchors.rightMargin: T.s(20)
        anchors.top: parent.top
        anchors.topMargin: T.s(18)
        spacing: T.s(10)
        Mono { id: name; text: lane.title; px: 11; color: T.text2; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(11 * 0.14); anchors.verticalCenter: parent.verticalCenter }
        Mono { id: n; text: lane.count; px: 11; color: T.hintText; anchors.verticalCenter: parent.verticalCenter }
        Rectangle { width: parent.width - name.width - n.width - T.s(20); height: T.line; color: T.surface0; anchors.verticalCenter: parent.verticalCenter }
    }
}
