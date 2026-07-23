# Hoot! 🦉

Hoot! is a self-hosted, local-network quiz game for classrooms, parties, and small in-person events. The shared host screen shows the questions; player phones are private answer controllers.

## Run an event

Requirements: a recent Rust toolchain, Node.js 20 or newer, and npm.

```bash
npm install
npm --prefix frontend install
npm run build
npm start
```

Open `http://localhost:8080/host` on the shared display. Hoot prints the host recovery key and detected LAN join addresses in the terminal. The host page shows the selected address as a QR code.

At event time only the Axum process runs. It serves the built Svelte app, WebSocket API, and local media from port 8080. To change defaults:

```bash
npm start -- --games content/games.json --bind 0.0.0.0:8080
```

The equivalent environment variables are `HOOT_GAMES`, `HOOT_BIND`, `HOOT_FRONTEND`, and `HOOT_PUBLIC_URL`. Set `HOOT_PUBLIC_URL` when automatic interface detection chooses the wrong address.

### Windows / WSL2

If you run Hoot inside WSL2 on Windows, the automatically detected join address(es) are WSL2-internal and are **not reachable** from phones on your Wi-Fi by default — you'll see `172.x.x.x`-style addresses that only exist inside the WSL2 virtual network. The server detects this at startup and prints (and shows on the host screen) a warning when it applies. To fix it:

1. In Windows PowerShell, run `ipconfig` and find the IPv4 address of your real Wi-Fi or Ethernet adapter.
2. Either:
   - Start Hoot with `HOOT_PUBLIC_URL=http://<that-ip>:8080`, or
   - Enable WSL2 "mirrored" networking (Windows 11 22H2+): add `networkingMode=mirrored` under `[wsl2]` in `%UserProfile%\.wslconfig`, then run `wsl --shutdown` and restart WSL2. This makes WSL2 share the Windows host's real network directly, so auto-detection works normally.
3. You may also need a Windows Firewall inbound rule allowing the port (default 8080), since Windows Firewall can block LAN traffic to WSL2 even after networking is otherwise reachable.

For development, run `npm run dev`; Vite listens on port 5173 and proxies API/WebSocket traffic to Axum on port 8080.

## Author games

Edit `content/games.json`, then use **Reload content** on the host screen between games. Invalid changes are rejected without replacing the current catalog. A minimal question looks like:

```json
{
  "type": "multiple_choice",
  "id": "capital-france",
  "prompt": "What is the capital of France?",
  "timeLimitSeconds": 20,
  "readingTimeSeconds": 5,
  "doublePoints": false,
  "options": [
    { "id": "london", "text": "London" },
    { "id": "paris", "text": "Paris" }
  ],
  "correctOptionId": "paris"
}
```

Free-text questions replace `options` and `correctOptionId` with `"acceptedAnswers": ["Paris"]`. Matching ignores case, accents, punctuation, and repeated whitespace, then allows a small fixed typo tolerance based on answer length.

Images go in `content/media/` and are referenced by filename, for example `"image": "map.webp"`. Every image requires `imageAlt`. PNG, JPEG, WebP, and GIF files up to 10 MiB are accepted. Question and option text is plain text, never HTML.

## Sessions and recovery

Player and host browsers keep opaque session tokens in local storage. Scores, submissions, phase deadlines, and tokens live in the Hoot server's memory, so browser refreshes and temporary disconnects preserve the game while the server is running.

The host recovery key is generated and printed whenever the server starts. Keep it private. Entering it on `/host` transfers host control without resetting the running event. Stopping the server intentionally clears the game, scores, players, session tokens, and recovery key; after a restart, browsers automatically return to a fresh host claim or player join screen.

## Checks

```bash
npm test
npm run check
```
