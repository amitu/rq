---
url: '{{api}}/repos/{{owner}}/{{repo}}/contributors'
query:
  per_page: '{{per_page}}'
vars:
  per_page: 15
---

-- view --

# Contributors

| Who | Commits |
|---|---:|
{% for c in response %}| [@{{ c.login }}](rq:user?login={{ c.login }}) | {{ c.contributions }} |
{% endfor %}

[the repo](rq:repo)
