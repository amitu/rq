---
url: '{{host}}/issues/{{number}}'
vars:
  number: { prompt: "Issue number", required: true }
---

-- description --

Where a row in the issues table leads. `rq r issues --follow 1` gets here without you
typing a number — that is what makes a view a page rather than a report.

-- view --

# {{ response.title }}

state: {{ response.state }}

[back to the list](rq:issues)
