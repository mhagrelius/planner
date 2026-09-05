import QtQuick
import Planner
import "../components"

// A project down the page, or a flat query view: unsectioned tasks first with
// no header, then each section as a lane. One recycling list would not do
// here because every lane is its own drop target on the board.
VScroll {
    id: lanes
    Column {
        width: parent.width
        Repeater {
            model: App.lanes
            Column {
                required property var modelData
                required property int index
                width: parent.width
                LaneHeader { visible: modelData.name.length > 0; title: modelData.name; count: modelData.count }
                Repeater {
                    model: modelData.tasks
                    TaskRow { showHints: App.openTask.length === 0 }
                }
            }
        }
        Item { width: 1; height: T.s(12) }
    }
    // Keep the cursor row in view as it moves.
    Connections {
        target: App
        function onChanged() {
            var y = T.s(54) * 0 + App.cursor * T.s(45)
            if (y < lanes.contentY) lanes.contentY = Math.max(0, y - T.s(45))
            else if (y + T.s(45) > lanes.contentY + lanes.height) lanes.contentY = Math.min(Math.max(0, lanes.contentHeight - lanes.height), y + T.s(90) - lanes.height)
        }
    }
}
