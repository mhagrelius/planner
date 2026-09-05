import QtQuick
import Planner
import "../components"

// Choosing a date, four ways at once: a natural-language field that takes the
// same phrases quick-add does, quick buttons, a Monday-first calendar, a time
// field, and the repeat phrase in a box. Every change is live on the task.
Item {
    id: picker
    anchors.fill: parent
    visible: App.picker.open === true
    readonly property var p: App.picker
    property string shownFor: ""

    function reset() {
        natural.text = ""
        timeField.text = p.time || ""
        repeatField.text = p.repeat || ""
    }
    onVisibleChanged: if (visible) { reset(); natural.forceActiveFocus() }
    Component.onCompleted: if (visible) reset()

    MouseArea { anchors.fill: parent; onClicked: App.closePicker() }
    Rectangle {
        x: Math.round((parent.width - width) / 2)
        y: Math.round((parent.height - height) / 2)
        width: T.s(340)
        height: body.implicitHeight + T.s(28)
        color: T.window
        radius: T.s(12)
        border.width: T.line
        border.color: T.surface1
        MouseArea { anchors.fill: parent }
        Column {
            id: body
            x: T.s(14); y: T.s(14)
            width: parent.width - T.s(28)
            spacing: T.s(12)

            Mono { text: picker.p.mode === "deadline" ? "deadline" : picker.p.mode === "bulk" ? (picker.p.count + " tasks · the day only; each keeps its time and repeat") : "schedule"
                   px: 10; color: T.hintText; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(10 * 0.16) }

            Rectangle {
                width: parent.width
                height: T.s(36)
                color: T.sidebar
                radius: T.s(8)
                border.width: T.line
                border.color: natural.activeFocus ? T.accent : T.surface0
                Row {
                    anchors.fill: parent; anchors.leftMargin: T.s(10); anchors.rightMargin: T.s(10)
                    spacing: T.s(10)
                    Mono { text: "›"; px: 14; color: T.accent; anchors.verticalCenter: parent.verticalCenter }
                    TextInput {
                        id: natural
                        width: parent.width - T.s(24)
                        anchors.verticalCenter: parent.verticalCenter
                        font.family: T.mono; font.pointSize: T.f(13); color: T.text
                        selectionColor: T.activeFill; selectedTextColor: T.activeText
                        Mono { visible: !natural.text.length; text: "next friday, in 3 days…"; px: 13; color: T.hintText }
                        onAccepted: { App.pickerType(text); if (!App.picker.error) text = "" }
                    }
                }
            }

            Row {
                spacing: T.s(6)
                Repeater {
                    model: [{id: "today", label: "Today"}, {id: "tomorrow", label: "Tomorrow"}, {id: "nextweek", label: "Next week"}, {id: "none", label: picker.p.mode === "deadline" ? "No deadline" : "No date"}]
                    Rectangle {
                        required property var modelData
                        radius: T.s(6)
                        color: quick.containsMouse ? T.surface1 : T.surface0
                        width: q.implicitWidth + T.s(20); height: q.implicitHeight + T.s(10)
                        Mono { id: q; anchors.centerIn: parent; text: modelData.label; px: 12; color: modelData.id === "none" ? T.meta : T.text2 }
                        MouseArea { id: quick; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: App.pickerQuick(modelData.id) }
                    }
                }
            }

            Row {
                width: parent.width
                spacing: T.s(8)
                Icon { name: "pan-start"; px: 14; anchors.verticalCenter: parent.verticalCenter; MouseArea { anchors.fill: parent; anchors.margins: -T.s(6); cursorShape: Qt.PointingHandCursor; onClicked: App.pickerMonth(-1) } }
                Mono { width: parent.width - T.s(44); horizontalAlignment: Text.AlignHCenter; text: picker.p.monthLabel || ""; px: 13; color: T.text }
                Icon { name: "pan-end"; px: 14; anchors.verticalCenter: parent.verticalCenter; MouseArea { anchors.fill: parent; anchors.margins: -T.s(6); cursorShape: Qt.PointingHandCursor; onClicked: App.pickerMonth(1) } }
            }

            Grid {
                columns: 7
                columnSpacing: T.s(2); rowSpacing: T.s(2)
                width: parent.width
                readonly property real cell: (width - T.s(12)) / 7
                Repeater {
                    model: ["M", "T", "W", "T", "F", "S", "S"]
                    Mono { required property string modelData; width: parent.cell; horizontalAlignment: Text.AlignHCenter; text: modelData; px: 12; color: T.meta; topPadding: T.s(4); bottomPadding: T.s(4) }
                }
                Repeater {
                    model: picker.p.days || []
                    Rectangle {
                        required property var modelData
                        width: parent.cell; height: T.s(26)
                        radius: T.s(6)
                        color: modelData.selected ? T.accent : dayArea.containsMouse ? T.surface0 : "transparent"
                        Mono { anchors.centerIn: parent; text: modelData.day; px: 12
                               color: modelData.selected ? T.onFill(T.accent) : !modelData.inMonth ? T.surface1 : modelData.today ? T.accent : T.text2 }
                        MouseArea { id: dayArea; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: App.pickerDay(modelData.iso) }
                    }
                }
            }

            Row {
                visible: picker.p.hasTime === true
                width: parent.width
                spacing: T.s(10)
                Mono { text: "time"; px: 12; color: T.hintText; width: T.s(52); anchors.verticalCenter: parent.verticalCenter }
                Rectangle {
                    width: parent.width - T.s(62)
                    height: T.s(30)
                    color: T.sidebar; radius: T.s(6)
                    border.width: T.line; border.color: timeField.activeFocus ? T.accent : T.surface0
                    TextInput {
                        id: timeField
                        anchors.fill: parent; anchors.leftMargin: T.s(8); anchors.rightMargin: T.s(8)
                        verticalAlignment: TextInput.AlignVCenter
                        font.family: T.mono; font.pointSize: T.f(12); color: T.text2
                        activeFocusOnTab: true
                        Mono { visible: !timeField.text.length; text: "9am, 17:30"; px: 12; color: T.meta; anchors.verticalCenter: parent.verticalCenter }
                        onAccepted: App.pickerTime(text)
                        onActiveFocusChanged: if (!activeFocus) App.pickerTime(text)
                    }
                }
            }

            Row {
                visible: picker.p.hasRepeat === true
                width: parent.width
                spacing: T.s(10)
                Mono { text: "repeat"; px: 12; color: T.hintText; width: T.s(52); anchors.verticalCenter: parent.verticalCenter }
                Rectangle {
                    width: parent.width - T.s(62) - T.s(24)
                    height: T.s(30)
                    color: T.sidebar; radius: T.s(6)
                    border.width: T.line; border.color: repeatField.activeFocus ? T.accent : repeatField.text.length ? T.surface1 : T.surface0
                    TextInput {
                        id: repeatField
                        anchors.fill: parent; anchors.leftMargin: T.s(8); anchors.rightMargin: T.s(8)
                        verticalAlignment: TextInput.AlignVCenter
                        font.family: T.mono; font.pointSize: T.f(12); color: T.text
                        activeFocusOnTab: true
                        Mono { visible: !repeatField.text.length; text: "every other monday"; px: 12; color: T.meta; anchors.verticalCenter: parent.verticalCenter }
                        onAccepted: App.pickerRepeat(text)
                        onActiveFocusChanged: if (!activeFocus) App.pickerRepeat(text)
                    }
                }
                Icon { name: "edit-clear"; px: 14; anchors.verticalCenter: parent.verticalCenter; opacity: repeatField.text.length ? 1 : 0.4
                       MouseArea { anchors.fill: parent; anchors.margins: -T.s(6); cursorShape: Qt.PointingHandCursor; onClicked: { repeatField.text = ""; App.pickerClearRepeat() } } }
            }

            Mono {
                width: parent.width
                wrapMode: Text.Wrap
                lineHeight: 1.6
                px: 11
                color: picker.p.error ? T.negative : T.hintText
                text: picker.p.error ? picker.p.error : picker.p.hasRepeat ? "every! counts from completion. Emptying the box stops the repeat and keeps the date." : (picker.p.summary || "")
            }
        }
    }
}
