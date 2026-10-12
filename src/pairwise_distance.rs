//! Mindestabstand zwischen allen Starts eines Trägers als **eine** Bedingung — die weiche Form
//! von `MinimumBreak` (timbra plan/69, Schritt 0).
//!
//! Die harte Form legt je Paar ein `MinimumDistance` an. Als Wunsch wäre das je Paar ein eigenes
//! Ziel, und Ziele werden bei jedem Zug durchsucht: bei n Terminen n(n-1)/2 Stück. Hier zählt eine
//! Bedingung alle Paare, die zu nah liegen.

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

    /// Paare zugewiesener Starts mit `|a - b| < min_distance`. Sortiert, dann zwei Zeiger:
    /// O(n log n) statt O(n²).
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

    /// Jedes zu nahe Paar zählt: wer einen Termin wegschiebt, sieht die Suche.
    fn violations(&self, assignment: &HashMap<VariableId, i64>) -> u32 {
        self.close_pairs(assignment)
    }

    /// Wird nur als Wunsch eingesetzt; die harte Form filtert über die Paar-Bedingungen.
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
        // 0,1,2 liegen paarweise teils zu nah: (0,1), (1,2); 0 und 2 haben Abstand 2 — genug.
        let assignment = HashMap::from([(vars[0], 0), (vars[1], 1), (vars[2], 2), (vars[3], 10)]);
        assert_eq!(rule.violations(&assignment), 2);

        let same_slot = HashMap::from([(vars[0], 5), (vars[1], 5), (vars[2], 5)]);
        assert_eq!(rule.violations(&same_slot), 3);

        let apart = HashMap::from([(vars[0], 0), (vars[1], 2), (vars[2], 4)]);
        assert!(rule.is_satisfied(&apart));
    }
}
