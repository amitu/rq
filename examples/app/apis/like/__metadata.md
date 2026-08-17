---
method: POST
url: '{{host}}/posts/{{id}}/like'
vars:
  id: { prompt: "Post id", required: true }
---

-- description --

Like a post — a link that changes something, then shows you the result.

-- view --

♥ {{ response.likes }} on @{{ response.author }}'s post.

{{ response.text }}

- [open it](rq:post?id={{ response.id }}) · [timeline](rq:timeline)
