---
vars:
  api: https://api.github.com
  owner: anthropics
  repo: claude-code
  GH_TOKEN: { env: GH_TOKEN, secret: true }
---

-- description --

GitHub's public API.

`GH_TOKEN` is optional. Without it you get GitHub's anonymous rate limit (60 requests an
hour) and the requests under `mine/` won't work; with it, 5,000 an hour and everything
does. `rq` omits an empty credential rather than sending `Authorization: Bearer ` — which
GitHub answers with a 401 — so the workbook is useful either way.

    export GH_TOKEN=ghp_…      # or: rq r repo --var GH_TOKEN=ghp_…
