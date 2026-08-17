---
url: '{{api}}/user/starred'
query:
  per_page: '{{per_page}}'
vars:
  per_page: 10
---

-- description --

What you have starred recently.

-- view --

# Recently starred

| Repo | ★ |
|---|---:|
{% for r in response %}| [{{ r.full_name }}](rq:repo?owner={{ r.owner.login }}&repo={{ r.name }}) | {{ r.stargazers_count }} |
{% endfor %}
