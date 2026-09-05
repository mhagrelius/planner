import QtQuick
import Planner
import "../components"

// Everything about one task. The pane carries no field-name labels: the icon
// is the label and the value is the content. Title and description are
// edited in place; text is set imperatively when the task changes, never
// bound, so a refresh cannot wipe what is being typed.
Rectangle {
    id: pane
    width: T.s(372)
    color: T.sidebar
    readonly property var d: App.detail
    readonly property string taskId: d.id || ""
    property string shownId: ""
    Rectangle { width: T.line; height: parent.height; color: T.surface0 }

    function reset() {
        if (shownId === taskId) return
        shownId = taskId
        titleField.text = d.title || ""
        descriptionField.text = d.description || ""
        subtaskField.text = ""
        labelField.text = ""
        noteField.text = ""
    }
    onDChanged: reset()
    Component.onCompleted: reset()

    function focusSubtask() { subtaskField.forceActiveFocus() }
    function focusNote() { noteField.forceActiveFocus() }

    Column {
        anchors.fill: parent
        Item {
            width: parent.width
            height: T.s(44)
            Row {
                anchors.fill: parent; anchors.leftMargin: T.s(18); anchors.rightMargin: T.s(18); anchors.topMargin: T.s(16)
                spacing: T.s(10)
                Mono { text: pane.d.parent ? "subtask" : "task"; px: 10; color: T.hintText; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(10 * 0.16)
                       width: parent.width - pin.width - del.width - T.s(20); anchors.verticalCenter: parent.verticalCenter }
                Keycap { id: pin; key: pane.d.pinned ? "ctrl+shift+p unpin" : "ctrl+shift+p pin"; keyColor: T.meta; anchors.verticalCenter: parent.verticalCenter }
                Keycap { id: del; key: "del"; keyColor: T.negative; anchors.verticalCenter: parent.verticalCenter }
            }
        }
        VScroll {
            width: parent.width
            height: parent.height - T.s(44)
            Column {
                x: T.s(18)
                width: parent.width - T.s(36)
                spacing: T.s(20)

                // A parent link, for a subtask opened from its parent.
                Row {
                    visible: !!pane.d.parent
                    spacing: T.s(8)
                    Icon { name: "go-next"; px: 12; rotation: 180; anchors.verticalCenter: parent.verticalCenter }
                    Mono { text: pane.d.parentTitle || ""; px: 12; color: T.meta; anchors.verticalCenter: parent.verticalCenter
                           MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.openTaskId(pane.d.parent) } }
                }

                TextEdit {
                    id: titleField
                    width: parent.width
                    font.family: T.sans
                    font.pointSize: T.f(19)
                    font.weight: Font.DemiBold
                    font.strikeout: pane.d.checked || false
                    color: pane.d.checked ? T.meta : T.text
                    selectionColor: T.activeFill
                    selectedTextColor: T.activeText
                    wrapMode: TextEdit.Wrap
                    activeFocusOnTab: true
                    Keys.onReturnPressed: function(event) { App.setTitle(pane.taskId, text); pane.forceActiveFocus(); event.accepted = true }
                    Keys.onEscapePressed: function(event) { text = pane.d.title; pane.forceActiveFocus(); event.accepted = true }
                    onActiveFocusChanged: if (!activeFocus) App.setTitle(pane.taskId, text)
                }

                TextEdit {
                    id: descriptionField
                    width: parent.width
                    font.family: T.sans
                    font.pointSize: T.f(14)
                    color: T.text2
                    selectionColor: T.activeFill
                    selectedTextColor: T.activeText
                    wrapMode: TextEdit.Wrap
                    activeFocusOnTab: true
                    textFormat: TextEdit.PlainText
                    Mono { visible: !descriptionField.text.length && !descriptionField.activeFocus; text: "add a description"; px: 13; color: T.hintText }
                    Keys.onEscapePressed: function(event) { pane.forceActiveFocus(); event.accepted = true }
                    onActiveFocusChanged: if (!activeFocus) App.setDescription(pane.taskId, text)
                }

                // Schedule, deadline, priority, project + labels: icon and value, no headings.
                Column {
                    width: parent.width
                    spacing: T.s(10)
                    Rectangle { width: parent.width; height: T.line; color: T.surface0 }
                    Item { width: 1; height: T.s(0) }
                    Item {
                        width: parent.width; height: scheduleRow.implicitHeight
                        Row {
                            id: scheduleRow
                            spacing: T.s(10)
                            Icon { name: "x-office-calendar"; anchors.verticalCenter: parent.verticalCenter }
                            Mono { text: pane.d.schedule || ""; px: 13; color: pane.d.hasSchedule ? (pane.d.scheduleRole ? T.role(pane.d.scheduleRole) : T.text2) : T.hintText
                                   anchors.verticalCenter: parent.verticalCenter }
                            Keycap { key: "ctrl+d"; px: 10; keyColor: T.meta; anchors.verticalCenter: parent.verticalCenter }
                        }
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.openDatePicker() }
                    }
                    Item {
                        width: parent.width; height: deadlineRow.implicitHeight
                        Row {
                            id: deadlineRow
                            spacing: T.s(10)
                            Icon { name: "alarm"; anchors.verticalCenter: parent.verticalCenter }
                            Mono { text: pane.d.deadline || ""; px: 13; anchors.verticalCenter: parent.verticalCenter
                                   color: pane.d.hasDeadline ? (pane.d.deadlineRole ? T.role(pane.d.deadlineRole) : T.text2) : T.hintText }
                        }
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.openDeadlinePicker() }
                    }
                    Item {
                        width: parent.width; height: priorityRow.implicitHeight
                        Row {
                            id: priorityRow
                            spacing: T.s(10)
                            Item {
                                width: T.s(15); height: T.s(15); anchors.verticalCenter: parent.verticalCenter
                                Rectangle { anchors.centerIn: parent; width: T.s(9); height: T.s(9); radius: T.s(3); color: "transparent"; border.width: Math.max(1, T.s(2))
                                            border.color: pane.d.priorityRole ? T.role(pane.d.priorityRole) : T.surface1 }
                            }
                            Mono { text: pane.d.priority || ""; px: 13; color: pane.d.priorityRole ? T.role(pane.d.priorityRole) : T.text2; anchors.verticalCenter: parent.verticalCenter }
                            Mono { text: "ctrl+1-4"; px: 10; color: T.hintText; anchors.verticalCenter: parent.verticalCenter }
                        }
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.cyclePriority(pane.taskId) }
                    }
                    Flow {
                        width: parent.width
                        spacing: T.s(8)
                        Icon { name: "folder"; anchors.verticalCenter: undefined; y: T.s(2) }
                        Mono { text: (pane.d.project || "") + (pane.d.section ? " " + pane.d.section : ""); px: 13; color: T.text2 }
                        Repeater {
                            model: pane.d.labels || []
                            LabelChip {
                                required property string modelData
                                text: "@" + modelData
                                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.toggleLabel(pane.taskId, modelData, false) }
                            }
                        }
                        // Type a label and press Enter; it is created if new.
                        Rectangle {
                            color: labelField.activeFocus ? T.window : "transparent"
                            radius: T.s(4)
                            border.width: T.line
                            border.color: labelField.activeFocus ? T.accent : T.surface1
                            width: Math.max(T.s(28), labelField.contentWidth + T.s(14))
                            height: T.s(20)
                            TextInput {
                                id: labelField
                                anchors.fill: parent; anchors.leftMargin: T.s(7); anchors.rightMargin: T.s(7)
                                verticalAlignment: TextInput.AlignVCenter
                                font.family: T.mono; font.pointSize: T.f(12); color: T.text
                                activeFocusOnTab: true
                                Mono { visible: !labelField.text.length; text: labelField.activeFocus ? "@label" : "+"; px: 12; color: T.hintText; anchors.verticalCenter: parent.verticalCenter }
                                onAccepted: { var name = text.replace(/^@/, ""); text = ""; App.toggleLabel(pane.taskId, name, true) }
                                Keys.onEscapePressed: function(event) { text = ""; pane.forceActiveFocus(); event.accepted = true }
                            }
                        }
                    }
                    Row {
                        visible: (pane.d.reminders || []).length > 0
                        spacing: T.s(10)
                        Icon { name: "alarm"; anchors.verticalCenter: parent.verticalCenter; opacity: 0.6 }
                        Mono { text: (pane.d.reminders || []).join(" · "); px: 12; color: T.meta; anchors.verticalCenter: parent.verticalCenter }
                    }
                }

                // Subtasks: a count, the rows, and the add affordance on its own hairline.
                Column {
                    width: parent.width
                    spacing: T.s(8)
                    Rectangle { width: parent.width; height: T.line; color: T.surface0 }
                    Mono { visible: !!pane.d.subtaskLine; text: pane.d.subtaskLine || ""; px: 12; color: T.meta; topPadding: T.s(6) }
                    Repeater {
                        model: pane.d.subtasks || []
                        Item {
                            required property var modelData
                            width: parent.width
                            height: T.s(30)
                            Row {
                                anchors.fill: parent
                                spacing: T.s(12)
                                Check { anchors.verticalCenter: parent.verticalCenter; checked: modelData.checked; onClicked: App.toggleTask(modelData.id) }
                                Sans { width: parent.width - T.s(18) - T.s(14) - T.s(24); text: modelData.content; px: 14; elide: Text.ElideRight
                                       color: modelData.checked ? T.meta : T.text2; font.strikeout: modelData.checked; anchors.verticalCenter: parent.verticalCenter }
                                Icon { name: "go-next"; px: 14; anchors.verticalCenter: parent.verticalCenter }
                            }
                            MouseArea { anchors.fill: parent; anchors.leftMargin: T.s(30); cursorShape: Qt.PointingHandCursor; onClicked: App.openTaskId(modelData.id) }
                        }
                    }
                    Rectangle { width: parent.width; height: T.line; color: T.surface0 }
                    Row {
                        width: parent.width
                        spacing: T.s(10)
                        Keycap { key: "ctrl+enter"; fill: T.positive; keyColor: T.onFill(T.positive); anchors.verticalCenter: parent.verticalCenter }
                        TextInput {
                            id: subtaskField
                            width: parent.width - T.s(90)
                            font.family: T.mono; font.pointSize: T.f(12); color: T.text
                            selectionColor: T.activeFill; selectedTextColor: T.activeText
                            activeFocusOnTab: true
                            anchors.verticalCenter: parent.verticalCenter
                            Mono { visible: !subtaskField.text.length; text: "add a subtask"; px: 12; color: T.meta; anchors.verticalCenter: parent.verticalCenter }
                            onAccepted: { App.addSubtask(pane.taskId, text); text = "" }
                            Keys.onEscapePressed: function(event) { text = ""; pane.forceActiveFocus(); event.accepted = true }
                        }
                    }
                }
                // Activity: dated notes, oldest first, and a field to add one.
                Column {
                    width: parent.width
                    spacing: T.s(8)
                    Rectangle { width: parent.width; height: T.line; color: T.surface0 }
                    Mono { text: "activity"; px: 10; color: T.hintText; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(10 * 0.16); topPadding: T.s(6) }
                    Repeater {
                        model: pane.d.notes || []
                        Item {
                            id: noteRow
                            required property var modelData
                            width: parent.width
                            height: noteText.implicitHeight + T.s(20)
                            Mono { id: stamp; x: 0; y: T.s(2); text: noteRow.modelData.when; px: 11; color: T.meta }
                            Sans { id: noteText; y: stamp.implicitHeight + T.s(4); width: parent.width - T.s(24); text: noteRow.modelData.text; px: 14; color: T.text2; wrapMode: Text.Wrap; lineHeight: 1.35 }
                            // A wrong note is deleted from its row; nothing is edited in place.
                            Icon { anchors.right: parent.right; y: T.s(2); name: "edit-clear"; px: 12; opacity: noteHover.containsMouse ? 0.9 : 0
                                   MouseArea { anchors.fill: parent; anchors.margins: -T.s(6); cursorShape: Qt.PointingHandCursor; onClicked: App.removeNote(pane.taskId, noteRow.modelData.at) } }
                            MouseArea { id: noteHover; anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.NoButton }
                        }
                    }
                    // A plain field, the key named quietly at its far end until it is used.
                    Item {
                        width: parent.width
                        height: T.s(28)
                        TextInput {
                            id: noteField
                            anchors.left: parent.left
                            anchors.right: noteHint.visible ? noteHint.left : parent.right
                            anchors.rightMargin: noteHint.visible ? T.s(10) : 0
                            anchors.verticalCenter: parent.verticalCenter
                            font.family: T.sans; font.pointSize: T.f(14); color: T.text
                            selectionColor: T.activeFill; selectedTextColor: T.activeText
                            activeFocusOnTab: true
                            Sans { visible: !noteField.text.length; text: noteField.activeFocus ? "what happened?" : "add a note"; px: 14; color: T.hintText; anchors.verticalCenter: parent.verticalCenter }
                            onAccepted: { App.addNote(pane.taskId, text); text = "" }
                            Keys.onEscapePressed: function(event) { text = ""; pane.forceActiveFocus(); event.accepted = true }
                        }
                        Mono { id: noteHint; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; visible: !noteField.activeFocus && !noteField.text.length
                               text: noteField.activeFocus ? "enter adds" : "ctrl+shift+enter"; px: 10; color: T.meta }
                    }
                }
                Item { width: 1; height: T.s(18) }
            }
        }
    }
}
