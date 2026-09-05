import QtQuick
import Planner

Text {
    property real px: 12.5
    property int weight: Font.Normal
    font.family: T.sans
    font.pointSize: T.f(px)
    font.weight: weight
    color: T.text
    verticalAlignment: Text.AlignVCenter
}
