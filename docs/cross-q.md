# `cross-q` — a reliable cross-converter for API-client collections

> You have a Postman collection. Your teammate lives in Bruno. CI reads OpenAPI. The new
> hire wants it in `rq`. `cross-q` is the one binary that moves a collection between any
> two of them — **without losing what it can't fully translate, and without ever lying
> about what it dropped.**

`cross-q` (binary: **`cq`**) is a single Rust binary that converts API-client
collections between formats — Postman, Requestly/`rq`, Insomnia, Bruno, HAR, OpenAPI,
cURL, and more. It is the sibling project to [`rq`](../README.md) (the Requestly CLI) and
[`cm`/cyberium](https://github.com/browserstack/cyberium) (the test-machine allocator),
and it exists to make one thing true: **importing a collection into Requestly should
never fail silently, and never fail on valid input.**

That mandate comes straight from **RQ-4300 — Importer Reliability and Schema-Driven
Rewrite** (RCA: **RQ-3458**, "Postman import rejects valid collections — null/numeric
key-value keys"). `cross-q` is the schema-driven engine that epic asks for, built as a
standalone converter first so the mapping logic can be tested to death before it ships
inside the product.

---

## How to read this document

- Want to **use it**? → §1 (the loop) and §7 (CLI surface).
- Want to know **what converts to what**? → §2 (support matrix).
- Want the **architecture** (why it's built this way)? → §4 (hub-and-spoke) and §5
  (the three-phase pipeline).
- Care about **reliability** (the whole point)? → §6.
- Designing the **canonical model**? → that's its own document:
  [`IDEALISED.md`](./IDEALISED.md).
- Emitting **Requestly** specifically? → [`FORMAT.md`](./FORMAT.md).

---

## 1. The loop — 30 seconds

```bash
# Convert a Postman collection into an rq (Requestly) project.
$ cq convert team.postman_collection.json --to rq -o ./apis

  ✓  parsed   postman v2.1        3 folders · 41 requests · 2 saved responses
  ✓  mapped   → idealised model   41/41 requests · 5 auth blocks · 118 variables
  ⚠  coerced  3 issues (see report)
  ✓  emitted  rq LOCAL_FS 1.12.0  ./apis/  (git-ready)

  report: ./apis/.cross-q/report.json   ·   cq report ./apis to read it
```

That's the whole product: **read one format, map it through an idealised model, write
another format — and hand you a report of everything that wasn't a clean 1:1.**

```bash
$ cq convert api.har         --to openapi -o spec.yaml     # HAR capture → OpenAPI spec
$ cq convert insomnia.json   --to bruno   -o ./bruno       # Insomnia → Bruno
$ cq convert './apis'        --to postman -o out.json       # rq project → Postman
$ cq curl 'curl https://...' --to rq       -o ./apis        # a curl → a named request
$ cq inspect team.json                                      # detect format, print a summary
$ cq report ./apis                                          # re-read the last conversion report
```

No GUI. No cloud. No account. A collection in, a collection out, a report on the side.

---

## 2. Support matrix (be honest about what's real)

`cross-q` is **hub-and-spoke**: every format is either an **importer** (→ idealised
model) or an **exporter** (idealised model →), or both. Adding a format is one mapper, not
N×M glue.

| Format | Import | Export | Notes |
|---|:--:|:--:|---|
| **Requestly / `rq`** (`LOCAL_FS` + export envelope) | ✅ | ✅ | Native. See [`FORMAT.md`](./FORMAT.md). |
| **Postman** Collection v2.0 / v2.1 (+ Environment, Globals) | ✅ | ✅ | The primary migration source (RQ-3458). |
| **OpenAPI** 3.0 / 3.1 + **Swagger** 2.0 (JSON/YAML) | ✅ | ✅ | Circular-ref safe (RQ-1978). |
| **HAR** 1.2 | ✅ | — | Capture → requests; null-byte safe. |
| **cURL** | ✅ | ✅ | Single command ↔ single request. |
| **Insomnia** v4/v5 | 🚧 planned | 🚧 planned | Not yet mapped — do not assume. |
| **Bruno** (`.bru`) | 🚧 planned | 🚧 planned | Requestly's importer is a stub today; we own it here. |
| **WSDL** (SOAP 1.1/1.2) · **SoapUI** | ✅ | — | Import only. |
| **dotenv** | ✅ | — | → environment variables. |
| **`.http`** (VSCode REST Client / JetBrains) · **Hurl** | 🚧 planned | 🚧 planned | Text-first neighbors. |

**Legend:** ✅ shipping · 🚧 planned (mapper not written — the CLI will tell you, not
guess). We follow the house rule from cyberium: *lead with honesty.* If a mapper isn't
done, `cq` refuses with `not_implemented`, it does not half-convert.

---

## 3. Why this exists (the RQ-4300 story)

The importer that ships inside Requestly today has three habits that RQ-4300 is chartered
to end:

1. **It rejects valid input.** RQ-3458: a 28MB Postman collection aborted because one
   key-value pair had `key: null` — legal in Postman, fatal to a strict `z.string()`.
2. **It fails without saying where.** A parse error deep in mapping surfaces as a generic
   "import failed," with no indication of which request, which phase, which field.
3. **It loses data silently.** A body mode with no Requestly equivalent, an auth type off
   the end of the enum — gone, no warning.

`cross-q` is the "schema-driven rewrite" half of that epic (RQ-4301), extracted into a
tool you can point at ten thousand real collections and measure. The reliability work
(RQ-4512 exact schemas, RQ-4513 validation-bypass fix, RQ-4589 OpenAPI XML bodies,
RQ-4591 per-stage error propagation / ADR-191, RQ-4302 error context across all three
phases, RQ-4690 the Layer-0 mapper contract + orchestrator) all lands here first, in a
place with no UI to hide behind.

---

## 4. Architecture — hub-and-spoke, not point-to-point

The mistake is to write a Postman→Requestly converter, then an Insomnia→Requestly
converter, then a Postman→Bruno converter… that's N×M converters, each with its own bugs.

`cross-q` writes **N importers + M exporters** around one canonical model — the
**Idealised Model** (the IR), specified in [`IDEALISED.md`](./IDEALISED.md):

```
  Postman ─┐                                   ┌─ Requestly / rq
  Insomnia ─┤                                   ├─ Postman
  Bruno    ─┤                                   ├─ Insomnia
  HAR      ─┼──►  parse ─► IDEALISED MODEL ─► emit  ─┼─ Bruno
  OpenAPI  ─┤          (the superset IR)           ├─ OpenAPI
  cURL     ─┘                                   └─ cURL
```

- **The IR is the superset**, not the intersection. It can hold anything any tool in the
  category can express — `rq`'s render templates and `parents:` DAG, MQTT/WebSocket
  configs, every auth type, dynamic and vault variables. A converter that mapped through
  the *intersection* would erase everything interesting; `cross-q` maps through the
  *union* and records what the target can't represent.
- **The IR schema is the contract.** It is checked into `crates/cq-model`, versioned, and
  every importer/exporter is written against it — the same discipline `rq`'s `policy.md`
  sibling calls "the schema is software."

---

## 5. The three-phase pipeline

Every conversion is exactly three phases, each with typed inputs, typed outputs, and its
own error channel (RQ-4302's "error context across all three import phases", RQ-4591's
per-stage propagation, ADR-191):

```
   PARSE                MAP                  EMIT
   ─────                ───                  ────
   bytes ─► source AST   source AST ─► IR      IR ─► target bytes
   (format-specific,     (the mapper           (format-specific,
    tolerant reader)      contract, RQ-4690)    strict writer)

   errors: SyntaxError   errors: MappingError  errors: EmitError
           + byte span           + IR path             + record id
```

- **PARSE** is *tolerant* (Postel's law, inbound): read the messiest real-world file the
  format allows. Coerce `key: 42` to `"42"`, drop nameless cookies, strip null bytes —
  and log each coercion. Abort **only** on genuinely ambiguous input (an object where a
  key must be a scalar), never on merely ugly input.
- **MAP** is the **mapper contract** (RQ-4690, "Layer 0"): a source AST in, an IR out,
  plus a list of `Provenance` and `Diagnostic` records. This is where a Postman
  `edgegrid` auth becomes IR `inherit` + a warning. The orchestrator runs mappers; the
  mappers never touch bytes.
- **EMIT** is *strict* (Postel's law, outbound): produce output that the target's own
  validator will accept on the first try. If the IR holds something the target can't
  represent (an MQTT request → Postman), the emitter **drops it loudly** — a diagnostic,
  never a silent omission.

Each phase can fail independently and says exactly where: which byte, which IR path,
which record id. "Import failed" is not an acceptable error message anywhere in
`cross-q`.

---

## 6. Reliability — the thing we actually sell

Three guarantees, each testable:

**1. Never fail on valid input.** Every format's real-world quirks get a fixture. The
RQ-3458 null-key case, numeric keys, empty keys, malformed percent-encoding, circular
`$ref`s, HAR null bytes — all are inputs `cross-q` *handles*, not inputs it dies on. New
crash-on-valid bugs become regression fixtures, permanently.

> **Coercion, not rejection.** `key: null → ""`, `key: 42 → "42"`, `value: [a,b] → "a b"`.
> But `key: {…}` still hard-fails — an object where a scalar belongs is genuinely
> ambiguous, and guessing would be the *worse* failure. We coerce the unambiguous and
> refuse the ambiguous. (Deliberately *not* a blanket `.catch("")` — that hides real
> corruption.)

**2. Never lose data silently.** Every conversion emits a **report** — a structured
JSON + human summary of what mapped cleanly (`ok`), what was changed to fit (`coerced`),
and what the target couldn't hold (`dropped`) — each with provenance back to the source
location. This is the same instinct as cyberium's append-only audit log: *if the tool
made a decision, the decision is on the record.*

**3. Always say where.** Errors and diagnostics carry a phase, a source span (parse), an
IR path (map), or a record id (emit). A failure you can't locate is a bug in `cross-q`,
not in your collection.

### Fidelity levels

Each `format → format` pair is rated so you know before you run:

| Level | Meaning |
|---|---|
| **round-trip** | A → B → A returns byte-equivalent structure. Nothing lost. |
| **lossless** | Everything in A is representable in B (B may add defaults). |
| **lossy (reported)** | Some A features have no B equivalent; each is in the report. |
| **degraded** | Structural downshift (e.g. gRPC → cURL); heavily reported. |

`cross-q` never silently upgrades a claim — a lossy conversion is *called* lossy, up
front, in the summary line.

---

## 7. CLI surface

```bash
cq convert <input> --to <format> [-o <output>]   # the main verb
cq inspect <input>                               # detect format + summarize, no output written
cq report  <path>                                # print the report from the last conversion
cq curl    '<curl-cmd>' --to <format> -o <path>  # a single curl → a request
cq formats                                       # list supported formats + fidelity matrix
cq validate <input> --as <format>               # does this file parse cleanly? (CI gate)
```

Flags that matter:
- `--to <format>` — target (`rq`, `postman`, `openapi`, `bruno`, `insomnia`, `har`,
  `curl`, …). Source format is auto-detected; override with `--from`.
- `-o, --output <path>` — file or directory (directory for tree formats like `rq`).
- `--report <path>` — where to write the machine-readable report (default:
  `<output>/.cross-q/report.json`).
- `--strict` — treat any `coerced` or `dropped` diagnostic as a non-zero exit (for CI).
- `--dry-run` — parse + map, print the report, write nothing.

Exit codes are meaningful: `0` clean, `2` completed with diagnostics, `3`
`not_implemented` (a mapper isn't written yet), `4` unrecoverable parse error.

---

## 8. Layout

```
cross-q/
├── README.md            # this file — product + architecture
├── IDEALISED.md         # the canonical Idealised Model (the IR spec)
├── Cargo.toml           # single workspace, single binary `cq`
└── crates/
    ├── cq-model/         # the Idealised Model — types + schema + versioning
    ├── cq-parse/         # PARSE: format-specific tolerant readers
    ├── cq-map/           # MAP: the mapper contract + orchestrator (RQ-4690)
    ├── cq-emit/          # EMIT: format-specific strict writers
    ├── cq-report/        # diagnostics, provenance, fidelity reporting
    └── cq-cli/           # the `cq` binary
```

One binary, one workspace — the same reason cyberium ships three modes in one `cm`:
shared model, shared schemas, one distribution, no releases to keep in lockstep.

---

## 9. What `cross-q` is NOT

- **Not a runner.** `cross-q` converts collections; it does not *execute* requests.
  Running is `rq`'s job (`rq r <name>`) and Newman's.
- **Not a sync engine or a cloud.** No accounts, no server, no proprietary store. Files
  in, files out. If `cross-q` vanished, your collections are still plain files.
- **Not a Requestly-only tool.** Requestly is the primary target because of RQ-4300, but
  the IR and the CLI are format-neutral — `postman → bruno` never touches Requestly.
- **Not the format spec for any single tool.** For Requestly's bytes, read
  [`FORMAT.md`](./FORMAT.md). For the canonical model, read
  [`IDEALISED.md`](./IDEALISED.md).

---

## The one-sentence version

`cross-q` converts an API-client collection from any supported format to any other by
mapping it through one idealised superset model, parsing tolerantly and emitting
strictly, and it never fails on valid input, never loses data without reporting it, and
never fails without telling you where.
