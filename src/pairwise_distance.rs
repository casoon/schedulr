//! Minimum distance between all starts of a holder as **one** constraint — the soft form of
//! `MinimumBreak` (timbra plan/69, step 0).
//!
//! The hard form posts one `MinimumDistance` per pair. As a preference that would be a separate
//! objective per pair, and objectives are scanned on every move: n(n-1)/2 of them for n
//! appointments. Here one constraint counts all pairs that are too close.

use std::collections::HashMap;
use unifier::VariableId;
use unifier::constraint::{Constraint, PropagationResult};
use unifier::model::domain::TrailedDomains;

#[derive(Debug, Clone)]
pub(crate) struct PairwiseDistance {
    starts: Vec<VariableId>,
    min_distance: i64,
    scope: Vec<VariableId>,
}

impl PairwiseDistance {
    pub(crate) fn new(starts: Vec<VariableId>, min_distance: i64) -> Self {
        let mut scope = starts.clone();
        scope.sort_unstable();
        scope.dedup();
        Self {
            starts,
            min_distance,
            scope,
        }
    }

    /// Pairs of assigned starts with `|a - b| < min_distance`. Sorted, then two pointers:
    /// O(n log n) instead of O(n²).
    fn close_pairs(&self, assignment: &HashMap<VariableId, i64>) -> u32 {
        let mut values: Vec<i64> = self
            .starts
            .iter()
            .filter_map(|start| assignment.get(start).copied())
            .collect();
        values.sort_unstable();
        let mut pairs = 0u32;
        let mut upper = 0;
        for (index, &value) in values.iter().enumerate() {
            upper = upper.max(index + 1);
            while upper < values.len() && values[upper].saturating_sub(value) < self.min_distance {
                upper += 1;
            }
            pairs = pairs.saturating_add(u32::try_from(upper - index - 1).unwrap_or(u32::MAX));
        }
        pairs
    }
}

impl Constraint for PairwiseDistance {
    fn name(&self) -> &str {
        "MinimumBreak"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn is_satisfied(&self, assignment: &HashMap<VariableId, i64>) -> bool {
        self.close_pairs(assignment) == 0
    }

    /// Every pair that is too close counts: moving one appointment away is visible to the search.
    fn violations(&self, assignment: &HashMap<VariableId, i64>) -> u32 {
        self.close_pairs(assignment)
    }

    /// Only used as a preference; the hard form filters through the pairwise constraints.
    fn propagate(&self, _domains: &mut TrailedDomains) -> PropagationResult {
        PropagationResult::Success { changed: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_every_pair_closer_than_the_distance() {
        let vars = [VariableId(0), VariableId(1), VariableId(2), VariableId(3)];
        let rule = PairwiseDistance::new(vars.to_vec(), 2);
        // Some pairs of 0,1,2 are too close: (0,1), (1,2); 0 and 2 are 2 apart — enough.
        let assignment = HashMap::from([(vars[0], 0), (vars[1], 1), (vars[2], 2), (vars[3], 10)]);
        assert_eq!(rule.violations(&assignment), 2);

        let same_slot = HashMap::from([(vars[0], 5), (vars[1], 5), (vars[2], 5)]);
        assert_eq!(rule.violations(&same_slot), 3);

        let apart = HashMap::from([(vars[0], 0), (vars[1], 2), (vars[2], 4)]);
        assert!(rule.is_satisfied(&apart));
    }
}
