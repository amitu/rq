---
url: '{{api}}/repos/{{owner}}/{{repo}}/contributors'
query:
  per_page: '{{per_page}}'
vars:
  per_page: 15
---

-- description --

Who has committed here, and how much.

-- view --

# Contributors

| Who | Commits |
|---|---:|
{% for c in response %}| [@{{ c.login }}](rq:user?login={{ c.login }}) | {{ c.contributions }} |
{% endfor %}

[the repo](rq:repo)
