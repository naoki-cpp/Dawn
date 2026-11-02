# Render disposable playtest

One free web service for a few trusted native desktop testers, not production.
See [ADR-0056](../adr/ADR-0056-invited-disposable-playtest.md).

## Deploy

1. Push this change to GitHub through a PR. In the Render dashboard choose
   New > Blueprint and connect this repository/branch containing `render.yaml`.
2. Confirm **Free**, one `dawn-playtest` service, and no database/disk resources.
   Singapore is selected for a Japan-based test; region cannot be changed later.
3. Supply `DAWN_INVITE_TOKENS` as a secret JSON array of individually generated
   64-character hex tokens, e.g. `["<tester-one-token>","<tester-two-token>"]`.
   Placeholders will be rejected. Never commit real tokens or paste them into
   an issue, PR, screenshot, URL, or build/start command.
4. Deploy manually. The pinned Rust toolchain builds `simulate` with the locked
   dependencies. It listens on Render's assigned `PORT`; missing or malformed
   invites fail startup rather than opening an unauthenticated listener.
5. Copy the service's public hostname. The client URL is
   `wss://<assigned-hostname>.onrender.com`, without an explicit internal port.

Generate each token locally with a cryptographically secure generator. For
example, Python's standard library (run once per tester):

```console
python -c "import secrets; print(secrets.token_hex(32))"
```

The administrator keeps the full list; distribute only each tester's own token
over a private channel. To revoke one, remove it from the list and restart the
service. This resets the game too. Existing connections are not dynamically
revalidated. Invites are bearer capabilities, not persistent player accounts.

## Native Godot client

Start the client and enter the service's `wss://` URL and your individual
invite code in the connection dialog, then select Connect. The code is masked,
cleared from the input after submission, and never saved to a file. It remains
in connection memory for reconnects until cancellation, timeout, or process exit.
The dialog closes only after Welcome; Cancel stops retries and clears the code
and resume ticket. An unsuccessful connection times out after 90 seconds.
For a local server, use `ws://127.0.0.1:7878` with an empty invite code.

Developers can optionally prefill the dialog using environment variables:

Close the old editor first; an already-running editor does not inherit new
environment values. In a new PowerShell session:

```powershell
$env:DAWN_SERVER_URL = "wss://<assigned-hostname>.onrender.com"
# Read without echo or putting the actual token in shell history.
$secret = Read-Host "Your invite token" -AsSecureString
$env:DAWN_INVITE_TOKEN = [System.Net.NetworkCredential]::new("", $secret).Password
& .\.tools\godot\4.7.2-stable\Godot_v4.7.2-stable_win64.exe --editor --path client
```

Use F5 to play, then select Connect. A freshly built GDExtension is required as usual. Clear the two
environment variables and reopen the editor to return to localhost. Tokens
exist in the process environment/memory; do not share process dumps. Browser
exports are unsupported because they cannot set custom handshake headers.

## Limits and verification

- All ships, inventories, Currency, orders and sessions reset on each cloud
  process start. Market SQLite is intentionally temporary even if the process
  restarts without Render replacing its filesystem. Do not test with valuable
  or private data. InMemoryJournal does not provide production durability.
- The journal grows during a running session; keep tests short, monitor RAM,
  and manually restart between sessions. The population cap is not a complete
  memory/DoS bound. No large-battle or sustained free-plan capacity is claimed.
- The free service may sleep after idle time and take time to wake. Native
  WebSocket heartbeat detects connection health; do not run artificial traffic
  to evade free-plan limits. This is not an always-on server.
- Default TCP health checks are used. There is no HTTP health endpoint; plain
  browser navigation is not a game connection and will fail the WS handshake.
- WSS terminates at Render. Invalid/missing invites receive HTTP 401 before
  Hello; connection logs must not contain tokens. A valid invite starts a fresh
  session; reconnect within the same process still uses ResumeTicket.
- Incoming messages/frames are capped at 64 KiB; at most eight concurrent
  handshakes and eight queued single-node admissions are retained. These do
  not replace production connection/output rate limiting.
- Single Sector only: cross-Sector gate travel is not available in this setup.
- No extra authentication server/Redis/Postgres; no automatic deploys. Review
  Render's current quotas, billing and spend settings before confirming create.

Local tests cover configuration, bearer matching, pre-Hello rejection and
credential pinning. Cloud provisioning, proxy forwarding, cold start, and
remote gameplay must be verified after connecting the Render account.

Official references: [Web services](https://render.com/docs/web-services),
[WebSocket/WSS](https://render.com/docs/websocket),
[Free service limits](https://render.com/docs/free),
[Blueprint fields](https://render.com/docs/blueprint-spec).
