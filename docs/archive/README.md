# Archive

Completed task briefs and the closed work log, kept for the record rather than
for action. Each one is headed `[COMPLETED -- HISTORICAL]` or `[ARCHIVED --
HISTORICAL]` and says where its results landed. They are here rather than in the
repository root because a root full of task specs reads like work in progress,
and none of this is.

| File | Commissioned | Results in |
|---|---|---|
| [`CODEX_TASK_BRIEF.md`](CODEX_TASK_BRIEF.md) | The replay-coverage audit | PROJECT_STATUS.md section 11, NEXT_STEPS_FINDINGS.md |
| [`CODEX_TASK_BRIEF_2.md`](CODEX_TASK_BRIEF_2.md) | The four open "needs work" items | PROJECT_STATUS.md section 14 |
| [`CODEX_TASK_BRIEF_3.md`](CODEX_TASK_BRIEF_3.md) | Whole-block payload preservation | PROJECT_STATUS.md section 7-C |
| [`PROJECT_STATUS.md`](PROJECT_STATUS.md) | Nothing -- it is the work log itself, sections 1-36 | Superseded by [`../../README.md`](../../README.md), [`../DATA.md`](../DATA.md), [`../USAGE.md`](../USAGE.md) |

Brief #3 is worth reading for something other than history: the design
constraints it argues for -- one row per block, never a fabricated per-field
split, an explicit marker no ordinary row can match -- are still the live
contract.

`PROJECT_STATUS.md` is here for the same reason and one more: its section
numbers are cited from README, USAGE and Rust and Python comments as the record
of why a decision was made. The reasoning still holds; the numbers around it
stopped being refreshed at section 36.
