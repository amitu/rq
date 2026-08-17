---
url: '{{host}}/issues'
query:
  state: open
  per_page: 5
vars:
  state: open
---

-- description --

A list worth rendering — the `-- view --` below is why this tool exists.

-- view --

# {{ response | length }} {{ vars.state }} issues

| # | Title | Author | Comments |
|---|---|---|---:|
{% for i in response %}| [#{{ i.number }}](rq:issue?number={{ i.number }}) | {{ i.title }} | @{{ i.user.login }} | {{ i.comments }} |
{% endfor %}

Follow a row with `--follow N`, or press its number in `--console`.
