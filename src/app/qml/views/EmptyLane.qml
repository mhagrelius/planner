import QtQuick
import Planner
import "../components"

// An empty board column still says it takes drops.
Item {
    height: T.s(60)
    Mono { anchors.centerIn: parent; text: "ctrl+← → moves a task here"; px: 11; color: T.hintText }
}
