import QtQuick
import QtQuick.Controls
import Planner

// Vertical scroll region with the 9px scrollbar from the design.
Flickable {
    id: flick
    default property alias content: holder.data
    property real padding: 0
    contentWidth: width
    contentHeight: holder.childrenRect.height + 2 * padding
    boundsBehavior: Flickable.StopAtBounds
    clip: true
    flickableDirection: Flickable.VerticalFlick
    Item {
        id: holder
        x: flick.padding
        y: flick.padding
        width: flick.width - 2 * flick.padding
    }
    ScrollBar.vertical: ScrollBar {
        id: bar
        policy: flick.contentHeight > flick.height ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff
        width: T.s(9)
        hoverEnabled: true
        background: Item {}
        contentItem: Item {
            implicitWidth: T.s(9)
            Rectangle {
                anchors.fill: parent
                anchors.margins: T.s(2)
                radius: T.s(3)
                color: bar.hovered || bar.pressed ? T.faint : T.borderStrong
            }
        }
    }
}
