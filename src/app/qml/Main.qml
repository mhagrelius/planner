import QtQuick
import QtQuick.Window
import Planner
import "components"
import "views"

// Hyprland draws the border and rounded corners; the app owns everything
// inside. Every size goes through T.s()/T.f() so the face follows the desktop
// text scale, every colour through T.<role>.
//
// There are no modes: conventional shortcuts only, handled in one place so a
// key means the same thing everywhere it is not being typed into a field.
Window {
    id: win
    width: Math.round(1240 * T.scale)
    height: Math.round(640 * T.scale)
    visible: true
    title: "Planner — " + App.viewTitle
    color: T.window

    readonly property bool typing: activeFocusItem && (activeFocusItem instanceof TextInput || activeFocusItem instanceof TextEdit)

    Item {
        id: root
        anchors.fill: parent
        focus: true

        Keys.onPressed: function(event) {
            const ctrl = event.modifiers & Qt.ControlModifier
            const shift = event.modifiers & Qt.ShiftModifier
            const promptOpen = App.prompt.length > 0
            const pickerOpen = App.picker.open === true

            // Escape closes the innermost thing, and the window when nothing is open.
            if (event.key === Qt.Key_Escape) {
                if (win.typing) { root.forceActiveFocus() }
                if (!App.escape() && !win.typing) win.close()
                event.accepted = true
                return
            }
            if (ctrl && event.key === Qt.Key_Q) { win.close(); event.accepted = true; return }

            if (pickerOpen) {
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) { App.closePicker(); root.forceActiveFocus(); event.accepted = true }
                return
            }
            if (promptOpen) {
                if (event.key === Qt.Key_Up) { App.movePrompt(-1); event.accepted = true }
                else if (event.key === Qt.Key_Down) { App.movePrompt(1); event.accepted = true }
                else if (ctrl && event.key === Qt.Key_K) { if (App.prompt === "add") App.submitKeepAdding(); else App.closePrompt(); event.accepted = true }
                else if (ctrl && event.key === Qt.Key_N && App.prompt !== "add") { App.openPrompt("add"); event.accepted = true }
                else if (ctrl && event.key === Qt.Key_F && App.prompt !== "find") { App.openPrompt("find"); event.accepted = true }
                return
            }

            // Global shortcuts, whether or not a detail field has focus.
            if (ctrl && shift && event.key === Qt.Key_P) { App.pinCursor(); event.accepted = true; return }
            if (ctrl && shift && event.key === Qt.Key_B) { App.toggleStyle(); event.accepted = true; return }
            if (ctrl && shift && event.key === Qt.Key_N) { App.newSection(); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_N) { App.openPrompt("add"); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_F) { App.openPrompt("find"); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_K) { App.openPrompt("palette"); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_B) { App.toggleRail(); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_D) { App.openDatePicker(); event.accepted = true; return }
            if (ctrl && shift && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter)) { if (App.openTask.length) detail.focusNote(); event.accepted = true; return }
            if (ctrl && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter)) { if (App.openTask.length) detail.focusSubtask(); event.accepted = true; return }
            if (win.typing) return
            if (ctrl && event.key === Qt.Key_A) { App.selectAll(); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_Z) { App.undo(); event.accepted = true; return }
            if (ctrl && event.key >= Qt.Key_1 && event.key <= Qt.Key_4) { App.setPriorityKey(event.key - Qt.Key_0); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_Up) { App.moveTaskVertical(-1); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_Down) { App.moveTaskVertical(1); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_Left) { App.moveTaskColumn(-1); event.accepted = true; return }
            if (ctrl && event.key === Qt.Key_Right) { App.moveTaskColumn(1); event.accepted = true; return }
            if (event.modifiers & (Qt.ControlModifier | Qt.MetaModifier | Qt.AltModifier)) return

            if (event.key === Qt.Key_Up) { App.moveCursor(-1); event.accepted = true }
            else if (event.key === Qt.Key_Down) { App.moveCursor(1); event.accepted = true }
            else if (event.key === Qt.Key_Left) { App.moveColumn(-1); event.accepted = true }
            else if (event.key === Qt.Key_Right) { App.moveColumn(1); event.accepted = true }
            else if (event.key === Qt.Key_Space) { App.space(); event.accepted = true }
            else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) { App.enter(); event.accepted = true }
            else if (event.key === Qt.Key_Delete) { App.deleteKey(); event.accepted = true }
            else if (event.key === Qt.Key_Tab && App.openTask.length) { detail.forceActiveFocus(); event.accepted = true }
            else if (event.key >= Qt.Key_1 && event.key <= Qt.Key_5) { App.goToKey(event.key - Qt.Key_0); event.accepted = true }
        }

        Row {
            anchors.fill: parent
            Rail { id: rail; visible: App.railVisible; height: parent.height }
            Item {
                width: parent.width - (rail.visible ? rail.width : 0) - (detail.visible ? detail.width : 0)
                height: parent.height
                Header { id: header; width: parent.width }
                Item {
                    id: content
                    anchors.top: header.bottom
                    anchors.bottom: statusLine.top
                    width: parent.width
                    Lanes { anchors.fill: parent; visible: !App.board && App.rowCount > 0 }
                    Board { anchors.fill: parent; visible: App.board }
                    EmptyView { anchors.fill: parent; visible: !App.board && App.rowCount === 0 }
                }
                StatusLine { id: statusLine; anchors.bottom: parent.bottom; width: parent.width }
            }
            Detail { id: detail; visible: App.openTask.length > 0; height: parent.height }
        }

        Prompt { }
        DatePicker { }
    }
}
