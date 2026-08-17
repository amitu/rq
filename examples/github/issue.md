---
url: '{{api}}/repos/{{owner}}/{{repo}}/issues/{{number}}'
vars:
  number: { prompt: "Issue number", required: true }
---

-- description --

One issue, rendered as something you'd actually read.

-- view --

# #{{ response.number }} · {{ response.title }}

**@{{ response.user.login }}** opened this {{ response.created_at | date('YYYY-MM-DD') }} · {{ response.state }}{% if response.labels %} · {% for l in response.labels %}`{{ l.name }}` {% endfor %}{% endif %}

---

{% if response.body %}{{ response.body }}{% endif %}

---

- [{{ response.comments }} comment(s)](rq:comments?number={{ response.number }})
- [@{{ response.user.login }}](rq:user?login={{ response.user.login }}) · [all issues](rq:issues) · [the repo](rq:repo)
