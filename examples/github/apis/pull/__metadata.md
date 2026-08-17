---
url: '{{api}}/repos/{{owner}}/{{repo}}/pulls/{{number}}'
vars:
  number: { prompt: "PR number", required: true }
---

-- view --

# #{{ response.number }} · {{ response.title }}

**@{{ response.user.login }}** wants to merge `{{ response.head.ref }}` into `{{ response.base.ref }}`

{{ response.additions }} additions, {{ response.deletions }} deletions across {{ response.changed_files }} file(s) · {{ response.commits }} commit(s)

---

{{ response.body }}

---

- [{{ response.comments }} comment(s)](rq:comments?number={{ response.number }})
- [all pull requests](rq:pulls) · [the repo](rq:repo)
