---
vars:
  host: http://127.0.0.1:8087
  user: amitu
  password: { default: hunter2, secret: true }
  api_key: { default: key-9f8e7d, secret: true }
---

-- description --

Points at a locally running `rq-testbed`. Start it with `cargo run -p rq-testbed`.

Every request here uses `{{host}}`, so a different port is just
`rq r <name> --var host=http://127.0.0.1:PORT`.
