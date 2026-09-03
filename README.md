# schedulr

Scheduling framework (activity/resource/interval DSL) for Rust, built
on [`unifier`](https://github.com/casoon/unifier)'s CSP/COP constraint
model and solvers, which in turn build on
[`pathwise`](https://github.com/casoon/pathwise)'s generic search and
optimization traits.

```
pathwise → unifier → schedulr → (application: timetabling, appointment booking, ...)
```

## Status

Concept phase — nothing implemented yet. See `plan/01-concept.md`
(local, untracked) for the relationship to `unifier` and open scope
questions.

## Problem class

Scheduling / timetabling / appointment booking on top of `unifier`'s
CSP/COP model: activities placed against resources over time, subject
to hard constraints (no double-booking, capacity, precedence,
calendar/opening-hours exclusions) and soft preferences (e.g.
proximity to a requested time slot).

## Installation

Not published to crates.io yet.

## License

MIT — see [LICENSE](LICENSE).
