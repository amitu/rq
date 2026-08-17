---
url: '{{api}}/repos/{{owner}}/{{repo}}/issues'
query:
  state: '{{state}}'
  per_page: '{{per_page}}'
vars:
  state: { default: open, prompt: "open, closed or all" }
  per_page: 10
---

-- description --

The issue list. Every row links to the issue itself — follow one and you're reading it.

-- view --

# {{ response | length }} {{ vars.state }} issues in {{ vars.owner }}/{{ vars.repo }}

| # | Title | Author | 💬 |
|---|---|---|---:|
{% for i in response %}| [#{{ i.number }}](rq:issue?number={{ i.number }}) | {{ i.title }} | [@{{ i.user.login }}](rq:user?login={{ i.user.login }}) | {{ i.comments }} |
{% endfor %}

[back to the repo](rq:repo)
