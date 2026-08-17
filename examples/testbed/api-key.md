---
url: '{{host}}/api-key'
auth: { type: api_key, key: api_key, value: '{{api_key}}', in: query }
---

-- description --

The same credential in the query string instead of a header — `in: header` moves it.
