---
title: Relations and scores
description: Hard relations tie two activities together in time. Score rules express preferences on three levels, and every solution reports how each rule contributed.
order: 3
---

## Activity relations


An `ActivityRelationConstraint` connects two activities with an `ActivityRelation`. It is a hard
constraint: the solver never returns a solution that violates it.

| Relation | Meaning |
| --- | --- |
| `SameStart` | Both activities start at the same time. |
| `Consecutive` | The second starts exactly when the first ends. |
| `Precedence { min_gap }` | The first ends at least `min_gap` units before the second starts. |
| `NoOverlap` | The two activities do not overlap in time. |

```rust
use schedulr::{ActivityId, ActivityRelation, ActivityRelationConstraint};

let problem = problem
    .with_relation(ActivityRelationConstraint::new(
        ActivityId(2),
        ActivityId(3),
        ActivityRelation::Consecutive,
    ))
    .with_relation(ActivityRelationConstraint::new(
        ActivityId(1),
        ActivityId(2),
        ActivityRelation::Precedence { min_gap: 1 },
    ));
```

`NoOverlap` is useful for activities that share no resource or participant, for example a
welcome session and an office hour that everyone should be able to attend. Activities that share
a resource with capacity 1 or a participant never overlap anyway.

Relations must reference known activities and cannot relate an activity to itself; both are
compile errors.

## Score rules

Preferences are `ScoreRule`s. Each rule has a category (a name you choose), a level, the activity
it applies to, a positive weight, and a kind. `ScoreRule::prefer_window` is the public
constructor:

```rust
use schedulr::{ActivityId, ScoreLevel, ScoreRule, TimeWindow};

let problem = problem.with_score_rule(ScoreRule::prefer_window(
    "kickoff on Monday at 09:00",
    ScoreLevel::Strong,
    ActivityId(1),
    TimeWindow::new(9, 10),
    1,
));
```

A `PreferWindow` rule costs `-weight` when the activity starts outside the preferred window and
0 when it starts inside. The second kind, `KeepStart`, is used by
[repair](../analysis-and-repair/) to keep activities where they were.

## Score levels

`ScoreLevel` has three tiers: `Strong`, `Medium` and `Weak`. They are compared
lexicographically: any improvement on a higher level beats every possible improvement on a lower
one, no matter the weights. Weights only compare rules on the same level.

When a problem has score rules, `solve()` runs branch and bound and optimises the score; without
rules it runs plain backtracking and returns the first feasible plan.
`SolveStatistics::optimal` tells you whether the search proved the result optimal.

## Reading the score

Every `Solution` carries:

- `score`: a `Score` with the fields `hard`, `strong`, `medium`, `weak` and `soft`;
- `score_components`: one `ScoreComponent` per rule with its category, level, activity and
  value.

The components let an application show why a plan looks the way it does, and store the scores
with each schedule version. From the [workshop plan](../../../showcase/workshop-plan/):

```text
Score: hard 0, strong 0, medium 0, weak 0
  Strong  kickoff on Monday at 09:00  a1  0
  Medium  Q&A on Tuesday afternoon    a5  0
  Weak    lab on Tuesday morning      a3  0
```

When several plans reach the same best score, the one `solve()` returns depends on the search
order and can differ between runs. Add rules if you need to prefer one of them.
