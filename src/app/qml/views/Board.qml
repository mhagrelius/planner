import QtQuick
import Planner
import "../components"

// The same sections sideways: one column per section, equal widths, every
// column a target for Ctrl+←→. The column holding the cursor has the surface1
// border; the cursor card is filled surface0 with an accent outline.
Item {
    id: board
    Row {
        anchors.fill: parent
        anchors.leftMargin: T.s(20); anchors.rightMargin: T.s(20)
        anchors.topMargin: T.s(6); anchors.bottomMargin: T.s(20)
        spacing: T.s(14)
        Repeater {
            model: App.lanes
            Rectangle {
                id: column
                required property var modelData
                required property int index
                width: (parent.width - T.s(14) * (App.lanes.length - 1)) / App.lanes.length
                height: parent.height
                color: T.sidebar
                radius: T.s(10)
                border.width: T.line
                border.color: modelData.current ? T.surface1 : T.surface0
                Behavior on border.color { ColorAnimation { duration: 120 } }
                MouseArea { anchors.fill: parent; onClicked: App.moveColumn(column.index - App.boardColumn) }
                Column {
                    anchors.fill: parent
                    Item {
                        width: parent.width
                        height: T.s(38)
                        Row {
                            anchors.fill: parent; anchors.leftMargin: T.s(14); anchors.rightMargin: T.s(14); anchors.topMargin: T.s(12)
                            Mono { text: column.modelData.name; px: 11; color: column.modelData.current ? T.text : T.text2; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(11 * 0.14)
                                   width: parent.width - T.s(20); elide: Text.ElideRight }
                            Mono { text: column.modelData.count; px: 11; color: T.hintText; width: T.s(20); horizontalAlignment: Text.AlignRight }
                        }
                        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.surface0 }
                    }
                    VScroll {
                        width: parent.width
                        height: parent.height - T.s(38)
                        padding: T.s(8)
                        Column {
                            width: parent.width
                            spacing: T.s(8)
                            Repeater {
                                model: column.modelData.tasks
                                Rectangle {
                                    id: card
                                    required property var modelData
                                    width: parent.width
                                    height: body.implicitHeight + T.s(20)
                                    radius: T.s(8)
                                    color: modelData.cursor || modelData.selected ? T.surface0 : T.window
                                    border.width: modelData.cursor ? T.line : 0
                                    border.color: modelData.cursor && App.selecting ? T.caution : T.accent
                                    MouseArea { anchors.fill: parent; onClicked: App.cursorTo(card.modelData.flat); onDoubleClicked: App.openTaskId(card.modelData.id) }
                                    Row {
                                        anchors.fill: parent; anchors.margins: T.s(10)
                                        spacing: T.s(10)
                                        Check { y: T.s(2); px: 16; ringRole: card.modelData.priorityRole; checked: card.modelData.checked; selected: card.modelData.selected
                                                onClicked: App.selecting ? App.toggleSelected(card.modelData.id) : App.toggleTask(card.modelData.id) }
                                        Column {
                                            id: body
                                            width: parent.width - T.s(26)
                                            spacing: T.s(4)
                                            Sans { width: parent.width; text: card.modelData.content; px: 14; wrapMode: Text.Wrap; lineHeight: 1.25
                                                   color: card.modelData.checked ? T.meta : T.text; font.strikeout: card.modelData.checked }
                                            Mono { visible: card.modelData.due.length > 0; text: card.modelData.due; px: 12
                                                   color: card.modelData.dueRole.length ? T.role(card.modelData.dueRole) : T.text2 }
                                            Flow {
                                                width: parent.width; spacing: T.s(6)
                                                visible: card.modelData.labels.length > 0 || card.modelData.deadline.length > 0 || card.modelData.subtasks.length > 0
                                                Mono { visible: card.modelData.subtasks.length > 0; text: card.modelData.subtasks; px: 12; color: T.meta }
                                                Mono { visible: card.modelData.deadline.length > 0; text: card.modelData.deadline; px: 12; color: card.modelData.deadlineRole.length ? T.role(card.modelData.deadlineRole) : T.meta }
                                                Repeater { model: card.modelData.labels; LabelChip { required property string modelData; text: "@" + modelData } }
                                            }
                                        }
                                    }
                                }
                            }
                            EmptyLane { visible: column.modelData.tasks.length === 0; width: parent.width }
                        }
                    }
                }
            }
        }
    }
}
