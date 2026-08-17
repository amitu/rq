---
url: '{{api}}/search/repositories'
query:
  q: '{{q}}'
  per_page: '{{per_page}}'
  sort: stars
vars:
  q: { default: 'api client cli', prompt: "Search repositories" }
  per_page: 10
---

-- description --

Search, and jump into any result — `rq r search --var q=user:amitu` lists someone's repos.

-- view --

# {{ response.total_count }} repositories matching `{{ vars.q }}`

| Repo | ★ | What |
|---|---:|---|
{% for r in response.items %}| [{{ r.full_name }}](rq:repo?owner={{ r.owner.login }}&repo={{ r.name }}) | {{ r.stargazers_count }} | {{ r.description }} |
{% endfor %}
