---
url: '{{api}}/repos/{{owner}}/{{repo}}/commits'
query:
  per_page: '{{per_page}}'
vars:
  per_page: 10
---

-- description --

The last few commits, newest first.

-- view --

# Recent commits

| SHA | Message | Author | When |
|---|---|---|---|
{% for c in response %}| `{{ c.sha[:7] }}` | {{ c.commit.message.split('\n')[0] }} | {{ c.commit.author.name }} | {{ c.commit.author.date | date('YYYY-MM-DD') }} |
{% endfor %}

[the repo](rq:repo)
