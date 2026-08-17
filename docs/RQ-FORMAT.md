# The `rq` file format

> One Markdown file per request. Frontmatter for the structured bits, named sections for
> everything that is prose or code. This is the format the `rq` CLI reads and writes.
>
> It is **not** the Requestly `LOCAL_FS` tree — that format splits a request across a dozen
> JSON files and is specified in [`FORMAT.md`](./FORMAT.md). `rq` uses the single-file form
> that `FORMAT.md` §10 describes as the north star; this document is that form, made real.

---

## 1. A request

```markdown
---
method: GET
url: https://api.github.com/repos/{{owner}}/{{repo}}/issues
headers:
  Accept: application/vnd.github+json
  Authorization: Bearer {{GH_TOKEN}}
query:
  state: open
  per_page: 5
vars:
  owner: { default: anthropics, prompt: "Repository owner" }
  repo: { default: claude-code }
  GH_TOKEN: { env: GH_TOKEN, secret: true, required: true }
parents: []
---

-- description --

List open issues for a repository.

-- view --

# {{ response | length }} open issues in **{{ vars.owner }}/{{ vars.repo }}**

| # | Title | Author |
|---|---|---|
{% for i in response %}| #{{ i.number }} | {{ i.title }} | @{{ i.user.login }} |
{% endfor %}

-- post --

rq.test('200 OK', () => rq.response.status === 200);
```

Everything about one request, top to bottom, in a file you can `cat`, `git diff`, and
hand-edit. `rq e <name>` opens it in `$EDITOR`.

---

## 2. Frontmatter

Required: `url`. Everything else is optional.

| Key | Type | Meaning |
|---|---|---|
| `method` | string | HTTP verb. Default `GET`. |
| `url` | string | The URL, templates intact. |
| `headers` | map | Request headers, in file order. |
| `query` | map | Query parameters, appended to the URL and percent-encoded. |
| `path_vars` | map | Fills `{name}` and `:name` placeholders in the URL. |
| `vars` | map | Declared inputs — see §4. |
| `capture` | map | Variables extracted from **this** response for dependents — see §5. |
| `parents` | list | Requests that must run first — see §5. |
| `auth` | map | See §3. |
| `form` | map | `application/x-www-form-urlencoded` body. |
| `form_data` | map | `multipart/form-data` body; a value of `@path` is a file part. |
| `file` | string | Send this file as the whole body. |
| `body_type` | string | Media type for the `-- body --` section. `json`, `xml`, `text/csv`, … |
| `timeout` | number | Milliseconds for the whole request. `0` = no limit. |
| `follow_redirects` | bool | Default `true`. |
| `verify_tls` | bool | Default `true`. |

**Reading rules.** Numbers and booleans where a string belongs are coerced and reported
(`per_page: 5` → `"5"`). Keys `rq` doesn't know are **kept verbatim** and re-emitted on
write, with a note and a did-you-mean — a file written by a newer `rq` survives an older
one. Genuinely ambiguous input (a mapping where a scalar belongs) is an error.

A request has **one** body. Declaring two (`form:` and a `-- body --` section, say) is an
error rather than a guess.

---

## 3. Auth

```yaml
auth: { type: basic,   username: u, password: "{{PASSWORD}}" }
auth: { type: bearer,  token: "{{TOKEN}}" }         # sends "Bearer <token>"
auth: { type: bearer,  token: "{{TOKEN}}", prefix: null }   # sends the bare token
auth: { type: api_key, key: X-Api-Key, value: "{{KEY}}", in: header }   # or in: query
auth: inherit    # explicitly take the enclosing collection's auth (the default)
auth: none
```

`prefix` is tri-state on purpose: absent = `Bearer`, a string = that string, `null` = no
prefix at all. Collapsing it silently changes what goes on the wire.

An auth type this build can't send (`oauth_2`, `hawk`, …) is **preserved in the file** and
reported on the run — a credential is never stripped just because it wasn't understood.

**An empty credential is not a credential.** If the token, key value, or both halves of a
basic pair resolve to nothing — unset, or still a literal `{{VAR}}` — the header is not
sent, and the run says so. That is what lets a collection declare
`auth: { type: bearer, token: '{{GH_TOKEN}}' }` for everyone and stay usable by someone who
hasn't set a token, instead of turning every public request into a 401.

An explicit `Authorization` header always wins over generated auth.

---

## 4. Variables

`{{name}}` anywhere in the url, headers, query, path vars, or body. Resolution order,
highest first — first write wins:

```
--var (command line)  >  capture (from a parent)  >  active environment
                      >  __global  >  declared defaults (request, then collection)
```

A declared variable says where its value comes from:

```yaml
vars:
  owner: anthropics                                   # shorthand for { default: … }
  repo:     { default: claude-code, prompt: "Repo" }
  GH_TOKEN: { env: GH_TOKEN, secret: true, required: true }
```

- `default` — used when nothing above supplies a value.
- `prompt` — the label to ask with. `rq r x --prompt` asks for every declared variable.
- `env` — read this process environment variable when no higher scope has a value.
- `secret` — never echoed when prompted, and masked in `--show request` and in printed
  output.
- `required` — the run fails rather than sending an empty value.

An unresolved `{{name}}` is **left exactly as written** and reported. A request that goes
out with a literal `{{token}}` is a visible bug; one that goes out with an empty header is
a mystery. `--strict` turns any such note into a non-zero exit.

---

## 5. Chaining — `parents:` and `capture:`

```yaml
# login/__metadata.md
method: POST
url: https://api.example.com/auth/login
capture:
  token: response.access_token
```

```yaml
# me/__metadata.md
url: https://api.example.com/me
headers:
  Authorization: Bearer {{token}}
parents: [login]
```

`rq r me` runs `login` first, captures `token` from its response, and uses it. The graph is
declared per request — no orchestration file, no `run.sh`. A parent shared by two children
runs once per invocation. Cycles are refused, with the cycle printed.

A bare name resolves to the sibling request first, then outward through the enclosing
collections, then across the project.

`capture:` values are paths into the same context a `-- view --` template sees (§7):
`response.access_token`, `response.items[0].id`, `headers.etag`, `status`.

**Cookies.** A run keeps a cookie jar, so the other common shape of a chain — the server
sets a session with `Set-Cookie` and expects it back — works with nothing declared. Host
and path matching, `Secure`, and `Max-Age=0` deletion are honoured; `Expires` is not
evaluated, because a run lasts seconds and a wrong date parser silently dropping a live
cookie would be worse. The jar lives for one invocation and is **never written to disk**: a
terminal client that quietly persisted your session cookies would be storing credentials
you never asked it to keep.

---

## 6. Sections

`-- name --` on a line of its own opens a section; it runs to the next marker or the end of
the file. A markdown rule (`---`) or an em-dash sentence is never mistaken for one.

| Section | What it is |
|---|---|
| `-- description --` | Free markdown, for your future self. |
| `-- view --` | The response render template (§7). |
| `-- body --` | The request body — raw text, JSON, XML, GraphQL. |
| `-- pre --` | JavaScript to run before the request. **Not executed by this build.** |
| `-- post --` | JavaScript to run after. **Not executed by this build.** |
| `-- form --` | Reserved for terminal input forms. Not implemented. |

Unknown sections are preserved verbatim, like unknown frontmatter keys.

**Scripts.** `-- pre --` and `-- post --` are parsed, carried, and round-tripped, but this
build ships **no script engine**: every run that has one says so on stderr, and `--strict`
fails on it. The engine is [`cross-q-context`](./CONTEXT.md); `rq` hosts it rather than
implementing it.

What the host already does, so that a script behaves the same here as in the app the day
the engine lands:

| The script does | `rq` does |
|---|---|
| `rq.request.headers.add/upsert/remove/clear` | applies the change **before** the request is sent |
| `rq.vars.set(...)` / `rq.environment.set(...)` | writes into the same runtime layer `capture:` writes to, so the next request in the graph reads it |
| `rq.test(name, fn)` | prints ✓/✗ per assertion and **exits non-zero if any failed** |
| `console.log(...)` | prints under the step it came from |
| `rq.execution.skipRequest()` | doesn't send the request, and says so |
| `rq.execution.setNextRequest(...)` | is **refused out loud** — `rq` walks the graph a request declares with `parents:`, so there is no linear order to redirect |
| `rq.cookies.jar(host)` | is seeded from the run's cookie jar (below) |

**Scripts form a chain, and a collection's scripts wrap its requests.** Every request in
the run — not just the one you named — executes its own chain:

```
pre-request    root collection → … → nearest collection → the request
post-response  the request → nearest collection → … → root collection
```

Pre-request runs outermost-in, post-response innermost-out, so a collection's scripts
*surround* the requests beneath it rather than merely preceding them. Along the chain:

- variables one script sets are visible to the next, **and** are substituted into the
  request before it is sent — the request is re-prepared after every script;
- header changes accumulate in call order;
- a `skipRequest()` aborts the rest of the chain, because running later scripts for a
  request that will never be sent mutates state for a call that didn't happen.

This is the app's own execution order (ADR-061's "sandwich", plus ADR-020/167/169), matched
deliberately: a collection has to behave the same whether it runs here or there.

`--script-timeout <ms>` bounds each script. The CLI runs the **safe** engine only:
`developer` mode is `node:vm`, which is not a security boundary, and a terminal client that
ran a collection's scripts with host access would be a liability rather than a feature.

Scripts imported from Postman keep their `pm.*` source **verbatim** with the dialect noted —
a textual `pm.` → `rq.` rename imports clean and throws at run time, which is the one
failure this project refuses to ship.

---

## 7. The `-- view --` template

Jinja-compatible (via minijinja). The context:

| Name | What |
|---|---|
| `response` | The parsed JSON body, or the raw text when it isn't JSON. |
| `status` / `status_text` | `200` / `OK`. |
| `headers` | Response headers, keys lowercased. |
| `body` | The raw response text. |
| `time_ms`, `bytes` | Elapsed time and body size. |
| `vars` | Every resolved variable. |
| `request` | `{ method, url }` as sent. |

**A view can link to other requests, which makes it a page rather than a report.** A
markdown link whose target starts with `rq:` points at another request in the project:

```markdown
| [#{{ i.number }}](rq:issue?number={{ i.number }}) | {{ i.title }} |
```

Those links are numbered in the output (`#1287 [1]`), and the numbers are how you follow
them — `rq r issues --follow 1` on the command line, or **the digit keys in the console**
(`rq r repo -c`), where `backspace` goes back the way it does in a browser. Anything after `?`
becomes variables for the request being opened, layered over the ones the run already had —
so following a link differs from the page you were on by exactly what the link said.

An ordinary `http(s)://` link renders but is **not** numbered: following one would mean
issuing a request the project never described.

Filters: everything minijinja ships, plus `date('YYYY-MM-DD')` for ISO-8601 timestamps
(`HH`, `mm`, `ss` too; non-ISO input passes through untouched).

**An undefined name is an error, not an empty string.** A template that silently renders
`# open issues` because a field was renamed is worse than one that says what broke.

The result is markdown, rendered for the terminal: headings, emphasis, code, lists, and
column-aligned tables. `--raw` prints the response body instead.

---

## 8. The project

```
my-apis/
├── __requestly.json          # project marker: { version, include[], exclude[] }
├── apis/
│   ├── issues/__metadata.md  # a request
│   └── github/               # a collection — just a directory
│       ├── __collection.md   # optional: shared headers / auth / vars / description
│       ├── login/__metadata.md
│       └── me/__metadata.md
├── environments/
│   ├── __global.md
│   └── staging.md
└── .requestly/state.json     # the active environment (machine-local, gitignored)
```

`rq` finds the project the way `git` finds a repo: walk up from the cwd looking for
`__requestly.json`. `RQ_PROJECT` and `--project <dir>` override that.

**The tree is the hierarchy.** A request's collection is the directory above it; nothing
stores a parent id, so `git mv` is a legal way to reorganize. `__`-prefixed directories are
`rq`'s own and are never entities.

`apis/__collection.md` is the **project-wide** one: `apis/` is not itself a request or a
collection you can name, so that file is where "every request in this project sends these
headers" goes. Below it, each `__collection.md` uses the same frontmatter, and its
`headers`, `auth`, and `vars` are inherited by every request beneath it — nearer collections win, and a request always wins
over its collections. Its `-- pre --` / `-- post --` sections run around every request
beneath it too (§6).

**Environments** are the same document with only a `vars:` block, so there is one format to
learn. `__global.md` is the global environment; it applies under whichever environment is
active.

```markdown
---
vars:
  host: https://staging.example.com
  TOKEN: { env: STAGING_TOKEN, secret: true }
---
```

---

## 9. Converting in and out

The format is one of cross-q's supported formats, not a private detail of the CLI, so it
maps through the [Idealised Model](./IDEALISED.md) like every other:

```bash
cq convert acme.postman_collection.json --to rq  --output ./my-apis   # bring one in
cq convert ./my-apis --to postman --output ./out                       # take it anywhere
cq convert ./my-apis --to bruno   --output ./out
cq convert ./my-apis --to requestly --output ./out   # the Requestly LOCAL_FS tree
```

A directory is detected as an `rq` project by its `__requestly.json` (or any
`__metadata.md`); a lone `.md` file with frontmatter is read as a single request. `rq
import` calls exactly this — the CLI owns no conversion of its own.

Two gates hold the pair honest, both in `crates/cross-q/tests/rq_format.rs`:

- **Idempotence** — `rq` → IR → `rq` → IR recovers the same model. If the emitter drops or
  reshapes a field, the test names it.
- **No hollow conversion** — over the pinned real-world Postman corpus, every request that
  enters the model is still there after a trip through the `rq` format.

What the format cannot yet hold is reported on the way out, per node: saved response
examples, reusable script packages, disabled headers (there is no disabled flag), and
non-HTTP protocols. An auth type the CLI can't *send* is still written to the file in full
and read back whole — it is reported, never stripped.

---

## 10. Not implemented yet

Named so you don't have to discover it:

- **Scripts** (`-- pre --` / `-- post --`) parse and round-trip, but do not execute — the
  host side is built and tested (§6), the engine is not here yet.
- **`-- form --`** is reserved; nothing reads it.
- **The interactive project browser** — bare `rq` prints the tree; it doesn't yet let you
  arrow around it, run, or edit from there. (The *post-run* console is real: `rq r x -c`.)
- **Terminal-width-aware tables** — columns are aligned to their content, so a table with
  very long cells is wider than an 80-column window and wraps. Nothing is truncated;
  narrow the column in the template (`{{ i.title | truncate(60) }}`) if you want it short.
- **Saved response examples** and data-driven iteration.
- Protocols other than HTTP. GraphQL imports as a JSON POST body.

**Timing, precisely.** `--show timing` and the console's timing pane break a request into
DNS, TCP, waiting and download — measured inside the HTTP stack, not estimated. The **TLS
handshake falls inside `waiting`**, because ureq completes it lazily on first use rather
than during connect. That was measured rather than assumed: against one host, an `https`
request reports a *smaller* TCP phase than plain `http` and a correspondingly larger wait.
A "TCP+TLS" figure would have looked better and told you less.

Everything in §§1–8 is real, tested, and on the wire.
