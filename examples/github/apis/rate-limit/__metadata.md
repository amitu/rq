---
url: '{{api}}/rate_limit'
---

-- description --

Start here: it works with or without a token, and tells you which you're using.

-- view --

# {{ response.resources.core.remaining }} / {{ response.resources.core.limit }} requests left

{% if response.resources.core.limit > 100 %}Authenticated.{% else %}Anonymous — export `GH_TOKEN` for 5,000/hour.{% endif %}

- [search quota]({{ 'rq:rate-limit' }}) · {{ response.resources.search.remaining }} / {{ response.resources.search.limit }}
