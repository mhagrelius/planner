import QtQuick
import Planner
import "../components"

// The rail: wordmark, the five built-in views with their number keycaps, then
// saved filters and the project tree. The selected row is filled surface1
// and its keycap takes the accent — no left stripes anywhere.
Rectangle {
    id: rail
    width: T.s(236)
    color: T.sidebar

    Column {
        anchors.fill: parent
        Item {
            width: parent.width
            height: T.s(44)
            Mono { x: T.s(16); anchors.verticalCenter: parent.verticalCenter; anchors.verticalCenterOffset: T.s(2); text: "planner"; px: 12; color: T.hintText
                   font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(12 * 0.16) }
            // The summon binding when Hyprland has one; the selection count while selecting.
            Rectangle {
                anchors.right: parent.right; anchors.rightMargin: T.s(16); anchors.verticalCenter: parent.verticalCenter; anchors.verticalCenterOffset: T.s(2)
                visible: App.selecting || App.summonKey.length > 0
                color: App.selecting ? T.caution : T.surface0
                radius: T.s(4)
                width: badge.implicitWidth + T.s(12)
                height: badge.implicitHeight + T.s(4)
                Mono { id: badge; anchors.centerIn: parent; px: 10; text: App.selecting ? App.selectionCount + " SELECTED" : App.summonKey
                       color: App.selecting ? T.onFill(T.caution) : T.meta }
            }
        }
        VScroll {
            width: parent.width
            height: parent.height - T.s(44)
            Column {
                width: parent.width
                Repeater {
                    model: App.rail
                    delegate: Loader {
                        required property var modelData
                        width: parent.width
                        sourceComponent: modelData.kind === "heading" ? heading : modelData.kind === "new-project" ? newProject : row
                        property var item: modelData
                    }
                }
                Item { width: 1; height: T.s(8) }
            }
        }
    }

    Component {
        id: heading
        Item {
            width: parent ? parent.width : 0
            height: T.s(16) + T.s(12) + T.s(6)
            Mono { x: T.s(16); y: T.s(16); text: item.title; px: 10; color: T.hintText; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(10 * 0.16) }
        }
    }

    Component {
        id: newProject
        Item {
            width: parent ? parent.width : 0
            height: T.s(34) + T.s(2)
            Rectangle {
                x: T.s(8); width: parent.width - T.s(16); height: T.s(34)
                radius: T.s(8)
                color: addHover.containsMouse ? T.hover : "transparent"
                MouseArea { id: addHover; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: App.newProject() }
                Row {
                    anchors.fill: parent; anchors.leftMargin: T.s(12); anchors.rightMargin: T.s(12)
                    spacing: T.s(10)
                    Icon { anchors.verticalCenter: parent.verticalCenter; name: "folder-new"; px: 15; tint: addHover.containsMouse ? T.text2 : T.hintText }
                    Mono { anchors.verticalCenter: parent.verticalCenter; text: "+ " + item.title; px: 12; color: addHover.containsMouse ? T.text2 : T.hintText }
                }
            }
        }
    }

    Component {
        id: row
        Item {
            width: parent ? parent.width : 0
            height: T.s(34) + T.s(2)
            Rectangle {
                x: T.s(8); width: parent.width - T.s(16); height: T.s(34)
                radius: T.s(8)
                color: item.selected ? T.surface1 : hover.containsMouse ? T.hover : "transparent"
                Behavior on color { ColorAnimation { duration: 120 } }
                MouseArea {
                    id: hover
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    cursorShape: Qt.PointingHandCursor
                    // The menu reaches the row it was opened on, so no need to visit it first.
                    onClicked: function (mouse) { if (mouse.button === Qt.RightButton) App.openRowMenu(item.id); else App.go(item.id) }
                }
                Row {
                    anchors.fill: parent
                    anchors.leftMargin: T.s(12) + item.depth * T.s(16)
                    anchors.rightMargin: T.s(12)
                    spacing: T.s(10)
                    // Built-in views carry a number keycap; projects a colour dot.
                    Rectangle {
                        visible: item.number > 0
                        anchors.verticalCenter: parent.verticalCenter
                        radius: T.s(4)
                        width: num.implicitWidth + T.s(12); height: num.implicitHeight + T.s(2)
                        color: item.selected ? (App.selecting ? T.caution : T.accent) : T.surface0
                        Mono { id: num; anchors.centerIn: parent; text: item.number; px: 11; color: item.selected ? T.onFill(App.selecting ? T.caution : T.accent) : T.meta }
                    }
                    Icon { visible: item.number > 0; anchors.verticalCenter: parent.verticalCenter; name: item.icon; px: 15; tint: item.selected ? T.text : T.hintText }
                    Item {
                        visible: item.number === 0
                        width: T.s(15); height: T.s(15)
                        anchors.verticalCenter: parent.verticalCenter
                        Rectangle { anchors.centerIn: parent; width: T.s(7); height: T.s(7); radius: T.s(2); color: item.color.length ? T.role(item.color) : T.hintText }
                    }
                    Sans {
                        anchors.verticalCenter: parent.verticalCenter
                        width: parent.width - T.s(15) - (item.number > 0 ? num.implicitWidth + T.s(12) + T.s(10) : 0) - count.width - T.s(20)
                        text: item.title
                        px: 14
                        elide: Text.ElideRight
                        color: item.selected ? T.text : T.text2
                        weight: item.selected ? Font.Medium : Font.Normal
                    }
                    Mono { id: count; anchors.verticalCenter: parent.verticalCenter; visible: item.count > 0; text: item.count > 0 ? item.count : ""; px: 12; color: item.selected ? T.text : T.meta }
                }
            }
        }
    }
}
