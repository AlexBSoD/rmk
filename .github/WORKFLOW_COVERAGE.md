# RMK workflow coverage gates

`build.yml` delegates firmware selection to `ci_policy.py plan --profile firmware`;
`ci.yml` delegates the complete general test profile to `--profile tests`.
The shared policy helper and its tests are separately owned: these workflow
changes require both to be installed together before publication.

The policy contract is conservative: documentation-only changes do not build
firmware or run the expensive test graph; test-only changes still run the entire
general test profile but do not build production firmware. Runtime, configuration,
build-policy and unknown changes retain the full firmware profile. Direct pushes
without validated evidence remain full. Push reuse requires successful original
same-workflow PR proof for the identical Git tree, not just equal commit messages,
ancestry or a previous reused result. Dispatch/release behavior remains full;
release and user-build workflow owners are unchanged. Automatic firmware pushes
are intentionally restricted to `main`; feature-branch updates are checked by
`pull_request`, avoiding a second push matrix. No tag triggers are added.

All original jobs, runners, matrices, features and test commands remain intact.
Only policy dependencies/conditions are added. In particular `.github/ci/test.sh`
continues to own its complete native, feature and macro tests; there is no second
local feature matrix. Concurrency is scoped by workflow, event and PR/ref so a
push does not cancel a PR's only validation wave.

Each workflow has an `always()` completion job depending on every original owner.
`workflow_complete.py` rejects failed/cancelled planners, missing decisions,
missing dependencies, and any missing/skipped full-profile owner. Its only full
profile exclusions mirror the original actual graph conditions:

- `bloat-report` is skipped outside `pull_request`.
- `build-esp` is skipped only when successful discovery emits an empty array or
  an array whose first item is null, matching `fromJson(esp)[0] != null`.
- All stable matrix children must succeed through their aggregate job result.

The summary distinguishes full coverage from policy skip/reuse. Only a verified
full graph can execute `record` and upload
`ci-proof-PROFILE-RUN_ID-RUN_ATTEMPT`; docs/reuse results cannot propagate new proof.
Original job/check names and event-specific bloat reporting are preserved. Branch
protection is not changed here; the integrator must reconcile required contexts
before landing.

## Focused verification

Install `PyYAML==6.0.3`, then run:

```sh
python3 -m unittest discover -s .github/scripts -p test_ci_policy.py
python3 -m unittest discover -s .github/scripts -p 'test_workflow_*.py'
actionlint -shellcheck= -pyflakes= .github/workflows/build.yml .github/workflows/ci.yml
git diff --check
```

Planner jobs execute both Python suites before making a selection. Workflow tests
freeze the pre-policy original owner payloads from `89a6f853`, including all build
and test steps, matrices and runners; deliberate future changes must review those
baselines. Completion tests cover each required owner failing/cancelling/skipping,
PR/push/manual graphs, ESP present/empty/null/invalid output, planner failure,
policy skip without proof, and the actual CLI output/summary path.

These are local graph/selection-boundary checks, not hosted Actions execution,
firmware compilation or hardware verification. Same-tree evidence lookup and
path classification must additionally pass the shared helper's own tests.
