---
url: '{{host}}/basic-auth'
auth: { type: basic, username: '{{user}}', password: '{{password}}' }
---

-- description --

`rq` builds the Authorization header from `auth:` — the password stays a secret variable
and is masked in `--show request`.
