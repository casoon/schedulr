# schedulr

Rust crate: scheduling framework (activities, resources, participants, calendars, rules and goals)
built on the [`unifier`](https://github.com/casoon/unifier) constraint solver. Published on
crates.io as `schedulr`; docs at https://casoon.github.io/schedulr/. Local status and plans:
`plan/` (gitignored).

## Fixed rules

- Language: **English** for everything in the repository — code, identifiers, comments, doc
  comments, string literals, tests, docs and commit messages. Only `plan/` (local, gitignored)
  may be German.
- User-facing changes get an entry under `## [Unreleased]` in `CHANGELOG.md` in the same commit.
- Before a push: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
