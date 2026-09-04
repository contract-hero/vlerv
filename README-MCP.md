# vlerv-mcp — send artifacts from Claude Code to your devices

`vlerv-mcp` is a stdio [MCP](https://modelcontextprotocol.io) server that gives a
coding agent one job: take a file the agent just produced and put it on a device
a person actually reads it on — an iPhone running Vlervcode, or another Mac
running Vlervtifacts.

The transport is [iroh](https://iroh.computer): a direct, hole-punched, QUIC
connection between the two machines, end-to-end encrypted, with an encrypted
public relay as fallback. **Nothing is uploaded to a server.** No public URL
exists. The bytes go from your machine to your device and nowhere else.

```
Claude Code ──stdio/JSON-RPC──▶ vlerv-mcp ──direct QUIC──▶ your iPhone
                                (a peer with                (Vlervcode)
                                 its own identity)
```

## Contents

- [What it is](#what-it-is)
- [Build](#build)
- [Register with Claude Code](#register-with-claude-code)
- [The tools](#the-tools)
- [Pairing an iOS device, step by step](#pairing-an-ios-device-step-by-step)
- [Where it keeps its files](#where-it-keeps-its-files)
- [Security model](#security-model)
- [Troubleshooting](#troubleshooting)

## What it is

`vlerv-mcp` is **its own peer**, not a remote control for the desktop app. It
holds its own ed25519 identity and its own `peers.json`, and it announces itself
as `Claude Code @ <hostname>`. Pairing a phone with this server is a separate
act from pairing that phone with Vlervtifacts on the same Mac — and revoking one
leaves the other alone.

It boots lazily. Registering it costs nothing: the process binds no socket and
makes no network connection until the first tool call that needs one.

Everything networked comes from the `vlerv-remote` crate unchanged — the same
request gate, the same `RootSet` path gate, the same peer-locked grants, the
same BLAKE3 verification the desktop app uses.

## Build

```sh
cargo build --release -p vlerv-mcp
```

The binary lands at `target/release/vlerv-mcp`. Note the absolute path; the next
step needs it.

```sh
echo "$(pwd)/target/release/vlerv-mcp"
```

## Register with Claude Code

```sh
claude mcp add vlerv -- /absolute/path/to/vlerv/target/release/vlerv-mcp
```

Add `--scope user` to make it available in every project instead of only the
current one:

```sh
claude mcp add vlerv --scope user -- /absolute/path/to/vlerv/target/release/vlerv-mcp
```

Check it:

```sh
claude mcp list
```

Inside Claude Code, `/mcp` shows the server and its eight tools.

### Optional environment

| Variable | Effect |
|---|---|
| `VLERV_MCP_ROOTS` | Colon-separated directories this server may send files from. **This is the send boundary**, not a hint: a path outside every root is refused. Defaults to the working directory Claude Code launched the server in. |
| `VLERV_MCP_STATE_DIR` | Where identity, peers and blobs live. Defaults to `~/Library/Application Support/Vlerv/mcp`. |
| `VLERV_STATE_DIR` | The Vlervtifacts state directory; the server uses its `mcp/` subdirectory. |

Pass them with `claude mcp add --env`:

```sh
claude mcp add vlerv --env VLERV_MCP_ROOTS=/Users/me/work -- /path/to/vlerv-mcp
```

## The tools

Talk to it in plain language — "send that report to my phone", "give me a link
for this chart", "pair my iPad". The tools below are what the model picks from.

### `beam_artifact { path, ttl_hours? }`

Publishes one local file as a `vlerv://receive?…` link. Give the link to a
person over any channel you already use; they open it, confirm, and the file
streams straight from your machine, verified by its BLAKE3 hash.

- `path` — absolute, or relative to the server's working directory. It must
  resolve inside `VLERV_MCP_ROOTS` (see [Security model](#security-model)).
- `ttl_hours` — 1 to 720, default 24.

Returns `link`, `ticket`, `name`, `size`, `expires_at`, `hash`.

**The link works only while the server process runs.** Claude Code starts the
server on demand and stops it when the session ends, so send the file promptly.

### `stop_beam { hash? }`

Revokes a link before it expires. The blobs request gate reads the offer
registry on every request, so the next fetch is refused at once — even from
somebody who still holds the link string.

- `hash` — the content hash `beam_artifact` and `server_status` report, or a
  prefix of 8 characters or more. Omit it to revoke every live link.

Use it the moment a link went to the wrong place. A link is a capability, and
the TTL is a backstop, not a control.

### `list_devices { probe? }`

Every device paired with this server: name, node id, the scope it was granted
here, when it was last seen, and presence.

- `probe` — when true, dials each device once for live presence. Without it
  presence is `"unknown"` unless a session is already open.

A probed device reads one of four words, and they are kept apart because you
act on each differently:

- `"online"` — a session is open, or a probe just opened one.
- `"offline"` — it did not answer at all.
- `"refused"` — it answered but would not open a session. It is awake and on
  the network, so this is not a connectivity problem, and **nothing about the
  pairing follows from it**: a version mismatch between the two builds, a
  device already at its session cap, and a stream that broke mid-handshake all
  read this way.
- `"unpaired"` — it answered and said this server is not on its peer list.
  This is the one that means the pairing is gone, and the only one that
  justifies `forget_device`.

The difference is decided by the QUIC close code on the wire, never by the
peer's own reason text.

### `send_to_device { path, device }`

Sends the file straight to one paired device. No link, no tap: the file lands
there and opens on that screen.

- `device` — a device name, part of one, or a prefix of the node id (4
  characters or more). An ambiguous or unknown name is an error that lists the
  valid ones.

Returns a `status`. **`delivered`** carries the name the **receiving** device
landed the file under (it renames on collision), the size it measured, and the
shared content hash.

**`queued`** means the device did not answer and the send was accepted anyway.
The file is copied into this server's state directory as it stands at that
moment, so later edits to it do not change what arrives, and it goes out on its
own as soon as the device is reachable. The device dialling in is the fast
path: that connection delivers the file at once. Failing that, this server
retries on its own — it looks at the queue every 60 seconds, and each device
waits out its own backoff first: 60 seconds after the first refused dial, then
2 minutes, then 5, then 10 minutes for every attempt after that.

**A queued send outlives the tool call, but not the process.** The bytes move
only while a `vlerv-mcp` is running against that state directory, so a send
accepted by a session that then closes goes out at the first network-touching
tool call of a later one. A record nobody could deliver is kept for 7 days and
then dropped, and its private copy goes with it. The queue holds at most 64
records or 1 GiB; a full queue refuses the send rather than dropping an older
one, and `server_status` lists what is waiting.

A device whose last completed handshake reported a scope narrower than
`control` is refused outright rather than queued: it would refuse the bytes on
arrival, and the copy would sit here for the week. For a device this server has
never completed a handshake with, the send is still queued, and the answer says
in as many words that the grant is unverified.

This needs the receiving device to have granted this server the **`control`**
scope. See [Security model](#security-model).

### `pair_device {}`

Mints a one-time `vlerv://pair?ticket=…` link and opens pairing for ten
minutes. Returns the link plus the instructions to read to the user.

### `pair_status {}`

Pairings waiting for confirmation, each with its **six fingerprint words**.
These words must be shown to the human and compared with the other device's
screen. Boots nothing.

### `confirm_pairing { accept, node_id?, scope? }`

Finishes or rejects a pending pairing.

- `accept` — `false` discards it and writes nothing to disk.
- `node_id` — only needed when more than one pairing is waiting.
- `scope` — what the **new device** may do on this server: `view-open`
  (default for a device that is new here), `browse` or `control`.
  Re-pairing a device that is already trusted and naming a scope **replaces**
  its grant, including narrowing it. Omitting `scope` names no grant, so an
  already-trusted device keeps the one it has.

### `forget_device { device }`

Unpairs one device and deletes everything this server was keeping for it.

- `device` — the device name, or a prefix of its node id, exactly as
  `send_to_device` matches it.

It removes the pairing, so that device can no longer reach this server, and it
**deletes the private copies** of any files still queued for it — those sends
never arrive. Pairing again restores the pairing; the deleted copies do not
come back, and each send has to be made again. The result says which of them
happened.

The order matters and is fixed: the pairing goes first, so a cleanup that
cannot finish leaves a device unpaired with its records still queued, never the
other way round. Whatever is left is finished by the next drain pass.

Use it when a device is no longer the user's, or when `list_devices` reports it
as `"unpaired"` — that device already removed this server, and until now the
only ways to agree again were editing `peers.json` by hand or waiting out the
seven-day record expiry.

Do **not** reach for it on a device reported `"refused"`. That word says only
that the device would not open a session, which a version mismatch or a session
cap also causes, and unpairing would destroy a pairing that works. It is also
not a way to cancel one queued send: there is no such tool, and this one takes
the pairing with it.

### `server_status {}`

This server's node id, its identity directory, whether it has booted the
network, its uptime, which beam links are still being served, which files other
devices pushed to it during this session, and which sends are still queued for
a device that has not answered. The pushed-file list holds the last 100
arrivals; `received_total` reports how many arrived in all. The structured
result carries every queued record, with the device it is for and the last
error it hit. It also carries `abandoned`: the sends this process accepted and then broke
its promise about — expired, dropped because the staged bytes went, or dropped
because the drain found the device unpaired — each with the file, the device
and the reason. A `forget_device` withdrawal is **not** on this list: that tool
already reported what it deleted to whoever asked for it. The list is in memory
only, because the record itself is gone by then, so it reports what **this**
process ended. The summary prints it only when there is something on it. The summary text prints that list too — except when
`queue_blocked_reason` is set, where it names the count and the reason and
stops there, because a record-by-record list under a queue that can move
nothing reads as progress.

## Pairing an iOS device, step by step

1. **Ask for it.** In Claude Code: *"pair my iPhone with the Vlerv server"*.
   The model calls `pair_device` and shows you a `vlerv://pair?ticket=…` link.

2. **Get the link onto the phone.** AirDrop it, iMessage it to yourself, put it
   in a note. The link is a capability with a ten-minute life — treat it like a
   password for those ten minutes.

3. **Open it on the phone.** Vlervcode opens and shows an incoming pairing from
   `Claude Code @ <your Mac>`, with **six words**.

4. **Compare the words.** Ask Claude Code for `pair_status`. It prints six
   words. They must be the same six words, in the same order, as the phone
   shows. **If they differ, stop** — something is between the two machines.
   Reject it: *"reject the pairing"* → `confirm_pairing { accept: false }`.

5. **Confirm on both sides.** Accept on the phone, and tell Claude Code
   *"the words match, confirm it"* → `confirm_pairing { accept: true }`.

6. **Grant control on the phone.** This is the step people miss. Pairing made
   the two machines know each other; it did not decide what each may do to the
   other. `confirm_pairing` set what the **phone** may do on this server. For
   the server to push files **to** the phone, the phone must grant the server
   the `control` scope:

   > On the phone, open Vlervcode's peer settings, find
   > **`Claude Code @ <your Mac>`**, and set its scope to **control**.

   Until then `send_to_device` is refused, with an error saying exactly this.

7. **Send something.** *"Send /Users/me/work/report.html to my iPhone"*. It
   lands in Vlervcode's received folder and opens in a tab.

Pairing a second Mac running Vlervtifacts is the same walkthrough; step 6 is its
Settings peer list instead.

## Where it keeps its files

`~/Library/Application Support/Vlerv/mcp/`

| Path | Content |
|---|---|
| `remote/identity.key` | The ed25519 secret key that IS this server's identity. Written `0600`. Deleting it changes the node id and orphans every pairing. |
| `remote/peers.json` | The devices this server trusts, and the scope each was granted here. Deleting an entry revokes it on the next request. |
| `remote/blobs/` | The content-addressed store staged files are served from. A staged copy is kept alive by a tag; when the last tag on it goes, the store's collector frees the bytes within a minute. |
| `remote/outbox/` | One `0600` record per send that was accepted for a device that did not answer, plus — in the store above — a private copy of the file itself, until it is delivered or expires. |
| `received/<date>/` | Files other devices pushed to this server. |

Nothing is ever written into your own source tree.

## Security model

**Two grants, one per direction.** Each side's `peers.json` says what the *other*
side may do *there*. `confirm_pairing { scope }` sets what a device may do on
this server. What this server may do on the device is set on the device. Pushing
a file is `control`, the widest scope, and it is never granted by default.

**The fingerprint is the whole point of pairing.** The six words derive from
both node ids, so a machine in the middle — which necessarily holds a different
key on each leg — cannot make the two screens agree. Skipping the comparison
skips the security.

**The path gate is a real boundary.** Every path argument passes
`security::canonicalize_and_check_root` over `VLERV_MCP_ROOTS` before anything
else happens, and then `beam::resolve_offerable`: a real file, under the
transfer size cap, resolvable on disk. A file outside every root is refused.

This is stricter than the desktop share sheet on purpose. There, a human picks
the file in a dialog. Here the caller is a language model, and its arguments can
be steered by text it merely read — a repository file, a fetched page, another
tool's output. Narrowing `VLERV_MCP_ROOTS` narrows what such a caller can ever
address.

**A link is revocable, not just expiring.** `stop_beam` drops the offer from the
registry the request gate consults, so revocation takes effect on the next
fetch. The TTL is the backstop for a link nobody remembered to stop.

**Received bytes are verified, not trusted.** A pushed artifact is BLAKE3-checked
chunk by chunk against the announced content address, capped on real measured
bytes, staged as `.partial` and only then moved into place.

**A pushed ticket is peer-locked.** The ticket in a push names the pushing peer;
a receiver refuses one that names a third machine, so a control peer can never
make it fetch from somewhere else.

**A queued send is a private copy of your file.** Accepting a send to a device
that is not there means copying the file into `remote/blobs/` and keeping it
there until it is delivered or the record expires — up to seven days. Nothing
leaves the machine in the meantime, and the copy is released the moment the
delivery lands. This is why a device that is known not to grant `control` is
refused outright instead of having files kept here for it.

**No stdout.** Stdout is the JSON-RPC channel. Every log line goes to stderr.

## Troubleshooting

**"… is not reachable"** — the device is asleep, off the network, or Vlervcode
is not running on it. Wake it, open the app, retry. A first connection from a
new network can take a few seconds while hole-punching works.

**"… has not granted this server control"** — step 6 of the walkthrough. The
message names the peer to widen and the scope to set.

**"no paired device matches …"** — the error lists the names that do exist. Use
one of those, or a node-id prefix.

**"path not found or out of root"** — one message for "does not exist" and "not
allowed", on purpose: a refusal must not tell a caller what exists. Check the
path, and check `VLERV_MCP_ROOTS` if you set it.

**The macOS firewall prompts on first use** — the server binds a UDP socket to
accept direct connections. Allow it, or the transfer falls back to a relay and
gets slower.

**A beam link stopped working** — the server process ended (the Claude Code
session closed) or the TTL expired. Mint a new one.

**A send came back "queued"** — the device did not answer, and the file is
waiting here instead of failing. Open Vlervcode on that device. A phone that
has been paired once turns its own **Listen at launch** switch on, so opening
the app both starts it listening and dials this server, and that dial is what
sends the file. If the switch was turned off afterwards, the app opens without
a socket, and the file waits until something on the phone opens its network —
**Connect** in its peer settings, or opening a remote device from it.
`server_status` shows every waiting record with the last error it hit.

**"another Vlerv process is already using the blob store"** — one state
directory serves one process at a time, and every Claude Code session starts
its own `vlerv-mcp`. That claim is also what makes one process the only one
that may move the queue: a session without it can list queued sends in
`server_status` but delivers none of them, and says so in
`queue_blocked_reason`. Close the other session, or give this one its own
`VLERV_MCP_STATE_DIR`. A separate state directory means a separate node id, so
that server starts with no paired devices. (The message itself says "its own
state directory" rather than naming a variable: the same check guards
Vlervtifacts. This server prefers `VLERV_MCP_STATE_DIR` and falls back to
`VLERV_STATE_DIR`; the app reads `VLERV_STATE_DIR` only.)

Watch for orphaned headless runs: `claude --print` sessions keep their server
alive, and one that never exits holds the store indefinitely. Find them with
`pgrep -fl vlerv-mcp`.

**Nothing appears in `/mcp`** — check `claude mcp list`, and confirm the path
you registered is absolute and the binary is executable. Run it by hand: it
should print one identity line to stderr and then wait.
