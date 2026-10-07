# Contacts service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A PC browses and searches a phone's contacts to call or text them from the
PC. The phone keeps the contacts; a PC asks for them on demand and keeps
them only in memory.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `contacts.read` | phone | Lists and searches contacts (needs the contacts permission) |
| `contacts.show` | PC | Shows contacts, and wants `contacts.changed` |

Both devices must allow it: the `contacts` device toggle, on by default,
on each side.

## 2. Messages

On the control stream. Each request is answered with the matching answer
or `error`.

```
t = "contacts.list"     id = n   b = { ? query: text, ? offset: uint, limit: uint }   // limit 1–200
t = "contacts.list"     re = n   b = { contacts: [ contact ] }

t = "contacts.changed"           b = {}                                               // phone → PC

contact = {
  id:        text,
  name:      text,
  numbers:   [ { number: text, ? label: text } ],   // 1–16 phone numbers
  ? starred: bool,                                  // Favorite contact (default false)
  ? photo:   bytes,                                 // Small JPEG, at most 16 KiB
}
```

### 2.1 Listing and searching

`contacts.list` returns contacts that have at least one phone number,
ordered with favorites (`starred: true`) first and then alphabetically by
`name`.

- `query` (optional, at most 256 bytes): when non-empty, filters to
  contacts whose `name` or any `number` matches `query` (case-insensitive;
  digits are also matched ignoring formatting).
- `offset` (optional, default `0`): skips the first `offset` matching
  contacts for paging.
- `limit` (`1..=200`): the most contacts to return in this page.

A phone answers with fewer items than `limit` when the answer wouldn't fit
in a frame (v0 §3), dropping from the end of the page. The PC advances
`offset` by the number of contacts received to fetch the next page.

Errors: `DENIED` when contacts are off for the PC, `UNSUPPORTED` when the
phone doesn't offer `contacts.read`.

### 2.2 Changes

The phone sends `contacts.changed` to connected PCs that offer
`contacts.show` when its contacts store changes (debounced). PCs then ask
again for what they show.

## 3. Privacy

Names, numbers and photos are never logged (v0 §11).
