# AI agents

Claude Code, Claude Desktop, Codex, Cursor, Gemini CLI, VS Code and other AI
assistants on your PC can use NectarLink: read your notifications and texts,
send a file to your phone, reply to a message, find a photo, control music.
Only with your permission, and the sensitive things always ask you first.

It's off until you turn it on.

## Turn it on

1. On the PC, open **Settings › AI agents** and turn on **Let AI agents use NectarLink**.
2. Under **Add an agent**, pick the app (Claude Code, Codex, Cursor...) and how much it may do:
   - **Read only**: reads devices, notifications, messages, calls, contacts, photos, files and media. Changes nothing.
   - **Everyday**: also sends messages, files and notifications, controls media, copies photos to the PC,
     opens phone apps on the PC and changes the phone's quick settings (Do Not Disturb, volume...).
3. Click **Add**. For most apps there's then an **Add to** button that puts NectarLink in the app's
   settings for you; restart the app afterwards. NectarLink keeps a copy of the old settings file next
   to it. If the button isn't there, or the app's settings file has comments in it, NectarLink shows
   what to paste instead, with a **Copy** button.

To change what one agent may do, click **Permissions** next to it and pick **Off**, **Read** or
**Read and act** for each area. Your phone's files, Deck tiles and turning PCs off are never in a
preset; give them here if you want an agent to use them.

The clipboard and one-time codes (the 6-digit kind) stay private in both
presets.

## What always asks you first

Sending a text, replying in a chat, placing or answering a call, deleting
anything (photos and phone files included), uninstalling a phone app, locking,
sleeping or shutting down a PC, accepting files from a nearby LocalSend device
and opening a link on your phone always show a window in NectarLink with exactly
what will happen and to whom. **Deny** is the default. If you say
no, the agent is told to stop. If an agent gets three no's in ten minutes, it's
paused until you resume it.

Sending files to your own phone, copying photos or phone files to the PC,
putting a file on the phone, opening a phone app on the PC, changing a quick
setting, pressing a Deck tile, waking a PC and locking this PC ask the first
time; you can choose **Always allow for this agent**.

## What agents can never do

Pair or unpair devices, change permissions, read keys or passwords, update or
install anything, wipe data, or unlock your PC. There's no way to ask for these.

Each device's own permissions still apply: if Messages is off for a phone in
NectarLink, no agent can read its texts either.

## Read-only, pause and remove

- **Read-only for every agent** stops all of them from changing anything at once.
- **Pause** stops one agent until you resume it. **Remove** deletes it and its key.
- **Activity** lists what each agent did or tried, and what you answered. It
  records who and what, never the text of your messages.

## Good to know

- Messages and notifications can contain text written to trick an AI ("ignore
  your instructions and..."). NectarLink marks that text as data for the agent,
  and the confirmation window is what stops anything being sent. Read it before
  you click Allow.
- Anything an agent reads goes to that agent's AI provider, like anything else
  you show it.
- An agent's key lives in NectarLink's own folder, not in the app's settings
  file, so it doesn't end up in a project you share.
- While it's running, an agent can follow what happens: a new notification or
  text, a call, a finished transfer, a device connecting. It only sees what its
  permissions allow.
- Agents on other computers or in the cloud (such as Grok) can't connect. Only
  apps on this PC can.
