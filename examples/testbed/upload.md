---
method: POST
url: '{{host}}/upload'
form_data:
  caption: from rq
  file: '@sample.txt'
---

-- description --

A multipart body. `@path` makes a field a file part. The path is relative to where you run `rq`, so
run this one from `examples/testbed/`.

-- view --

{{ response.count }} part(s):
{% for p in response.parts %}- {{ p.name }}{% if p.filename %} ({{ p.filename }}, {{ p.size }} bytes){% endif %}
{% endfor %}
