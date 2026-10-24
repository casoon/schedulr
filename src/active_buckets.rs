//! How many buckets a holder occupies at all — "on at most four days" (timbra plan/62, G6).
//!
//! Unlike a daily load, this does not ask *how much* lies in a bucket, but *whether* anything
//! lies in it. Which buckets stay free is left to the search.

use std::collections::{BTreeSet, HashMap};
use unifier::VariableId;
use unifier::constraint::{
    Assignment as UnifierAssignment, BucketRange, BucketedTask, Constraint, Explanation,
    PropagationResult,
};
use unifier::model::domain::{Domain, TrailedDomains};

#[derive(Debug, Clone)]
pub(crate) struct ActiveBucketLimit {
    tasks: Vec<BucketedTask>,
    ranges: Vec<BucketRange>,
    limit: usize,
    scope: Vec<VariableId>,
}

impl ActiveBucketLimit {
    pub(crate) fn new(tasks: Vec<BucketedTask>, ranges: Vec<BucketRange>, limit: usize) -> Self {
        let mut scope: Vec<VariableId> = tasks
            .iter()
            .flat_map(|task| std::iter::once(task.start).chain(task.presence))
            .collect();
        scope.sort_unstable();
        scope.dedup();
        Self {
            tasks,
            ranges,
            limit,
            scope,
        }
    }

    fn buckets_of(&self, start: i64, duration: i64) -> Vec<usize> {
        let end = start.saturating_add(duration.max(1));
        self.ranges
            .iter()
            .filter(|range| range.start < end && start < range.end)
            .map(|range| range.bucket)
            .collect()
    }

    /// The buckets containing an already decided, present task. Monotone: more occupied tasks
    /// can only yield more buckets — an excess on a partial assignment remains one.
    fn active(&self, assignment: &HashMap<VariableId, i64>) -> BTreeSet<usize> {
        let mut buckets = BTreeSet::new();
        for task in &self.tasks {
            let present = task
                .presence
                .is_none_or(|presence| assignment.get(&presence) == Some(&1));
            if !present {
                continue;
            }
            let Some(&start) = assignment.get(&task.start) else {
                continue;
            };
            buckets.extend(self.buckets_of(start, task.duration));
        }
        buckets
    }
}

impl Constraint for ActiveBucketLimit {
    fn name(&self) -> &str {
        "MaximumActiveBuckets"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn is_satisfied(&self, assignment: &HashMap<VariableId, i64>) -> bool {
        self.active(assignment).len() <= self.limit
    }

    /// How many buckets are occupied beyond the limit: clearing a day is visible to the search.
    fn violations(&self, assignment: &HashMap<VariableId, i64>) -> u32 {
        u32::try_from(self.active(assignment).len().saturating_sub(self.limit)).unwrap_or(u32::MAX)
    }

    fn explain(&self, assignment: &UnifierAssignment) -> Option<Explanation> {
        let active = self.active(assignment).len();
        (active > self.limit).then(|| Explanation {
            constraint_name: "MaximumActiveBuckets",
            involved: self.scope.clone(),
            message: format!(
                "active in {active} period buckets, at most {} allowed",
                self.limit
            ),
        })
    }

    /// Once as many buckets are surely occupied as allowed, the remaining surely present tasks may
    /// only lie there — starts whose interval touches another bucket are removed. Surely occupied
    /// means: task surely present and its start decided. More surely occupied buckets than
    /// allowed is a conflict.
    fn propagate(&self, domains: &mut TrailedDomains) -> PropagationResult {
        let surely_present = |task: &BucketedTask, domains: &TrailedDomains| match task.presence {
            None => true,
            Some(presence) => domains
                .get(&presence)
                .is_some_and(|domain| domain.len() == 1 && domain.min() == Some(1)),
        };
        let mut active = BTreeSet::new();
        for task in &self.tasks {
            if !surely_present(task, domains) {
                continue;
            }
            let Some(domain) = domains.get(&task.start) else {
                continue;
            };
            if domain.len() == 1
                && let Some(start) = domain.min()
            {
                active.extend(self.buckets_of(start, task.duration));
            }
        }
        if active.len() > self.limit {
            return PropagationResult::Conflict;
        }
        if active.len() < self.limit {
            return PropagationResult::Success { changed: false };
        }
        let mut changed = false;
        for task in &self.tasks {
            if !surely_present(task, domains) {
                continue;
            }
            let Some(values) = domains.get(&task.start).map(|domain| domain.values()) else {
                continue;
            };
            if values.len() <= 1 {
                continue;
            }
            let outside: Vec<i64> = values
                .into_iter()
                .filter(|&start| {
                    self.buckets_of(start, task.duration)
                        .iter()
                        .any(|bucket| !active.contains(bucket))
                })
                .collect();
            if outside.is_empty() {
                continue;
            }
            let emptied = domains
                .mutate(task.start, |domain| {
                    let mut any = false;
                    for value in &outside {
                        any |= domain.remove(*value);
                    }
                    any
                })
                .unwrap_or(false);
            changed |= emptied;
            if domains.get(&task.start).is_some_and(Domain::is_empty) {
                return PropagationResult::Conflict;
            }
        }
        PropagationResult::Success { changed }
    }
}
