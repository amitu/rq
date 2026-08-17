---
url: '{{api}}/users/{{login}}'
vars:
  login: { default: '{{owner}}', prompt: "GitHub username" }
---

-- view --

# {{ response.name or response.login }} (@{{ response.login }})

{{ response.bio }}

{{ response.public_repos }} repos · {{ response.followers }} followers · joined {{ response.created_at | date('YYYY-MM-DD') }}
{% if response.company %}works at {{ response.company }} · {% endif %}{% if response.location %}{{ response.location }}{% endif %}

- [their repositories](rq:search?q=user:{{ response.login }})
- [back to {{ vars.owner }}/{{ vars.repo }}](rq:repo)
