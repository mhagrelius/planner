import QtQuick
import Planner
import "../components"

// Command palette, quick add, quick find, a text input, or a confirmation:
// one shell, different sigil. The Today surface behind is dimmed under a scrim.
Item {
    id: prompt
    anchors.fill: parent
    visible: App.prompt.length > 0
    readonly property string kind: App.prompt
    readonly property color sigilColor: kind === "add" ? T.positive : kind === "find" ? T.info : T.accent
    readonly property string sigil: kind === "add" ? "+" : kind === "find" ? "/" : "›"

    function focusField() { field.forceActiveFocus(); field.cursorPosition = field.text.length }
    onVisibleChanged: if (visible) { field.text = App.promptQuery; focusField() }
    Component.onCompleted: if (visible) { field.text = App.promptQuery; focusField() }
    Connections {
        target: App
        function onChanged() { if (prompt.visible && field.text !== App.promptQuery) { field.text = App.promptQuery; field.cursorPosition = field.text.length } }
    }

    Rectangle { anchors.fill: parent; color: Qt.rgba(T.window.r * 0.55, T.window.g * 0.55, T.window.b * 0.55, 0.72); MouseArea { anchors.fill: parent; onClicked: App.closePrompt() } }

    Rectangle {
        id: box
        x: Math.round((parent.width - width) / 2)
        y: T.s(64)
        width: Math.min(T.s(kind === "palette" ? 660 : 610), parent.width - T.s(40))
        height: column.implicitHeight
        color: T.window
        radius: T.s(12)
        border.width: T.line
        border.color: T.surface1
        clip: true
        Column {
            id: column
            width: parent.width
            // The prompt row: sigil, the query, and the count at the right.
            Item {
                width: parent.width
                height: ((prompt.kind === "confirm" || prompt.kind === "status") ? 0 : T.s(48)) + (App.promptTitle.length && prompt.kind !== "status" ? T.s(28) : 0)
                Mono { visible: App.promptTitle.length > 0 && prompt.kind !== "status"; x: T.s(16); y: T.s(12); text: App.promptTitle; px: 11; color: T.hintText; width: parent.width - T.s(32); wrapMode: Text.Wrap }
                Row {
                    visible: prompt.kind !== "confirm" && prompt.kind !== "status"
                    anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
                    anchors.leftMargin: T.s(16); anchors.rightMargin: T.s(16); anchors.bottomMargin: T.s(14)
                    spacing: T.s(10)
                    Mono { text: prompt.sigil; px: 16; color: prompt.sigilColor; anchors.verticalCenter: parent.verticalCenter }
                    Item {
                        width: parent.width - T.s(26) - count.width - T.s(10)
                        height: field.implicitHeight
                        anchors.verticalCenter: parent.verticalCenter
                        TextInput {
                            id: field
                            width: parent.width
                            font.family: T.mono
                            font.pointSize: T.f(15)
                            color: T.text
                            selectionColor: T.activeFill
                            selectedTextColor: T.activeText
                            cursorDelegate: Rectangle { width: T.s(2); color: prompt.sigilColor; visible: field.activeFocus }
                            onTextChanged: App.setPromptQuery(text)
                            onAccepted: App.runPrompt()
                            Keys.onPressed: function(event) {
                                if (event.key === Qt.Key_K && (event.modifiers & Qt.ControlModifier)) { App.submitKeepAdding(); event.accepted = true }
                            }
                            Mono { visible: !field.text.length; text: App.promptPlaceholder; px: 15; color: T.hintText; elide: Text.ElideRight; width: parent.width }
                        }
                        // Recognised tokens in the accent, all the same hue: hue would encode kind
                        // for people who can tell hues apart, so kind goes on the chips instead.
                        Repeater {
                            model: prompt.kind === "add" ? (App.addPreview.spans || []) : []
                            Rectangle {
                                required property var modelData
                                readonly property rect a: { var tie = field.text + field.width; return field.positionToRectangle(modelData.start) }
                                readonly property rect b: { var tie = field.text + field.width; return field.positionToRectangle(modelData.end) }
                                x: a.x; y: a.y; width: b.x - a.x; height: a.height
                                color: "transparent"
                                clip: true
                                Text { x: -parent.x; text: field.text; font: field.font; color: prompt.sigilColor }
                            }
                        }
                    }
                    Mono { id: count; text: App.promptCount; px: 11; color: T.hintText; anchors.verticalCenter: parent.verticalCenter }
                }
            }
            Rectangle { width: parent.width; height: T.line; color: T.surface0; visible: prompt.kind !== "confirm" && prompt.kind !== "status" }

            // Sync status: what syncing has and has not done, as label and value rows.
            Column {
                visible: prompt.kind === "status"
                width: parent.width
                Item {
                    width: parent.width; height: T.s(40)
                    Mono { x: T.s(16); anchors.verticalCenter: parent.verticalCenter; text: "sync"; px: 10; color: T.hintText; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(10 * 0.16) }
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.surface0 }
                }
                Repeater {
                    model: App.promptRows
                    Item {
                        required property var modelData
                        width: parent.width
                        height: value.implicitHeight + T.s(14)
                        Mono { x: T.s(16); y: T.s(7); width: T.s(110); text: modelData.k; px: 12; color: T.hintText }
                        Mono { id: value; x: T.s(136); y: T.s(7); width: parent.width - T.s(152); text: modelData.v; px: 12; color: T.text2; wrapMode: Text.Wrap
                               font.family: modelData.k === "server" || modelData.k === "file" || modelData.k === "records here" ? T.mono : T.mono }
                    }
                }
                Mono { x: T.s(16); width: parent.width - T.s(32); topPadding: T.s(10); bottomPadding: T.s(14); text: App.promptTitle; px: 11; color: T.hintText; wrapMode: Text.Wrap; lineHeight: 1.5 }
            }

            // Quick add: chips in parser order, then the app's own hint.
            Column {
                visible: prompt.kind === "add"
                width: parent.width
                Flow {
                    x: T.s(16); width: parent.width - T.s(32); topPadding: T.s(14); bottomPadding: T.s(10); spacing: T.s(6)
                    Repeater {
                        model: App.addPreview.chips || []
                        LabelChip { required property var modelData; text: modelData.text; icon: modelData.icon; fg: modelData.role ? T.role(modelData.role) : T.text2; height: T.s(22) }
                    }
                }
                Mono { x: T.s(16); width: parent.width - T.s(32); text: App.addPreview.hint || ""; px: 11; color: T.hintText; wrapMode: Text.Wrap; lineHeight: 1.7; bottomPadding: T.s(14) }
            }

            // A confirmation: the question, and two keys.
            Column {
                visible: prompt.kind === "confirm"
                width: parent.width
                Sans { x: T.s(16); width: parent.width - T.s(32); topPadding: T.s(16); bottomPadding: T.s(14); text: App.promptTitle; px: 14; wrapMode: Text.Wrap; color: T.text }
            }

            // An input's live error, e.g. why a filter will not parse.
            Mono { visible: prompt.kind === "input" && App.promptError.length > 0; x: T.s(16); topPadding: T.s(8); bottomPadding: T.s(6); text: App.promptError; px: 11; color: T.negative }

            // Results, grouped under uppercase headings.
            Column {
                visible: prompt.kind === "palette" || prompt.kind === "find"
                width: parent.width
                topPadding: T.s(6); bottomPadding: T.s(8)
                Repeater {
                    model: App.promptResults
                    Loader {
                        required property var modelData
                        required property int index
                        width: parent.width
                        property var item: modelData
                        // Position among the selectable rows (headings do not count).
                        property int itemIndex: {
                            var n = 0
                            for (var i = 0; i < index; ++i) if (App.promptResults[i].kind !== "heading") ++n
                            return n
                        }
                        sourceComponent: modelData.kind === "heading" ? headingRow : resultRow
                    }
                }
                Mono { visible: App.promptResults.length === 0; x: T.s(16); topPadding: T.s(8); bottomPadding: T.s(6); text: prompt.kind === "find" && !App.promptQuery.length ? "type to search" : "nothing matches"; px: 12; color: T.hintText }
            }

            // Footer: keys, and one line of what this prompt is for.
            Rectangle {
                width: parent.width
                height: T.s(38)
                color: T.sidebar
                Rectangle { width: parent.width; height: T.line; color: T.surface0 }
                Row {
                    anchors.left: parent.left; anchors.leftMargin: T.s(16); anchors.verticalCenter: parent.verticalCenter
                    spacing: T.s(16)
                    Keycap { visible: prompt.kind === "palette" || prompt.kind === "find"; key: "↑↓"; text: "move" }
                    Keycap { visible: prompt.kind !== "status"; key: "enter"; text: prompt.kind === "add" ? "add" : prompt.kind === "find" ? "open" : prompt.kind === "palette" ? "run" : prompt.kind === "confirm" ? "delete" : "save" }
                    Keycap { visible: prompt.kind === "add"; key: "ctrl k"; text: "keep adding"; fill: App.keepAdding ? T.positive : T.surface0; keyColor: App.keepAdding ? T.onFill(T.positive) : T.text }
                    Keycap { key: "esc"; text: prompt.kind === "confirm" ? "keep it" : prompt.kind === "status" ? "close" : "dismiss" }
                }
                Mono {
                    anchors.right: parent.right; anchors.rightMargin: T.s(16); anchors.verticalCenter: parent.verticalCenter
                    px: 11; color: T.hintText
                    text: prompt.kind === "add" ? (App.addPreview.destination || "") : prompt.kind === "find" ? "tasks, projects and labels at once"
                        : prompt.kind === "palette" ? "every view is a query — filters live in the same list" : prompt.kind === "status" ? "planner sync status, from a shell" : ""
                }
            }
        }
    }

    Component {
        id: headingRow
        Item {
            width: parent ? parent.width : 0
            height: T.s(24)
            Mono { x: T.s(16); anchors.bottom: parent.bottom; anchors.bottomMargin: T.s(4); text: item.title; px: 10; color: T.hintText; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(10 * 0.16) }
        }
    }
    Component {
        id: resultRow
        Rectangle {
            id: r
            width: parent ? parent.width : 0
            height: T.s(38)
            readonly property bool active: itemIndex === App.promptIndex
            color: active ? T.surface0 : "transparent"
            opacity: item.completed ? 0.6 : 1
            MouseArea { anchors.fill: parent; hoverEnabled: true; onPositionChanged: App.promptTo(itemIndex); onClicked: { App.promptTo(itemIndex); App.runPrompt() } }
            Row {
                anchors.fill: parent; anchors.leftMargin: T.s(16); anchors.rightMargin: T.s(16)
                spacing: T.s(12)
                // A task shows its ring; an action or view its icon.
                Item {
                    width: T.s(15); height: T.s(15); anchors.verticalCenter: parent.verticalCenter
                    Rectangle { visible: item.kind === "task"; anchors.centerIn: parent; width: T.s(9); height: T.s(9); radius: T.s(3)
                                color: item.completed ? T.surface1 : "transparent"; border.width: item.completed ? 0 : Math.max(1, T.s(2))
                                border.color: item.priorityRole ? T.role(item.priorityRole) : T.surface1 }
                    Icon { visible: item.kind !== "task" && !!item.icon && !(item.color && item.color.length); anchors.centerIn: parent; name: item.icon || ""; tint: r.active ? T.text : T.hintText }
                    Rectangle { visible: !!(item.color && item.color.length); anchors.centerIn: parent; width: T.s(7); height: T.s(7); radius: T.s(2); color: item.color ? T.role(item.color) : T.hintText }
                }
                Sans { width: parent.width - T.s(15) - T.s(24) - trailing.width; text: item.title; px: 15; elide: Text.ElideRight; anchors.verticalCenter: parent.verticalCenter
                       color: item.completed ? T.meta : r.active ? T.text : T.text2; font.strikeout: item.completed || false }
                Row {
                    id: trailing
                    spacing: T.s(8)
                    anchors.verticalCenter: parent.verticalCenter
                    Mono { visible: !!item.context; text: item.context || ""; px: 12; color: item.contextRole ? T.role(item.contextRole) : T.meta; anchors.verticalCenter: parent.verticalCenter }
                    LabelChip { visible: !!item.chip; text: item.chip || ""; px: 11; fg: r.active ? T.text : T.meta; color: r.active ? T.surface1 : T.surface0; anchors.verticalCenter: parent.verticalCenter }
                    Keycap { visible: !!item.key; key: item.key || ""; keyColor: r.active ? T.text : T.meta; fill: r.active ? T.surface1 : T.surface0; anchors.verticalCenter: parent.verticalCenter }
                }
            }
        }
    }
}
