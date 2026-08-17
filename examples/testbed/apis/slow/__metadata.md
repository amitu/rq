---
url: '{{host}}/delay/{{ms}}'
vars:
  ms: 300
timeout: 5000
---

-- description --

Sleeps before answering. Good for watching the `waiting` phase in
`rq r slow --show timing`, or for proving `timeout:` works: try `--var ms=6000`.
