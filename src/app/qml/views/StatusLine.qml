import QtQuick
import Planner
import "../components"

// The status line: what is shown, how much of it, and the keys that act on
// it. In a selection its right half becomes the bulk actions — nothing slides
// in from the bottom.
Rectangle {
    id: line
    height: T.s(30)
    color: T.sidebar
    Rectangle { width: parent.width; height: T.line; color: T.surface0 }
    // The right-hand hints keep their room; the left and middle texts share
    // what is left and elide rather than run under them.
    readonly property real rightWidth: App.status.selecting ? keys.implicitWidth : rightText.implicitWidth
    readonly property real available: Math.max(0, width - rightWidth - T.s(16) - T.s(14) - T.s(26) - T.s(12))
    Row {
        anchors.left: parent.left
        anchors.leftMargin: T.s(16)
        height: parent.height
        Mono { id: leftText; text: App.status.left; px: 12; color: App.status.selecting ? T.caution : T.text2; anchors.verticalCenter: parent.verticalCenter; elide: Text.ElideRight
               width: Math.min(implicitWidth, line.available * (middleText.text.length ? 0.55 : 1)) }
        Item { width: T.s(26); height: 1 }
        Mono { id: middleText; text: App.status.middle; px: 12; anchors.verticalCenter: parent.verticalCenter; elide: Text.ElideRight
               color: App.status.middleRole.length ? T.role(App.status.middleRole) : T.meta
               width: Math.max(0, Math.min(implicitWidth, line.available - leftText.width)) }
    }
    Mono {
        id: rightText
        anchors.right: parent.right; anchors.rightMargin: T.s(14); anchors.verticalCenter: parent.verticalCenter
        visible: !App.status.selecting
        text: App.status.right; px: 12; color: T.hintText
    }
    Row {
        id: keys
        anchors.right: parent.right; anchors.rightMargin: T.s(14); anchors.verticalCenter: parent.verticalCenter
        visible: App.status.selecting
        spacing: T.s(14)
        Repeater {
            model: App.status.keys
            Keycap { required property var modelData; key: modelData.key; text: modelData.text
                     fill: modelData.accent.length ? T.role(modelData.accent) : T.surface1
                     keyColor: modelData.accent.length ? T.onFill(T.role(modelData.accent)) : T.text }
        }
    }
}
