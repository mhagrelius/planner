import QtQuick
import Planner
import "../components"

// The view title, its actual query in mono, and the global keycap hints or the
// list/board toggle. Showing the query is deliberate: every view is a query,
// and this is where that becomes visible.
Item {
    id: header
    height: T.s(54)
    Row {
        anchors.left: parent.left; anchors.leftMargin: T.s(20)
        anchors.top: parent.top; anchors.topMargin: T.s(18)
        spacing: T.s(14)
        Sans { id: title; text: App.viewTitle; px: 20; weight: Font.DemiBold; font.letterSpacing: T.r(-0.2) }
        Mono { text: App.viewSubtitle; px: 12; color: T.meta; anchors.baseline: title.baseline }
    }
    Row {
        anchors.right: parent.right; anchors.rightMargin: T.s(20)
        anchors.verticalCenter: parent.verticalCenter
        anchors.verticalCenterOffset: T.s(3)
        spacing: T.s(14)
        visible: !App.isProject || App.selecting
        Repeater {
            model: App.headerHints
            Keycap { required property var modelData; key: modelData.key; text: modelData.text
                     fill: modelData.accent.length ? T.role(modelData.accent) : T.surface0
                     keyColor: modelData.accent.length ? T.onFill(T.role(modelData.accent)) : T.text }
        }
    }
    // list | board, on a mantle track.
    Rectangle {
        visible: App.isProject && !App.selecting
        anchors.right: parent.right; anchors.rightMargin: T.s(20)
        anchors.verticalCenter: parent.verticalCenter
        anchors.verticalCenterOffset: T.s(3)
        color: T.sidebar
        radius: T.s(8)
        width: toggle.implicitWidth + T.s(4)
        height: toggle.implicitHeight + T.s(4)
        Row {
            id: toggle
            anchors.centerIn: parent
            spacing: T.s(2)
            Repeater {
                model: [{id: "list", icon: "view-list", on: !App.board}, {id: "board", icon: "view-grid", on: App.board}]
                Rectangle {
                    required property var modelData
                    radius: T.s(6)
                    color: modelData.on ? T.surface0 : "transparent"
                    width: inner.implicitWidth + T.s(20); height: inner.implicitHeight + T.s(10)
                    Row {
                        id: inner; anchors.centerIn: parent; spacing: T.s(6)
                        Icon { name: modelData.icon; px: 13; tint: modelData.on ? T.text : T.hintText; anchors.verticalCenter: parent.verticalCenter }
                        Mono { text: modelData.id; px: 11; color: modelData.on ? T.text : T.meta }
                    }
                    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: if (!modelData.on) App.toggleStyle() }
                }
            }
        }
    }
}
