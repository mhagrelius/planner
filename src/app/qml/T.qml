pragma Singleton
import QtQuick
import Planner

// Design tokens. Every dimension in the interface is written at the 1360×900
// design size and passed through s() so the whole face follows the desktop
// text scale (`omarchy display text size`), the same way the Omarchy shell
// scales its spacing with its font base size. Colours come straight from the
// Palette derived from the active theme's colors.toml.
QtObject {
    id: root

    readonly property real scale: Palette.textScale
    readonly property real ppp: Palette.pointsPerPixel

    function s(px) { return Math.round(px * root.scale) }
    function r(px) { return px * root.scale }
    function f(px) { return px * root.scale * root.ppp }
    readonly property int line: Math.max(1, Math.round(root.scale))

    readonly property string sans: Palette.sansFamily
    readonly property string mono: Palette.monoFamily

    readonly property bool dark: Palette.dark
    readonly property color window: Palette.window
    readonly property color sidebar: Palette.sidebar
    readonly property color card: Palette.card
    readonly property color header: Palette.header
    readonly property color groupRow: Palette.groupRow
    readonly property color hover: Palette.hover
    readonly property color divider: Palette.divider
    readonly property color track: Palette.track
    readonly property color border: Palette.border
    readonly property color borderStrong: Palette.borderStrong
    readonly property color text: Palette.text
    readonly property color text2: Palette.text2
    readonly property color muted: Palette.muted
    readonly property color faint: Palette.faint
    readonly property color accent: Palette.accent
    readonly property color accentText: Palette.accentText
    readonly property color selection: Palette.selection
    readonly property color activeFill: Palette.activeFill
    readonly property color activeText: Palette.activeText
    readonly property color positive: Palette.positive
    readonly property color positiveDim: Palette.positiveDim
    readonly property color negative: Palette.negative
    readonly property color warning: Palette.warning
    readonly property color caution: Palette.caution
    readonly property color cautionAlt: Palette.cautionAlt
    readonly property color teal: Palette.teal
    readonly property color blueGray: Palette.blueGray
    readonly property color violet: Palette.violet
    readonly property color pink: Palette.pink
    readonly property color info: Palette.info
    readonly property color blue: Palette.blue
    readonly property color positiveBg: Palette.positiveBg
    readonly property color infoBg: Palette.infoBg
    readonly property color warningBg: Palette.warningBg
    readonly property color cautionBg: Palette.cautionBg
    readonly property color errorBg: Palette.errorBg
    readonly property color neutralBg: Palette.neutralBg
    readonly property color destructiveHover: Palette.destructiveHover
    readonly property color invalidBand: Palette.invalidBand
    readonly property color dragBg: Palette.dragBg
    readonly property color tealBorder: Palette.tealBorder
    readonly property color tooltipBg: Palette.tooltipBg

    // The design's text ramp between text2 and muted: overlay2 for secondary
    // mono meta, subtext0 for hints and uppercase labels. Surface-ramp colours
    // are for fills, borders and separators only — never for copy.
    function mix(a, b, t) { return Qt.rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1) }
    readonly property color meta: mix(text2, muted, 0.5)
    readonly property color hintText: mix(text2, muted, 0.25)
    // Fills: surface0 for keycaps, chips and hairlines; surface1 for the
    // selected rail row, popover borders and the unset checkbox ring.
    readonly property color surface0: selection
    readonly property color surface1: borderStrong
    // Text on a saturated fill (a keycap, a checked box).
    function onFill(c) { return (0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b) > 0.5 ? Qt.rgba(0.07, 0.07, 0.11, 1) : Qt.rgba(1, 1, 1, 0.94) }
    // A hex string for the icon provider.
    function hex(c) { return Qt.rgba(c.r, c.g, c.b, 1).toString() }

    // Resolve a palette role name from model data. Reading `text` first ties
    // the binding to the palette's change signal so rows re-tint live.
    function role(name) {
        var tie = Palette.text
        if (name === "meta") return root.meta
        if (name === "hintText") return root.hintText
        return Palette.role(name)
    }

    function brighten(c, amount) {
        return Qt.rgba(Math.min(1, c.r * amount), Math.min(1, c.g * amount), Math.min(1, c.b * amount), c.a)
    }

    // Single cursor-following tooltip, owned by Main.qml.
    property var tip: null
    property real tipX: 0
    property real tipY: 0
    function showTip(payload, x, y) {
        if (!payload) return
        if (tip && tip.title === payload.title && Math.abs(tipX - x) < 4 && Math.abs(tipY - y) < 4) return
        tipX = x
        tipY = y
        tip = payload
    }
    function hideTip() { if (tip) tip = null }
}
