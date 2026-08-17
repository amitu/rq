---
url: '{{api}}/repos/{{owner}}/{{repo}}/issues/{{number}}/comments'
vars:
  number: { prompt: "Issue number", required: true }
---

-- description --

The conversation on one issue.

-- view --

# {{ response | length }} comment(s) on #{{ vars.number }}

{% for c in response %}
**@{{ c.user.login }}** · {{ c.created_at | date('YYYY-MM-DD HH:mm') }}

{{ c.body }}

---
{% endfor %}

[back to the issue](rq:issue?number={{ vars.number }})
