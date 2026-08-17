---
url: '{{api}}/repos/{{owner}}/{{repo}}/releases'
query:
  per_page: '{{per_page}}'
vars:
  per_page: 5
---

-- description --

Recent releases, with their notes rendered.

-- view --

# Releases

{% for r in response %}## {{ r.name or r.tag_name }} · {{ r.published_at | date('YYYY-MM-DD') }}

{{ r.body }}

---
{% endfor %}

[the repo](rq:repo)
