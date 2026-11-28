// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// A model that follows a list of rows (objects with a unique string `id`)
// by changing only what differs: a row that is new is inserted, one that is
// gone removed, one that moved is moved, one that changed is updated where
// it is. A view of it keeps its scroll position and the delegates it has
// (their images, their focus), which a plain array as a model, replaced
// whole at every change, does not: a list view puts itself back at the top
// and builds every delegate again.
// A delegate reads its row from the `json` role: `JSON.parse(json)`.
ListModel {
    id: model

    function sync(rows) {
        const wanted = {}
        const list = []
        for (let i = 0; i < rows.length; i++) {
            if (wanted[rows[i].id] === true)
                continue
            wanted[rows[i].id] = true
            list.push(rows[i])
        }
        for (let i = count - 1; i >= 0; i--) {
            if (wanted[get(i).rid] !== true)
                remove(i)
        }
        for (let i = 0; i < list.length; i++) {
            const json = JSON.stringify(list[i])
            if (i < count && get(i).rid === list[i].id) {
                if (get(i).json !== json)
                    setProperty(i, "json", json)
                continue
            }
            let from = -1
            for (let j = i + 1; j < count; j++) {
                if (get(j).rid === list[i].id) {
                    from = j
                    break
                }
            }
            if (from >= 0) {
                move(from, i, 1)
                if (get(i).json !== json)
                    setProperty(i, "json", json)
            } else {
                insert(i, { rid: list[i].id, json: json })
            }
        }
    }

    // The ID of the row at `index`, or "".
    function idAt(index) {
        return index >= 0 && index < count ? get(index).rid : ""
    }

    // The index of the row with this ID, or -1.
    function indexOfId(id) {
        for (let i = 0; i < count; i++) {
            if (get(i).rid === id)
                return i
        }
        return -1
    }
}
