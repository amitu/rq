---
url: '{{host}}/me'
parents: [login]
---

-- description --

The same endpoint with **no** Authorization header. It works because the cookie jar kept
the `session` cookie `login` set — the half of chaining that needs nothing declared.

-- view --

Authenticated via {{ response.authenticated_via }}.
