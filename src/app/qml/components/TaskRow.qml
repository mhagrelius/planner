import QtQuick
import Planner

// One task in a list: ring, title, meta, labels, and the due column.
// Renders and reports; the App decides what a click means.
Rectangle {
    id: row
    required property var modelData
    property bool showHints: true
    readonly property bool isCursor: modelData.cursor
    readonly property bool isSelected: modelData.selected
    readonly property bool selecting: App.selecting
    // What is left for the title once the ring, the meta and the due column have their share.
    readonly property real free: width - T.s(40) - T.s(18) - T.s(12) - T.s(104) - T.s(12) - (meta.implicitWidth > 0 ? meta.implicitWidth + T.s(12) : 0)
    width: parent ? parent.width : 0
    height: T.s(45)
    color: isSelected || (isCursor && !selecting) ? T.surface0 : "transparent"
    Behavior on color { ColorAnimation { duration: 120 } }
    // The cursor inside a selection is the row with the yellow outline.
    Rectangle { anchors.fill: parent; color: "transparent"; visible: row.isCursor && row.selecting; border.width: T.line; border.color: T.caution }

    MouseArea {
        anchors.fill: parent
        onClicked: App.cursorTo(row.modelData.flat)
        onDoubleClicked: App.openTaskId(row.modelData.id)
    }

    Row {
        anchors.fill: parent
        anchors.leftMargin: T.s(20)
        anchors.rightMargin: T.s(20)
        spacing: T.s(12)
        Check {
            anchors.verticalCenter: parent.verticalCenter
            ringRole: row.modelData.priorityRole
            checked: row.modelData.checked
            selected: row.isSelected
            onClicked: row.selecting ? App.toggleSelected(row.modelData.id) : App.toggleTask(row.modelData.id)
        }
        Sans {
            id: title
            anchors.verticalCenter: parent.verticalCenter
            width: Math.max(T.s(80), row.free - (hints.visible ? hints.implicitWidth + T.s(12) : 0))
            text: row.modelData.content
            px: 15
            elide: Text.ElideRight
            color: row.modelData.checked ? T.meta : (row.isCursor && !row.selecting) ? T.text : T.text2
            font.strikeout: row.modelData.checked
        }
        // Keycap hints are visible, not discovered: the cursor row says what it
        // takes, when there is room to say it.
        Row {
            id: hints
            visible: row.isCursor && row.showHints && !row.selecting && row.free - implicitWidth - T.s(12) >= T.s(160)
            spacing: T.s(10)
            anchors.verticalCenter: parent.verticalCenter
            Keycap { key: "space"; text: "done"; fill: T.surface1 }
            Keycap { key: "enter"; text: "open"; fill: T.surface1 }
            Keycap { key: "ctrl+d"; text: "date"; fill: T.surface1 }
        }
        Row {
            id: meta
            anchors.verticalCenter: parent.verticalCenter
            spacing: T.s(12)
            Mono { visible: row.modelData.subtasks.length > 0; text: row.modelData.subtasks; px: 12; color: T.meta; anchors.verticalCenter: parent.verticalCenter }
            Mono { visible: row.modelData.deadline.length > 0; text: row.modelData.deadline; px: 12; anchors.verticalCenter: parent.verticalCenter
                   color: row.modelData.deadlineRole.length ? T.role(row.modelData.deadlineRole) : T.meta }
            Mono { visible: row.modelData.repeat.length > 0; text: row.modelData.repeat; px: 12; color: T.meta; anchors.verticalCenter: parent.verticalCenter }
            Icon { visible: row.modelData.repeat.length > 0; name: "media-playlist-repeat"; px: 14; anchors.verticalCenter: parent.verticalCenter }
            Icon { visible: row.modelData.pinned; name: "view-pin"; px: 14; anchors.verticalCenter: parent.verticalCenter }
            Repeater {
                model: row.modelData.labels
                LabelChip { required property string modelData; text: "@" + modelData; anchors.verticalCenter: parent.verticalCenter
                            color: (row.isCursor || row.isSelected) ? T.surface1 : T.surface0 }
            }
        }
        // The due column is fixed and right-aligned so dates line up down the list.
        Mono {
            id: due
            anchors.verticalCenter: parent.verticalCenter
            width: T.s(104)
            horizontalAlignment: Text.AlignRight
            text: row.modelData.due
            px: 13
            color: row.modelData.dueRole.length ? T.role(row.modelData.dueRole) : T.text2
        }
    }
}
