// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// Which items of a list are selected, by ID. Ctrl (Cmd on a Mac) + click
// toggles one, Shift + click selects from the last one clicked to this one,
// and once something is selected a plain click toggles too, until the
// selection is cancelled. A list shows a checkbox on each item while `active`.
QtObject {
    id: selection

    // The selected IDs, as { id: true }: replaced at each change, so bindings follow.
    property var picked: ({})
    readonly property int count: Object.keys(picked).length
    // Selecting: from the Select button, or from the first Ctrl or Shift + click.
    property bool active: false
    // The last item clicked, where a Shift + click starts from.
    property string anchorId: ""
    // Started by the Select button: stays on with nothing selected.
    property bool explicit: false

    function has(id) {
        return picked[id] === true
    }

    // The selected IDs, in no order.
    function ids() {
        return Object.keys(picked)
    }

    // The selected IDs, in the order of `order`.
    function inOrder(order) {
        return order.filter(id => picked[id] === true)
    }

    // The Select button: selecting with nothing selected yet.
    function start() {
        explicit = true
        active = true
    }

    // Cancels: nothing is selected, and it's no longer selecting.
    function clear() {
        picked = ({})
        anchorId = ""
        explicit = false
        active = false
    }

    function toggle(id) {
        const next = Object.assign({}, picked)
        if (next[id] === true)
            delete next[id]
        else
            next[id] = true
        picked = next
        anchorId = id
        active = explicit || Object.keys(next).length > 0
    }

    // Selects `id` and everything between it and the last one clicked, in the
    // order of `order`; with none clicked yet it's just `id`.
    function range(id, order) {
        const from = order.indexOf(anchorId)
        const to = order.indexOf(id)
        if (from < 0 || to < 0) {
            toggle(id)
            return
        }
        const next = Object.assign({}, picked)
        for (let i = Math.min(from, to); i <= Math.max(from, to); i++)
            next[order[i]] = true
        picked = next
        active = true
    }

    function selectAll(order) {
        const next = {}
        for (let i = 0; i < order.length; i++)
            next[order[i]] = true
        picked = next
        if (order.length > 0)
            active = true
    }

    // A click on the item `id` of the list `order`. True when the selection
    // took it; false when it's a plain click on something not being selected,
    // which opens it.
    function click(id, order, ctrl, shift) {
        if (shift && anchorId.length > 0) {
            range(id, order)
            return true
        }
        if (ctrl || shift || active) {
            toggle(id)
            return true
        }
        return false
    }

    // Lets go of what is no longer in the list `order` (and of the mode when
    // nothing is left of a selection that wasn't started with the button).
    function prune(order) {
        const keep = {}
        let changed = false
        for (const id of Object.keys(picked)) {
            if (order.indexOf(id) >= 0)
                keep[id] = true
            else
                changed = true
        }
        if (!changed)
            return
        picked = keep
        if (order.indexOf(anchorId) < 0)
            anchorId = ""
        active = explicit || Object.keys(keep).length > 0
    }
}
