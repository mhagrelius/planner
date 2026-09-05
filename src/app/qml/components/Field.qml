import QtQuick
import QtQuick.Controls
import QtQuick.Window
import Planner

// Text input box: window fill, 1px border, accent focus ring.
TextField {
    id: field
    property bool mono: false
    property real px: 12
    property real padX: 9
    property real padY: 6
    property real radiusPx: 6
    property bool invalid: false
    font.family: mono ? T.mono : T.sans
    font.pointSize: T.f(px)
    color: T.text
    placeholderTextColor: T.faint
    selectionColor: T.activeFill
    selectedTextColor: T.activeText
    leftPadding: T.s(padX)
    rightPadding: T.s(padX)
    topPadding: T.s(padY)
    bottomPadding: T.s(padY)
    implicitHeight: contentHeight + topPadding + bottomPadding + 2 * T.line
    // Escape hands keyboard focus back to the window so 1–8 and / work again.
    Keys.onEscapePressed: function(event) { var w = Window.window; if (w) w.contentItem.forceActiveFocus(); event.accepted = true }
    background: Rectangle {
        color: T.window
        radius: T.s(field.radiusPx)
        border.width: T.line
        border.color: field.invalid ? T.errorBg : field.activeFocus ? T.accent : T.border
    }
}
