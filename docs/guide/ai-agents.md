# AI agents

Claude Code, Codex, Cursor, Gemini CLI, VS Code and other AI assistants on your
PC can use NectarLink: read your notifications and texts, send a file to your
phone, reply to a message, control music. Only with your permission, and the
sensitive things always ask you first.

It's off until you turn it on.

## Turn it on

1. On the PC, open **Settings › AI agents** and turn on **Let AI agents use NectarLink**.
2. Under **Add an agent**, pick the app (Claude Code, Codex, Cursor...) and how much it may do:
   - **Read only**: reads devices, notifications, messages, calls, contacts, files and media. Changes nothing.
   - **Everyday**: also sends messages, files and notifications, and controls media.
3. Click **Add**. NectarLink shows what to paste into that app, with a **Copy** button.
   For Claude Code and Gemini CLI it's one command to run in a terminal.

The clipboard and one-time codes (the 6-digit kind) stay private in both
presets.

## What always asks you first

Sending a text, replying in a chat, placing or answering a call, deleting
anything and opening a link on your phone always show a window in NectarLink
with exactly what will be sent and to whom. **Deny** is the default. If you say
no, the agent is told to stop. If an agent gets three no's in ten minutes, it's
paused until you resume it.

Sending files to your own phone and locking the PC ask the first time; you can
choose **Always allow for this agent**.

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
- Agents on other computers or in the cloud (such as Grok) can't connect. Only
  apps on this PC can.
