# Handoff (2026-11-18, fourth session)

## State
- Branch **`beta`** (local only, **not pushed**) at `e884ebe`, one commit on top of
  `feat/full-app-orchestrator` (`5aeec8d`, in sync with origin). Everything from sessions 1–4
  is in that commit. The working tree is clean except this file and `.playwright-mcp/`
  (now in `.git/info/exclude`).
- `feat/full-app-orchestrator` and `main` are untouched.
- Copy the beta: `git clone -b beta /home/john/Projects/ApolloOfficeImproved ApolloOffice-beta`
- This file is not committed (and so is not in a clone).
- Tests: `cargo test` green (101 unit + 4 e2e). All `ui/*.js` pass `node --check`.

## What is in the beta commit
1. **Docker setup help** (`src/web.rs`, `ui/p.js`)
   - `docker_diag()` → `{ok, state: ok|missing|denied|stopped|slow, m, cmd, url, after, warn, start}`.
     The fix is structured data; the UI no longer parses prose.
   - A `docker info` timeout (`run` gives `c=-1` with `e="timeout after N s"`) is `slow`, not
     `missing`. Only a spawn failure is `missing`.
   - `denied` carries `warn` (docker group is root-equivalent) plus a rootless Docker link, and
     does not poll (a group change needs a new login).
   - `GET /api/check/docker` (15 s timeout) checks only Docker. The setup page polls it with a
     `setTimeout` chain (one request in flight, 3 s gap, max 60), tied to its own page element and
     a generation counter, so a re-rendered setup page or "Build without Docker" ends it.
     `runAuto` has a re-entry guard.
   - `POST /api/setup/docker` starts Docker Desktop (macOS/Windows).
   - Tools row says "not ready" instead of "missing" when Docker is installed but unusable.
   - Regression fixed: `setup()` bound `#bautoraw.onclick`, but that button only exists inside
     the Docker help now; it threw and "Check again" never got its handler.
   - Verified in the browser with mocked endpoints (4 s responses → max 1 in flight; one
     auto-setup after Docker came up; denied view rendered, no polling).
2. **Setup progress bar**: `src/prog.rs` (BuildKit parser, 3 tests), `GET /api/setup/progress`,
   `setupBar()` in `ui/p.js`, CSS `.spb*`.
3. **Starter office**: `eng::starter_office()` / `seed_office()`, built-ins on the floor
   (`ui/o.js`), "Use the starter office" button, `covered()` repair of saved scenes (`ui/l.js`).
4. **Sandbox watchdog**: `src/wd.sh` (kills stalled dependency fetches after `AO_STALL_SECS`,
   default 180; opt out `AO_NO_WATCHDOG=1`), network timeouts/retries for cargo/npm/pip in `PRE`.
5. **Phone layout**: nav drawer, office first, scrollable worker strip (`tnav()`, `data-nosb`).
   Not verified in a browser at phone widths (owner chose to skip this).
6. **Layout step 4** (`ui/l.js` `fit()`): in scene mode (not while editing), visible panels
   stacked in one column (same x/w, touching edges) share its height. A panel whose content needs
   less than its saved height shrinks to fit; the others split the rest in saved proportions.
   Saved values never change. Re-runs on `.bd` childList mutations (rAF-debounced) and resize;
   `data-fit` stops redundant resize events. Locked panels keep their size. New defaults
   Queue 30 / PRs 26 / Log 38 (old 36/29/29 is in `OLS`, so unchanged scenes migrate).
   Verified at 1440x900.

## Closed
- The stalled fixer job: marked resolved by the owner, not to be checked.
- Docker self-review items (timeout as "missing", overlapping polls, polling the full
  `/api/check`, prose parsing, docker group warning): all fixed in the beta commit.

## Next
1. Owner question open: a more prominent entry for huge requests. Today it is the "Ask" button
   (front desk chat, needs the orchestrator on; long paste becomes an attached spec, 512 KB
   limit). Proposal: a "New request" button with a full-size composer.
2. CSS cleanup: `@layer` cascade layers in `style.css`; merge the three duplicate
   `@media (min-width:861px)` grid blocks. Long term: one layout system (grid areas plus
   container queries) instead of absolute % scenes.
3. Watchdog follow-ups: a visible `cargo fetch` setup step with its own timeout; a "waiting on
   network" state per worker; tell worker prompts not to pipe long builds through `head`.
4. Phone layout checks (skipped this session): `data-nosb` fix, worker strip at 1280x720 and
   390px, the stray scrollbar stub at the right of the header.
5. Possible tweak: the Log height changes a little as Queue items come and go (fit-to-content).
   If that feels jumpy, limit shrinking to empty panels only.
6. When the beta is accepted: move the commit to `feat/full-app-orchestrator`
   (`git switch feat/full-app-orchestrator && git merge --ff-only beta`) and push on request.

## Test server
- Start: `echo 'exec cargo run -- --port 47301 --data /tmp/apollo-test' | newgrp docker`
  (`newgrp` gives docker group access without logging out).
- Stop by PID: `pgrep -f 'target/debug/apollo-office --port'`, then `kill <pid>`. Do not use
  `pkill -f` with a pattern that also matches your own shell.
- Login link: `./target/debug/apollo-office --data /tmp/apollo-test --port 47301 login-link`
- UI files are read from disk in debug builds: a page reload picks up `ui/` changes; Rust
  changes need a restart.
- The setup page only opens by itself when there are no repositories or machines. To test it,
  temporarily expose `setup` (e.g. `A.__setupTest = setup` next to `let asked` in `p.js`) and
  mock `fetch`; remove the hook afterwards.
- Test repository: `cute-johnny/testing-enviorment`. Sandbox image is built. A crew member
  "Scout" exists (hired by the owner); leave it alone.
- Browser: brave MCP. Navigating to the same URL with only a new hash does not reload; use
  `location.reload()`. Screenshots go under `.playwright-mcp/`.

## Commit rules
Commit or push only when the owner asks. Git has no identity configured globally: commit with
`git -c user.name=cute-johnny -c user.email=introvert69@protonmail.com commit ...` (the identity of
the existing commits). End messages with the Co-Authored-By line from the session.
