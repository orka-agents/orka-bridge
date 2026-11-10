# Native Run: small correctness boundaries

Issue [#74](https://github.com/orka-agents/orka-bridge/issues/74) extends the
working native Run path with three independently testable corrections. Start
with the existing parser, runner and transport; grow their contracts only where
the observed edge cases require it. This follows Gall's law: evolve the small
working flow rather than introduce another execution framework.

## Separation of concerns

| Boundary | Responsibility |
|---|---|
| `engine/run_source.rs` | Decode immutable saved bytes, validate the supported source schema, classify builtin names, extract and validate all test cases before preparation. No I/O. |
| `engine/run.rs` | Prepare resource bodies, submit once, retain acknowledged identities and uncertain writes, observe only acknowledged resources, aggregate evidence. |
| `RunTransport` implementations | Native-owned endpoint, namespace, credentials, request bounds and connection lifetime. Source files cannot select transport authority. |
| Existing app owners and renderer | Preserve the run receipt and present the report. An evidence verdict is separate from submission/observation state. |

## Test fences: explicit support, no silent omission

Native Run accepts a top-level opening line exactly <code>```yaml agent-test</code>.
Each block decodes as one case using the existing closed YAML schema. Closing
fences use backticks of at least the opening length, with optional surrounding
whitespace. A missing test closing fence refuses the source.

Indented, tilde, longer-opening and other noncanonical `agent-test` fences are
rejected before any Agent or Task is submitted, including when a valid case
appears elsewhere in the file. This is intentionally a small supported source
format, not a second general Markdown implementation. Authors can move the test
to the canonical fence without changing its YAML.

Ordinary Markdown code examples are tracked by delimiter character and opening
length. An apparent test inside an example is inert; a shorter or different
delimiter does not close that example. Fence recognition and case YAML decoding
are separate helpers so Markdown delimiters cannot become execution policy.
Unclosed example fences also refuse the source rather than hiding later tests.

## Builtin names: classification before mutation

`kind: builtin` must name an exact known builtin, not merely a nonempty string.
Unknown, differently cased or padded names are source errors, so they cannot
fall through to an otherwise authorized custom Tool when an Agent is created.

The explicit native catalogue follows `KnownBuiltInToolNames` (default registry,
chat and coordination names) at Orka source
[`7916cd1c`](https://github.com/orka-agents/orka/blob/7916cd1c8c083ade6b10df6ba408f854d930190c/internal/tools/registry.go),
with coordination names in
[`internal/aitools/resolver.go`](https://github.com/orka-agents/orka/blob/7916cd1c8c083ade6b10df6ba408f854d930190c/internal/aitools/resolver.go).
Review this catalogue when changing supported Orka versions. New builtins remain
unsupported until reviewed; custom and MCP tools need their own source mapping.

This check classifies source intent, not target availability or authorization.
Orka and the existing connection/permission checks still decide what the target
can execute. No new tool-discovery request, guest protocol operation or
credential path is needed for this correction.

## Partial runs: failure is evidence, missing is unknown

Submission may acknowledge an Agent and only some of its Tasks. Observation
reads those receipts only; an uncertain write is never retried or looked up by
an unacknowledged name. The report includes each missing case as
`Unacknowledged`, with no answer/tool evidence and unknown assertions/verdict.
That phase does not assert that the remote Task is absent or cancelled.

Use the existing aggregation rule for the complete case list:

- Any known failed case makes the run verdict failed.
- Otherwise, any unknown case makes the run verdict unknown.
- Passed requires every case to be passed.

An incomplete submission therefore cannot erase an observed failure or become
a pass. Transport errors and Stop retain their existing handling; this does
not add automatic observation recovery or remote cancellation.

## Verification

`app/src-tauri/tests/native_run.rs` covers noncanonical fences beside valid
cases, nested Markdown examples, exact builtin names, and a real loopback HTTP
submission that loses the second Task reply. The latter observes the retained
first Task across failed, cancelled, assertion-failed, passed and missing-answer
outcomes, and checks that observation performs only reads of acknowledged
resources. These are native runner tests, not evidence of a live model/VM run.
`tests/example_run.rs` additionally exercises the saved-file app owner through
the VM protocol fixture: an unknown builtin refuses before any remote write and
leaves the client usable.

### Local verification record

Verified on Apple silicon with Node 26.5.0, npm 11.17.0 and Rust 1.98.1:

- Frontend install, lint, all 179 tests and production build passed.
- All release integration-test targets passed, including 26 native Run tests,
  11 example Run tests, source capture and cluster transport tests.
- Debug engine, native Run, example Run and image-profile tests passed.
- Rust formatting, debug/release all-target clippy and release doc-tests passed.
- The production frontend dependency audit found no vulnerabilities. The full
  audit reported the existing moderate development-only minimatch dependency chain.

The full release test command stopped in the library suite: 192 passed, three
failed and one was ignored. The unchanged base `ce154b4` reproduced all three
failures with the same errors: the Apple identifier helper's 100 ms timeout,
credential capture returning `Failed`, and the nonzero credential fixture not
executing. Their cause is not established by this work; the successful
integration suites do not replace that failed library result.

No live-model/VM or native UI walk was performed for these edge cases. Report
presentation continues through the existing renderer; the new missing-case
phase is verified at the native report boundary.
