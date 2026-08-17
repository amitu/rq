---
headers:
  Accept: application/vnd.github+json
  X-GitHub-Api-Version: '2022-11-28'
auth: { type: bearer, token: '{{GH_TOKEN}}' }
---

-- description --

Everything below inherits GitHub's media type, the API version header, and the token — one
place, because a collection is a directory and this file is what it has to say.

An empty `GH_TOKEN` means no `Authorization` header goes out at all, so the public requests
work anonymously.
