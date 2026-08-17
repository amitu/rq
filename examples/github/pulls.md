---
url: '{{api}}/repos/{{owner}}/{{repo}}/pulls'
query:
  state: '{{state}}'
  per_page: '{{per_page}}'
vars:
  state: open
  per_page: 10
---

-- description --

Open pull requests. Every row opens one.

-- view --

# {{ response | length }} {{ vars.state }} pull requests

| # | Title | Author | Branch |
|---|---|---|---|
{% for p in response %}| [#{{ p.number }}](rq:pull?number={{ p.number }}) | {{ p.title }} | [@{{ p.user.login }}](rq:user?login={{ p.user.login }}) | `{{ p.head.ref }}` |
{% endfor %}

[the repo](rq:repo)
