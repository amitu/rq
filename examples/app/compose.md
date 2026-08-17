---
method: POST
url: '{{host}}/posts'
headers:
  Content-Type: application/json
---

-- description --

Write a post

-- form --

text: { label: "What's happening?", required: true, multiline: true, help: "280 characters, like the old days" }
author: { label: "Posting as", default: '{{me}}' }

-- body --

{"text": "{{text}}", "author": "{{author}}"}

-- view --

Posted as **@{{ response.author }}**.

{{ response.text }}

- [see it](rq:post?id={{ response.id }}) · [back to the timeline](rq:timeline)
