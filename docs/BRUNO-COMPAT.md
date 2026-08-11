# Bruno `bru.*` → `rq.*` compat mapping

Design reference for the Bruno phase of the `cross-q-context` compat layer — the AST rewrite +
runtime shim that lets Bruno pre-request / post-response / test scripts run on the `rq.*` runtime
(`docs/CONTEXT.md`). Companion to CONTEXT.md §3 (which specifies the `pm.*` phase).

**Sources:** Bruno JS API reference (docs.usebruno.com/testing/script/javascript-reference),
inbuilt libraries, testing/Chai docs, the safe-vs-developer runtime notes, and the `rq.*` surface
in `docs/CONTEXT.md` §2.

**Key structural fact:** `rq.*` was designed to mirror `pm.*` member-for-member, so the Postman
phase is a scope-aware *root swap*. **Bruno is not a namespace mirror** — its API is a different
shape (flat `bru.*` functions, `req`/`res` getter/setter *methods*, bare `test`/`expect`/`assert`
globals). So the Bruno phase reuses the same OXC + scope-stack + byte-offset-replacement engine,
but needs a richer replacement vocabulary (call→property, call→assignment, global→member) plus a
companion runtime shim for the long tail.

Mapping kinds: **rename** (member swap) · **reshape** (fn↔method↔property, args rearranged, same
behavior) · **semantic** (behavior differs — noted) · **gap** (no `rq.*` home).

---

## 1. Clean rename / reshape (the ~70% that dominates real scripts)

**Variable scopes** — the biggest, and all mechanical. Bruno's four scopes map to `rq`'s four,
with a fixed verb normalization:

| Bruno verb | `rq` verb |
|---|---|
| `getX(k)` | `.get(k)` |
| `setX(k,v)` | `.set(k,v)` |
| `hasX(k)` | `.has(k)` |
| `deleteX(k)` | `.unset(k)` |
| `deleteAllX()` | `.clear()` |
| `getAllX()` | `.toObject()` *(semantic: string-view vs typed — see §2)* |

Scope routing:
- `bru.getEnvVar/setEnvVar/...` → `rq.environment.*`
- `bru.getGlobalEnvVar/...` → `rq.globals.*` *(semantic — Bruno "global env" ≈ Postman globals niche)*
- `bru.getCollectionVar/...` → `rq.collectionVariables.*` *(rq mutators are silent no-ops with no collection context)*
- `bru.getVar/setVar/...` → `rq.variables.*` (merged runtime scope)

**Tests / flow / net:**
- `test(name, fn)` → `rq.test(name, fn)` (bare global → member; identical throw=fail semantics)
- `expect(x)` → `rq.expect(x)` (both Chai `ExpectStatic`)
- `bru.setNextRequest(n|null)` / `bru.runner.setNextRequest(n)` → `rq.execution.setNextRequest(n|null)`
- `bru.sendRequest(opts, cb?)` → `rq.sendRequest(input, cb?)` (input `{url, method?, header?, body?}` aligns; 4xx/5xx not an error)
- `bru.isSafeMode()` → `rq.isSafeMode` (**method→property**)
- `bru.runner.iterationIndex` → `rq.info.iteration`; `totalIterations` → `rq.info.iterationCount`
- iterationData read → `rq.iterationData.get/has/toObject` (read-only)

**`req` (method→property flips — the one twist vs the pm swap):**
- `req.getUrl()`→`rq.request.url`; `req.setUrl(u)`→`rq.request.url = u`
- `req.getMethod()`→`rq.request.method`; `setMethod`→assignment
- `req.getBody()`→`rq.request.body`; `setBody`→assignment
- `req.getName()`→`rq.info.requestName`
- `req.getHeader(n)`→`rq.request.headers.get(n)`; `setHeader(n,v)`→`rq.request.headers.upsert({key:n,value:v})`; `deleteHeader(n)`→`.remove(n)`
- `req.headerList.{get,has,all,add,upsert,remove,clear}` → `rq.request.headers.{...}` (core PropertyList ops map)

**`res` (read-only):**
- `res.getStatus()`/`res.status`→`rq.response.status`(/`.code`); `getStatusText`/`statusText`→`.statusText`
- `res.getBody()`/`res.body`→`rq.response.body` (or `.json()`/`.text()`); `getResponseTime`→`.responseTime`(/`.time`); `getSize`→`.size`
- `res.getHeader(n)`→`rq.response.headers.get(n)`

**Cookies (jar):** `bru.cookies.jar()`→`rq.cookies.jar()`; `setCookie(url,name,val)`→`jar.set(...)`;
`getCookie`→`jar.get`; `getCookies`→`jar.getAll`; `deleteCookie`→`jar.unset`; `deleteCookies`→`jar.clear(url)`.

**Globals:** `console`→`console`; `require('lodash')`/`_`→`_`; dynamic vars (`$guid`,`$randomInt`,…)→injected helpers.

---

## 2. Semantic (translatable, needs care — the ~20%)

- **`getAllX()` → `toObject()`**: `toObject()` is an always-string view; `getAllEnvVars()` preserves types. Type-sensitive consumers differ.
- **`setEnvVar(k,v,{persist:true})`**: `rq` has no persist flag — drop the option; in-memory vs on-disk lifetime differs.
- **`bru.getFolderVar` / `getRequestVar`**: `rq` has no folder/request scope — best-effort via merged `rq.variables`; may resolve to a stale/other value (**silent-wrong** — insidious).
- **`bru.getSecretVar(k)` → `rq.vault.get(k)`** (read-only vault).
- **`bru.runner.skipRequest()` → `rq.execution.skipRequest()`**: `rq` skip is **pre-request only** (throws elsewhere) — confirm phase.
- **`bru.runner.stopExecution()` → `rq.execution.setNextRequest(null)`**: closest analog, not a 1:1 verb.
- **`req.getHeaders()` / `res.getHeaders()`**: Bruno returns a plain object; `rq` `headers.all()` returns a list — reshape the shape.
- **`req.getHost/getPath/getQueryString/getPathParams`**: no direct accessors; compute from `new URL(rq.request.url)` / `rq.request.queryParams`.
- **`jar.getCookie` returns a cookie object in Bruno vs a value string in `rq`**; `jar.hasCookie` derived from `get`; `setCookies` looped over `jar.set`.
- **Request-scoped (URL-less) cookie list** (`bru.cookies.get/add/...`): `rq` jar is URL-scoped — only reachable by threading the request URL.
- **`expect(...).jsonBody()/.jsonSchema()`** (Bruno Chai extensions): in `rq` these live on `rq.response.to.have.*`, not a free `expect` chain — register as a Chai plugin at runtime rather than rewrite.
- **`bru.sleep(ms)`**: shimmable, but safe-mode timer delays are ignored (microtask-only) — timing degrades, doesn't crash.
- **`crypto-js` / `xml2js` / `tv4`**: `rq` offers native `crypto`, an `xml2Json` helper, and Ajv (`jsonSchema`) — different API surfaces, not drop-in.
- **`axios` / `node-fetch`** → rewrite to `fetch` / `rq.sendRequest`.

---

## 3. Gap list (~10%) — break vs warn-and-noop

**Safe to warn-and-noop** (script keeps running, loses a side effect):
`bru.disableParsingResponseJson()`, `setEnvVar(...,{persist})`, `req.setMaxRedirects/setTimeout/getTimeout/getTags/getAuthMode/getExecutionMode/getExecutionPlatform/onFail`, `bru.getTestResults/getAssertionResults` (→ `[]`), `bru.runner.stopExecution` (→ setNextRequest(null)), `bru.sleep` (microtask).

**Genuinely breaks scripts** (return value is load-bearing / no equivalent):
- **Node/host access** — `bru.cwd()`, `fs`/`process`/`Buffer`/`__dirname`, `require` of arbitrary npm, `bru.getProcessEnv(k)`. No `rq` home; fail hard.
- **`assert.*` (Chai `assert`)** — `rq` exposes only `expect`. **MUST be shimmed, never noop'd** — a vanished assertion turns a failing test green (high-risk).
- **`res.getUrl()`/`res.url` and `res.setBody()`** — `rq.response` has no URL and is read-only.
- **OAuth2 cred store** — `bru.getOauth2CredentialVar` / `resetOauth2Credential`.
- **Full PropertyList / URL-part accessors** — `req.getHost/getPath/getQueryString`, `headerList.one/count/find/filter/each/map/reduce/toObject/populate/assimilate`, request-cookie PropertyList — need computed shims or throw `TypeError: not a function`.
- **`bru.interpolate(str)`** — needs a real shim over the variable scopes.
- **inbuilt libs not injected by `rq`** — `moment`, `uuid`, `nanoid`, `jsonwebtoken`, `ajv`, `cheerio` — only reachable if the host mirrors Bruno's allowlist.

Hard gaps should emit **hard diagnostics at transform time**, not silently produce broken scripts.

---

## 4. Verdict & architecture

Rough split of a typical Bruno script: **~70% clean rename/reshape · ~20% semantic (translatable) · ~10% hard gap.**

An AST-rewrite (like the `pm→rq` root swap, plus a reshape table) is the right architecture and
reuses the existing OXC / scope-stack / byte-offset-replacement engine — but Bruno needs:

1. **A richer replacement vocabulary** vs the pm identifier swap:
   - member/verb renames (variable scopes, `delete`→`unset`) — plain node rewrites;
   - **method↔property flips** (`req.getUrl()`→`rq.request.url`, `bru.isSafeMode()`→`rq.isSafeMode`) — rewrite a `CallExpression` into a `MemberExpression`, and setters into an `AssignmentExpression` (drop the `()`), which the pm phase never had to do;
   - **bare-global → member** (`test`/`expect`/`assert`) — scope-stack-gated (skip user-shadowed identifiers), then prefix `rq.`.
2. **A companion runtime shim** injected as `__bruCompat.*` on top of `rq.*` for the awkward long
   tail (`bru.interpolate`, `sleep`, jar `hasCookie`/`setCookies`, `getFolderVar`/`getRequestVar`,
   the full header/cookie PropertyList, `req.getHost/getPath/getQueryString`, Chai `assert`) —
   rewrite those to `__bruCompat.*` rather than inline. Hybrid: AST-rewrite the ~90% that maps to
   `rq.*` members; route the rest through the shim.
3. **A Chai plugin** for Bruno's `jsonBody()`/`jsonSchema()` chain extensions (safer than rewriting).
4. **A post-rewrite validation pass** asserting every emitted `rq.*` / `__bruCompat.*` target
   actually exists — exactly as the pm phase already does. `assert.*` must be shimmed (never
   noop'd); Node/fs/process/arbitrary-`require`/`res.setBody` should hard-error at transform time.

**Declarative note:** `.bru` `vars:pre-request` / `vars:post-response` and `assert` blocks are
declarative data, not scripts — cross-q already models them as first-class IR (`RequestBehavior`,
`Assertion`). They are out of scope for this script-translation layer.
