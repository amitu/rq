---
url: '{{host}}/me'
headers:
  Authorization: Bearer {{token}}
parents: [login]
---

-- description --

Uses the captured token. `rq r me` runs `login` first — no orchestration file.

-- view --

**{{ response.name }}** ({{ response.email }})
  joined {{ response.joined_at | date('YYYY-MM-DD') }}
  plan: {{ response.plan }} · via {{ response.authenticated_via }}
