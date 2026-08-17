---
url: '{{host}}/timeline'
query:
  limit: '{{limit}}'
vars:
  limit: 10
---

-- description --

The home page.

-- view --

# Timeline · {{ response.total }} posts

{% for p in response.posts %}**@{{ p.author }}** · {{ p.at | date('HH:mm') }} · ♥ {{ p.likes }}
  {{ p.text }}
  [open](rq:post?id={{ p.id }}) · [♥ like](rq:like?id={{ p.id }}) · [@{{ p.author }}](rq:person?handle={{ p.author }})

{% endfor %}
---

[write a post](rq:compose) — or press `f` to fill the form here
