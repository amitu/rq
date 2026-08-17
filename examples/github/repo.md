---
url: '{{api}}/repos/{{owner}}/{{repo}}'
---

-- description --

A repository, and the front page of this workbook: every link below opens another request.

-- view --

# {{ response.full_name }}

{{ response.description }}

★ {{ response.stargazers_count }} · forks {{ response.forks_count }} · open issues {{ response.open_issues_count }} · {{ response.language }}

- [issues](rq:issues) · [pull requests](rq:pulls) · [commits](rq:commits) · [releases](rq:releases)
- [contributors](rq:contributors)
- [owner: @{{ response.owner.login }}](rq:user?login={{ response.owner.login }})

Default branch `{{ response.default_branch }}`, updated {{ response.updated_at | date('YYYY-MM-DD') }}.
