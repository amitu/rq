---
url: '{{host}}/issues'
query:
  state: open
  per_page: 5
vars:
  state: open
---

-- description --

A list worth rendering — the `-- view --` below is why this tool exists.

-- view --

# {{ response | length }} {{ vars.state }} issues

| # | Title | Author | Comments |
|---|---|---|---:|
{% for i in response %}| [#{{ i.number }}](rq:issue?number={{ i.number }}) | {{ i.title }} | @{{ i.user.login }} | {{ i.comments }} |
{% endfor %}

Follow a row with `--follow N`, or press its number in `--console`.

-- post --

rq.test('the list came back', () => {
  if (rq.response.status !== 200) throw new Error('status was ' + rq.response.status);
});
rq.test('it has five issues', () => {
  if (JSON.parse(rq.response.body).length !== 5) throw new Error('wrong count');
});
console.log('checked', JSON.parse(rq.response.body).length, 'issues');
rq.variables.set('first_issue', String(JSON.parse(rq.response.body)[0].number));
