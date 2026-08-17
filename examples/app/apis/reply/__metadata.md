---
method: POST
url: '{{host}}/posts'
headers:
  Content-Type: application/json
vars:
  reply_to: { prompt: "Replying to post", required: true }
---

-- description --

Reply to a post

-- form --

text: { label: "Your reply", required: true }
author: { label: "Posting as", default: '{{me}}' }

-- body --

{"text": "{{text}}", "author": "{{author}}", "reply_to": {{reply_to}}}

-- view --

Replied.

- [see the thread](rq:post?id={{ response.reply_to }}) · [timeline](rq:timeline)
