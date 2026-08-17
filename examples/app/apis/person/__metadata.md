---
url: '{{host}}/people/{{handle}}'
vars:
  handle: { prompt: "Whose posts?", required: true }
---

-- view --

# @{{ response.handle }}

{{ response.posts }} posts · ♥ {{ response.likes }} received

{% for p in response.timeline %}- {{ p.text }} · [open](rq:post?id={{ p.id }})
{% endfor %}

[timeline](rq:timeline)
