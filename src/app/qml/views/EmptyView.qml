import QtQuick
import Planner
import "../components"

// An empty view still says what it is for, in its own words.
Item {
    Column {
        anchors.centerIn: parent
        spacing: T.s(14)
        Icon { anchors.horizontalCenter: parent.horizontalCenter; name: App.empty.icon || "object-select"; px: 44; tint: T.hintText; opacity: 0.5 }
        Sans { anchors.horizontalCenter: parent.horizontalCenter; text: App.empty.title || ""; px: 18; weight: Font.DemiBold }
        Mono { anchors.horizontalCenter: parent.horizontalCenter; text: App.empty.description || ""; px: 13; color: T.hintText; horizontalAlignment: Text.AlignHCenter }
        Item { width: 1; height: T.s(6) }
        Keycap { anchors.horizontalCenter: parent.horizontalCenter; key: App.empty.key || ""; text: App.empty.hint || "" }
    }
}
