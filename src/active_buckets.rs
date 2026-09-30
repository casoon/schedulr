//! Wie viele Buckets ein Träger überhaupt belegt — „an höchstens vier Tagen“ (timbra plan/62, G6).
//!
//! Anders als eine Tageslast fragt das nicht, *wie viel* in einem Bucket liegt, sondern *ob*
//! etwas darin liegt. Welche Buckets frei bleiben, entscheidet die Suche.

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

    /// Die Buckets, in denen eine bereits entschiedene, anwesende Aufgabe liegt. Monoton: mehr
    /// Belegtes kann nur mehr Buckets ergeben — ein Zuviel auf einer Teilbelegung bleibt eins.
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

    /// Wie viele Buckets zu viel belegt sind: wer einen Tag räumt, sieht die Suche.
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

    /// Sind so viele Buckets sicher belegt, wie erlaubt, dürfen die übrigen sicher anwesenden
    /// Aufgaben nur noch dort liegen — Starts, deren Intervall einen anderen Bucket berührt,
    /// fallen weg. Sicher belegt heißt: Aufgabe sicher anwesend und ihr Start entschieden. Mehr
    /// sicher belegte Buckets als erlaubt ist ein Konflikt.
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
