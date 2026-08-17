---
url: '{{api}}/user'
---

-- description --

You, according to your token.

-- view --

# {{ response.name or response.login }} (@{{ response.login }})

{{ response.public_repos }} public repos · {{ response.followers }} followers

- [my starred repos](rq:starred)
- [my profile as anyone sees it](rq:user?login={{ response.login }})
