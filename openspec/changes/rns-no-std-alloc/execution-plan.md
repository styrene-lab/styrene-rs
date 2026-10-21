# Supervised implementation and acceptance

## Authority and roles

The user authorized supervised local implementation after establishing this plan.
The approved initial target is `aarch64-unknown-none`. Cortex-M support is excluded.

| Role | Responsibility |
|---|---|
| Coordinator | This supervising session: resolve contracts, dispatch bounded agents, review handoffs, integrate commits, maintain the corpus, and report progress |
| Implementation agents | Make and validate changes only within assigned paths in isolated worktrees |
| Independent reviewer | Review the integrated revision and security/compatibility evidence without authoring the changes under review |
| Final acceptor | Wilson: approve any scope change, external delivery, merge, and final acceptance |

Local source edits, isolated tests, focused commits, and local integration are
authorized. Do not modify existing developer or validation checkouts.

The following remain approval-gated: host/toolchain provisioning, pushes,
opening a GitHub PR, merging, issue closure, OpenSpec archival, and release.
The repository uses GitHub PRs, not GitLab MRs. A green local result does not
authorize any of these external or lifecycle actions.

## Shared baseline and decision gates

Commit this corpus before dispatching implementation worktrees. Record that
planning commit as the common lane base. The product base remains
`ab46aa1f13de5663c684225dc27dfcba330d965f`.

- D1 is resolved: atomic-capable AArch64 allocation-backed core only. Record the
  compiler target configuration with the target evidence.
- D2 must specify the executable target harness and runner before that harness
  is implemented. Missing environment prerequisites block execution, not unrelated
  local dependency or core work.
- D3 must specify RNG entry points, state ownership, and failure semantics before
  core implementation or consumer API assertions begin.

The coordinator records decisions in `design.md`. Agents must not independently
invent incompatible APIs or widen the supported platform envelope.

## Work packages and exclusive write ownership

Each lane gets its own branch and worktree from the planning commit. Agents may
read other paths. They must request a handoff before writing outside their scope.

| Lane | Owned paths | Cases | Inputs and output |
|---|---|---|---|
| A — Dependency boundary | `crates/libs/styrene-rns/Cargo.toml`, product `Cargo.lock` only if resolution requires it | 001, 002, 008, 010 | Approved feature design; return dependency changes, resolved graph evidence, and exact commit |
| B — Core and security tests | Non-transport files under `crates/libs/styrene-rns/src/`; `crates/libs/styrene-rns/tests/` | 004–009 | Approved D3; return RNG plumbing, API documentation, focused regression tests, and exact commit |
| C — Consumer and validation gates | Proposed `tests/compile/rns-no-std/`, its fixture lockfile, `scripts/check-rns-no-std*`, `.github/workflows/ci.yml`, `justfile` | 001–003, 010–012 | Approved D1/D2 and D3 API contract; return isolated consumer, target/harness gates, command documentation, and exact commit |
| Coordinator | This OpenSpec change and final support documentation; integration branch | All | Reviewed lane commits and evidence; return integrated revision and acceptance summary |
| Reviewer | No write ownership | All, especially 004–009 | Integrated commit and evidence; return findings with severity and acceptance recommendation |

Lane B owns encryption and decryption together because they share source files.
It must not edit transport code without coordinator reassignment.

Lane A is the sole writer of the product manifest and lockfile. Lane C owns only
the independent fixture's manifest and lockfile. If B needs test dependencies,
it submits the exact requirement to A. Root workspace-manifest changes require
coordinator review before any lane edits them.

Only the coordinator updates shared task checkboxes and acceptance status.
Implementers report coverage but cannot declare final acceptance themselves.

## Dependencies and integration order

1. Commit the corpus and resolve D3 before starting A and B concurrently.
2. Start C after D2 is recorded. C may establish a failing baseline while A and B
   work, but API assertions must use the approved D3 contract.
3. Review lane diffs and integrate A and B as one validation unit. Dependency
   isolation can expose core errors until both changes are present.
4. Integrate C and execute the complete target and hosted matrix on that combined
   revision. A lane's partial success cannot close a combined acceptance case.
5. Obtain independent review. Fix findings in the owning lane or assign a bounded
   integration correction. Revalidate affected cases after every correction.

No agent rebases, resets, cherry-picks, or rewrites another agent's worktree.
The coordinator integrates clean committed handoffs into `fix/rns-no-std-78`.
Do not integrate dirty working-tree copies. Stop on unexplained conflicts and
resolve behavior with the owning agent rather than blindly choosing a side.

Before proposing a PR, recheck remote `main`. If it changed, review the overlap
and validate the updated integration result before making compatibility claims.
Do not silently absorb unrelated reconciliation or Identity history.

## Required lane handoff

Every agent must return:

- Lane, branch, worktree, base SHA, result SHA, and clean/dirty state.
- Changed paths and any requested ownership exception.
- Case IDs covered and exact commands with exit statuses.
- Evidence directory, toolchain, target, features, and relevant lockfile hashes.
- Known failures, blocked prerequisites, and tests not executed.
- Compatibility/security concerns and dependent lane requirements.

An agent must not mark a case passed merely because its component compiled.
The coordinator binds acceptance to the final integrated source state.

## Evidence custody

Use `/tmp/opencode/` for disposable execution scratch and external Cargo output.
Retain reviewed evidence under
`~/.local/state/styrene-lab/issue-78/runs/<lane-or-integration>/<run-id>/`.
This is a local evidence store, not a product-source checkout or release location.

Before deleting scratch, preserve the required logs, manifests, hashes, and
summaries. Retain acceptance evidence through merge and final acceptance.
Delete it only through an explicit cleanup decision. Do not retain secrets,
operator identities, or live traffic.

Create a concise case-indexed verification summary in this change before PR
preparation. Link exact source revisions and evidence hashes. After publication
is authorized, provide reviewable sanitized summaries and CI artifact links.
A private local path alone is not sufficient evidence for an external reviewer.

## Review and delivery gates

| Gate | Required evidence and owner |
|---|---|
| Local integration | Coordinator verifies clean handoffs, bounded diff, and case coverage on the integrated revision |
| Independent review | Reviewer checks D3/API compatibility, entropy state ownership, zeroization, authentication order, persistence, feature isolation, and test effectiveness |
| PR readiness | Coordinator supplies all required passing cases, applicable repository format/lint/test/policy/interop results, exact revision, limitations, and reviewer disposition |
| PR publication | Wilson authorizes push and PR creation; coordinator targets `main` and includes the compatibility/evidence summary |
| Merge readiness | Final PR-head checks and independent review are accepted; any integration with newer main is validated; no required case is blocked |
| Merge and acceptance | Wilson authorizes merge and acceptance against the reviewed revision; verify the resulting main revision and record the result |
| Issue closure and archival | Separate authorization after acceptance; validate OpenSpec and run `archive --check` before any archive action |

Repository protection does not substitute for these gates. Prior inspection
found no required-review or required-CI rules. Never merge merely because GitHub
permits the operation.

Crate publication is not part of issue acceptance. No release action is implied.
Host, emulator, and hardware evidence retain their separate meanings.

## Progress reporting and stop conditions

Report after contract decisions, lane handoffs, integration checks, independent
review, and any blocker that changes delivery scope or acceptance confidence.
Each update states completed work, current evidence, remaining blockers, and the
next action. Do not report anticipated tests as passed.

Stop the affected lane for an out-of-scope file, unapproved API change, missing
provisioning permission, source mutation during validation, unexplained conflict,
or security/compatibility regression. Other independent work may continue.
