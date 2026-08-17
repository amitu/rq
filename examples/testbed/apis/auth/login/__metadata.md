---
method: POST
url: '{{host}}/auth/login'
headers:
  Content-Type: application/json
capture:
  token: response.access_token
---

-- description --

Sign in. Two things come back and both matter to `rq`:

- `access_token` in the body, which `capture:` lifts into `{{token}}` for `me`
- a `session` cookie, which the run's jar picks up on its own for `me-by-cookie`

-- body --

{"user": "{{user}}", "pass": "{{password}}"}

-- view --

Signed in. Token expires in {{ response.expires_in }}s.
