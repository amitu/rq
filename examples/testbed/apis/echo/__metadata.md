---
method: POST
url: '{{host}}/echo'
query:
  trace: '{{trace}}'
headers:
  Content-Type: application/json
  X-Trace: '{{trace}}'
vars:
  trace: { default: abc123, prompt: "Trace id" }
---

-- description --

Mirrors whatever arrived. Handy for "did it actually send what I think it sent?" —
`rq r echo --show request` next to this output answers that.

-- body --

{"hello": "world", "trace": "{{trace}}"}

-- view --

{{ response.method }} {{ response.path }} — {{ response.body_bytes }} bytes in
X-Trace as received: {{ response.headers['x-trace'] }}
