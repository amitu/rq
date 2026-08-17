---
url: '{{host}}/posts/{{id}}'
vars:
  id: { prompt: "Post id", required: true }
---

-- description --

One post, its replies, and what you can do about it.

-- view --

# @{{ response.author }}

{{ response.text }}

♥ {{ response.likes }} · {{ response.at | date('YYYY-MM-DD HH:mm') }}

- [♥ like this](rq:like?id={{ response.id }}) · [reply](rq:reply?reply_to={{ response.id }})
- [@{{ response.author }}'s posts](rq:person?handle={{ response.author }}) · [timeline](rq:timeline)

{% if response.replies %}
## {{ response.replies | length }} repl{% if response.replies | length == 1 %}y{% else %}ies{% endif %}

{% for r in response.replies %}**@{{ r.author }}** · {{ r.text }} · [open](rq:post?id={{ r.id }})
{% endfor %}{% endif %}
