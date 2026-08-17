---
vars:
  GH_TOKEN: { env: GH_TOKEN, secret: true, required: true }
---

-- description --

These need a token — they're about *you*, so there is nothing to see anonymously.
`required: true` here means the run stops with a clear message instead of a puzzling 401.
