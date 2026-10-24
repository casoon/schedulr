//! Rules about a holder's occupancy **per bucket** — the daily rules from timbra plan 69,
//! package A: minimum load, maximum span, a break between two buckets, a free run inside a
//! window, occupancy of specific bucket sections, distinct groups per bucket — and three
//! objectives (single block per sub-window, shared presence, simultaneously free).
//!
//! All of them work on the concrete bucket windows (`BucketRange`), not on the bucket index:
//! Monday of week A and Monday of week B are two days. A bucket counts as **active** if the
//! holder occupies something in it; most rules only look at active buckets.
//!
//! Rules that are not monotone (the minimum load: only more occupancy can satisfy it) judge only
//! once every task is decided — otherwise a partial assignment would cut off sound solutions.

use crate::model::ResourceId;
use std::collections::{BTreeSet, HashMap};
use unifier::VariableId;
use unifier::constraint::{BucketRange, BucketedTask, Constraint, PropagationResult};
use unifier::model::domain::{Domain, TrailedDomains};
use unifier::score::Objective;

/// A holder's occupied sections, merged into blocks per bucket window.
#[derive(Debug, Clone)]
pub(crate) struct Track {
    tasks: Vec<BucketedTask>,
    /// Sorted by start.
    ranges: Vec<BucketRange>,
    scope: Vec<VariableId>,
}

impl Track {
    pub(crate) fn new(tasks: Vec<BucketedTask>, mut ranges: Vec<BucketRange>) -> Self {
        ranges.sort_unstable_by_key(|range| range.start);
        let mut scope: Vec<VariableId> = tasks
            .iter()
            .flat_map(|task| std::iter::once(task.start).chain(task.presence))
            .collect();
        scope.sort_unstable();
        scope.dedup();
        Self {
            tasks,
            ranges,
            scope,
        }
    }

    fn present(task: &BucketedTask, assignment: &HashMap<VariableId, i64>) -> bool {
        task.presence
            .is_none_or(|presence| assignment.get(&presence) == Some(&1))
    }

    /// Whether every task is decided: start and, if present, presence.
    fn decided(&self, assignment: &HashMap<VariableId, i64>) -> bool {
        self.scope.iter().all(|var| assignment.contains_key(var))
    }

    /// Per bucket window the occupied blocks `[start, end)`, sorted and merged; whatever extends
    /// beyond a window is cut off at its edge.
    fn blocks(&self, assignment: &HashMap<VariableId, i64>) -> Vec<Vec<(i64, i64)>> {
        let mut per_range: Vec<Vec<(i64, i64)>> = vec![Vec::new(); self.ranges.len()];
        for task in &self.tasks {
            if !Self::present(task, assignment) {
                continue;
            }
            let Some(&start) = assignment.get(&task.start) else {
                continue;
            };
            let end = start.saturating_add(task.duration.max(1));
            for (index, range) in self.ranges.iter().enumerate() {
                let from = start.max(range.start);
                let to = end.min(range.end);
                if from < to {
                    per_range[index].push((from, to));
                }
            }
        }
        for intervals in &mut per_range {
            intervals.sort_unstable();
            let mut merged: Vec<(i64, i64)> = Vec::with_capacity(intervals.len());
            for &(start, end) in intervals.iter() {
                match merged.last_mut() {
                    Some(last) if start <= last.1 => last.1 = last.1.max(end),
                    _ => merged.push((start, end)),
                }
            }
            *intervals = merged;
        }
        per_range
    }
}

fn occupied(blocks: &[(i64, i64)]) -> i64 {
    blocks.iter().map(|(start, end)| end - start).sum()
}

/// Occupied units of `blocks` within the window `[from, to)`.
fn occupied_within(blocks: &[(i64, i64)], from: i64, to: i64) -> i64 {
    blocks
        .iter()
        .map(|&(start, end)| (end.min(to) - start.max(from)).max(0))
        .sum()
}

fn to_u32(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

/// The individual rules. Each is a constraint over a [`Track`]; `violations` counts the missing
/// or excess units so that a search sees the direction.
#[derive(Debug, Clone, Copy)]
pub(crate) enum BucketRuleKind {
    /// In every active bucket at least `limit` occupied units.
    MinimumLoad { limit: i64 },
    /// In every bucket at most `limit` units from the first start to the last end.
    MaximumSpan { limit: i64 },
    /// Between two consecutive active buckets at least `limit` free units: from the last end to
    /// the end of the window plus from the start of the window to the first start.
    MinimumGapBetween { limit: i64 },
    /// In every active bucket a free run of at least `min_run` units within the section
    /// `[start_offset, end_offset)` of the window.
    FreeRunWithin {
        start_offset: i64,
        end_offset: i64,
        min_run: i64,
    },
    /// Across all buckets at most `limit` occupied units within the section
    /// `[start_offset, end_offset)` of each window.
    MaximumAtOffsets {
        start_offset: i64,
        end_offset: i64,
        limit: i64,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct BucketRule {
    track: Track,
    kind: BucketRuleKind,
}

impl BucketRule {
    pub(crate) fn new(track: Track, kind: BucketRuleKind) -> Self {
        Self { track, kind }
    }

    fn count(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let blocks = self.track.blocks(assignment);
        let ranges = &self.track.ranges;
        match self.kind {
            BucketRuleKind::MinimumLoad { limit } => {
                if !self.track.decided(assignment) {
                    return 0;
                }
                blocks
                    .iter()
                    .map(|day| occupied(day))
                    .filter(|&load| load > 0)
                    .map(|load| (limit - load).max(0))
                    .sum()
            }
            BucketRuleKind::MaximumSpan { limit } => blocks
                .iter()
                .filter_map(|day| Some(day.last()?.1 - day.first()?.0))
                .map(|span| (span - limit).max(0))
                .sum(),
            BucketRuleKind::MinimumGapBetween { limit } => {
                let mut deficit = 0;
                for index in 1..ranges.len() {
                    let (Some(evening), Some(morning)) =
                        (blocks[index - 1].last(), blocks[index].first())
                    else {
                        continue;
                    };
                    let gap =
                        (ranges[index - 1].end - evening.1) + (morning.0 - ranges[index].start);
                    deficit += (limit - gap).max(0);
                }
                deficit
            }
            BucketRuleKind::FreeRunWithin {
                start_offset,
                end_offset,
                min_run,
            } => {
                let mut deficit = 0;
                for (range, day) in ranges.iter().zip(&blocks) {
                    if day.is_empty() {
                        continue;
                    }
                    let from = range.start + start_offset;
                    let to = (range.start + end_offset).min(range.end);
                    let mut longest = 0;
                    let mut cursor = from;
                    for &(start, end) in day {
                        if end <= from || start >= to {
                            continue;
                        }
                        longest = longest.max(start.max(from) - cursor);
                        cursor = cursor.max(end.min(to));
                    }
                    longest = longest.max(to - cursor);
                    deficit += (min_run - longest).max(0);
                }
                deficit
            }
            BucketRuleKind::MaximumAtOffsets {
                start_offset,
                end_offset,
                limit,
            } => {
                let total: i64 = ranges
                    .iter()
                    .zip(&blocks)
                    .map(|(range, day)| {
                        occupied_within(day, range.start + start_offset, range.start + end_offset)
                    })
                    .sum();
                (total - limit).max(0)
            }
        }
    }
}

impl Constraint for BucketRule {
    fn name(&self) -> &str {
        match self.kind {
            BucketRuleKind::MinimumLoad { .. } => "MinimumDailyLoad",
            BucketRuleKind::MaximumSpan { .. } => "MaximumDailySpan",
            BucketRuleKind::MinimumGapBetween { .. } => "MinimumGapBetweenBuckets",
            BucketRuleKind::FreeRunWithin { .. } => "FreeRunWithin",
            BucketRuleKind::MaximumAtOffsets { .. } => "MaximumOccupancyAtOffsets",
        }
    }

    fn scope(&self) -> &[VariableId] {
        &self.track.scope
    }

    fn is_satisfied(&self, assignment: &HashMap<VariableId, i64>) -> bool {
        self.count(assignment) == 0
    }

    fn violations(&self, assignment: &HashMap<VariableId, i64>) -> u32 {
        to_u32(self.count(assignment))
    }

    /// No domain filtering: whether a day becomes too short or too long only settles with
    /// several tasks. The rule stays hard through `is_satisfied`.
    fn propagate(&self, _domains: &mut TrailedDomains) -> PropagationResult {
        PropagationResult::Success { changed: false }
    }
}

/// At most `limit` distinct groups per bucket — "at most five different subjects per day".
/// A task belongs to the bucket in which it starts.
#[derive(Debug, Clone)]
pub(crate) struct DistinctPerBucket {
    /// `(task, group key)`.
    tasks: Vec<(BucketedTask, usize)>,
    ranges: Vec<BucketRange>,
    limit: usize,
    scope: Vec<VariableId>,
}

impl DistinctPerBucket {
    pub(crate) fn new(
        tasks: Vec<(BucketedTask, usize)>,
        mut ranges: Vec<BucketRange>,
        limit: usize,
    ) -> Self {
        ranges.sort_unstable_by_key(|range| range.start);
        let mut scope: Vec<VariableId> = tasks
            .iter()
            .flat_map(|(task, _)| std::iter::once(task.start).chain(task.presence))
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

    fn excess(&self, assignment: &HashMap<VariableId, i64>) -> usize {
        let mut per_range: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); self.ranges.len()];
        for (task, key) in &self.tasks {
            if !Track::present(task, assignment) {
                continue;
            }
            let Some(&start) = assignment.get(&task.start) else {
                continue;
            };
            if let Some(index) = self
                .ranges
                .iter()
                .position(|range| range.start <= start && start < range.end)
            {
                per_range[index].insert(*key);
            }
        }
        per_range
            .iter()
            .map(|keys| keys.len().saturating_sub(self.limit))
            .sum()
    }
}

impl Constraint for DistinctPerBucket {
    fn name(&self) -> &str {
        "MaximumDistinctPerBucket"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn is_satisfied(&self, assignment: &HashMap<VariableId, i64>) -> bool {
        self.excess(assignment) == 0
    }

    fn violations(&self, assignment: &HashMap<VariableId, i64>) -> u32 {
        u32::try_from(self.excess(assignment)).unwrap_or(u32::MAX)
    }

    fn propagate(&self, _domains: &mut TrailedDomains) -> PropagationResult {
        PropagationResult::Success { changed: false }
    }
}

fn penalty(weight: i64, amount: i64) -> i64 {
    amount.saturating_mul(weight).saturating_neg()
}

fn bound(weight: i64) -> i64 {
    if weight < 0 { i64::MAX } else { 0 }
}

/// A single occupied block alone in one half of an active bucket costs — "do not come in for a
/// single hour in the afternoon". `split_offset` separates the halves; alone means: in that half
/// exactly `max_units` or fewer occupied units, but more than none.
#[derive(Debug)]
pub(crate) struct IsolatedBlockObjective {
    track: Track,
    split_offset: i64,
    max_units: i64,
    weight: i64,
}

impl IsolatedBlockObjective {
    pub(crate) fn new(track: Track, split_offset: i64, max_units: i64, weight: i64) -> Self {
        Self {
            track,
            split_offset,
            max_units,
            weight,
        }
    }
}

impl Objective for IsolatedBlockObjective {
    fn name(&self) -> &str {
        "IsolatedBlock"
    }

    fn scope(&self) -> &[VariableId] {
        &self.track.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let blocks = self.track.blocks(assignment);
        let mut isolated = 0;
        for (range, day) in self.track.ranges.iter().zip(&blocks) {
            let split = range.start + self.split_offset;
            for (from, to) in [(range.start, split), (split, range.end)] {
                let units = occupied_within(day, from, to);
                if units > 0 && units <= self.max_units {
                    isolated += 1;
                }
            }
        }
        penalty(self.weight, isolated)
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        bound(self.weight)
    }
}

/// A task shared by two holders: present if both are.
#[derive(Debug, Clone)]
pub(crate) struct JointTask {
    pub(crate) start: VariableId,
    pub(crate) presences: Vec<VariableId>,
}

/// In every bucket in which the anchor is active, at least one shared task with the
/// companion — "the form teacher sees their class every day".
#[derive(Debug)]
pub(crate) struct JointPresenceObjective {
    anchor: Track,
    joint: Vec<JointTask>,
    weight: i64,
    scope: Vec<VariableId>,
}

impl JointPresenceObjective {
    pub(crate) fn new(anchor: Track, joint: Vec<JointTask>, weight: i64) -> Self {
        let mut scope = anchor.scope.clone();
        for task in &joint {
            scope.push(task.start);
            scope.extend(task.presences.iter().copied());
        }
        scope.sort_unstable();
        scope.dedup();
        Self {
            anchor,
            joint,
            weight,
            scope,
        }
    }
}

impl Objective for JointPresenceObjective {
    fn name(&self) -> &str {
        "JointPresence"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let blocks = self.anchor.blocks(assignment);
        let mut met = vec![false; self.anchor.ranges.len()];
        for task in &self.joint {
            if !task
                .presences
                .iter()
                .all(|presence| assignment.get(presence) == Some(&1))
            {
                continue;
            }
            let Some(&start) = assignment.get(&task.start) else {
                continue;
            };
            if let Some(index) = self
                .anchor
                .ranges
                .iter()
                .position(|range| range.start <= start && start < range.end)
            {
                met[index] = true;
            }
        }
        let missed = blocks
            .iter()
            .zip(&met)
            .filter(|(day, met)| !day.is_empty() && !**met)
            .count() as i64;
        penalty(self.weight, missed)
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        bound(self.weight)
    }
}

/// At most `limit` holders free at the same time within the section `[start_offset, end_offset)`
/// of their active buckets — "not all classes on lunch break at once". Every unit in which more
/// than `limit` are free costs per excess holder.
#[derive(Debug)]
pub(crate) struct ConcurrentFreeObjective {
    tracks: Vec<Track>,
    start_offset: i64,
    end_offset: i64,
    limit: i64,
    weight: i64,
    scope: Vec<VariableId>,
}

impl ConcurrentFreeObjective {
    pub(crate) fn new(
        tracks: Vec<Track>,
        start_offset: i64,
        end_offset: i64,
        limit: i64,
        weight: i64,
    ) -> Self {
        let mut scope: Vec<VariableId> = tracks
            .iter()
            .flat_map(|track| track.scope.iter().copied())
            .collect();
        scope.sort_unstable();
        scope.dedup();
        Self {
            tracks,
            start_offset,
            end_offset,
            limit,
            weight,
            scope,
        }
    }
}

impl Objective for ConcurrentFreeObjective {
    fn name(&self) -> &str {
        "ConcurrentFree"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        // All holders share the same bucket windows; counting is per unit of the section.
        let mut free: HashMap<i64, i64> = HashMap::new();
        for track in &self.tracks {
            let blocks = track.blocks(assignment);
            for (range, day) in track.ranges.iter().zip(&blocks) {
                if day.is_empty() {
                    continue;
                }
                let from = range.start + self.start_offset;
                let to = (range.start + self.end_offset).min(range.end);
                for unit in from..to {
                    if occupied_within(day, unit, unit + 1) == 0 {
                        *free.entry(unit).or_insert(0) += 1;
                    }
                }
            }
        }
        let excess: i64 = free.values().map(|count| (count - self.limit).max(0)).sum();
        penalty(self.weight, excess)
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        bound(self.weight)
    }
}

/// The kinds of [`ActivityGoalObjective`] — distribution objectives over a set of tasks
/// (timbra plan 69, package B). Offsets count from the start of the respective bucket window.
#[derive(Debug, Clone)]
pub(crate) enum ActivityGoalShape {
    /// Every pair of consecutive bucket windows in which both start something costs.
    NotOnAdjacentBuckets,
    /// Every further start on an already occupied offset costs.
    OffsetVariety,
    /// Every further distinct offset beyond the first costs.
    OffsetStability,
    /// The number of starts in `[start_offset, end_offset)` lies in `[min, max]`; each one below
    /// or above costs.
    StartsWithinOffsets {
        start_offset: i64,
        end_offset: i64,
        min: i64,
        max: i64,
    },
    /// Pairs from **different** groups (`group` per task) in the same bucket.
    Proximity { mode: Proximity },
    /// Every contiguous block of at least `min_length` units that neither starts nor ends at one
    /// of the `edges` offsets costs.
    BlockAtEdges { min_length: i64, edges: Vec<i64> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Proximity {
    /// Directly back to back costs.
    AvoidAdjacent,
    /// On the same day costs.
    AvoidSameBucket,
    /// On the same day, but not directly back to back, costs.
    PreferAdjacent,
}

/// A task of a distribution objective: start, duration and the group it belongs to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GoalTask {
    pub(crate) start: VariableId,
    pub(crate) duration: i64,
    pub(crate) group: usize,
}

#[derive(Debug)]
pub(crate) struct ActivityGoalObjective {
    tasks: Vec<GoalTask>,
    ranges: Vec<BucketRange>,
    shape: ActivityGoalShape,
    weight: i64,
    scope: Vec<VariableId>,
}

impl ActivityGoalObjective {
    pub(crate) fn new(
        tasks: Vec<GoalTask>,
        mut ranges: Vec<BucketRange>,
        shape: ActivityGoalShape,
        weight: i64,
    ) -> Self {
        ranges.sort_unstable_by_key(|range| range.start);
        let mut scope: Vec<VariableId> = tasks.iter().map(|task| task.start).collect();
        scope.sort_unstable();
        scope.dedup();
        Self {
            tasks,
            ranges,
            shape,
            weight,
            scope,
        }
    }

    /// Per assigned task: bucket index (in `ranges`), offset, end, group.
    fn placed(&self, assignment: &HashMap<VariableId, i64>) -> Vec<(usize, i64, i64, usize)> {
        self.tasks
            .iter()
            .filter_map(|task| {
                let start = *assignment.get(&task.start)?;
                let index = self
                    .ranges
                    .iter()
                    .position(|range| range.start <= start && start < range.end)?;
                let offset = start - self.ranges[index].start;
                Some((index, offset, offset + task.duration.max(1), task.group))
            })
            .collect()
    }

    fn count(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let placed = self.placed(assignment);
        match &self.shape {
            ActivityGoalShape::NotOnAdjacentBuckets => {
                let used: BTreeSet<usize> = placed.iter().map(|p| p.0).collect();
                used.iter()
                    .filter(|&&index| used.contains(&(index + 1)))
                    .count() as i64
            }
            ActivityGoalShape::OffsetVariety => {
                let mut seen: HashMap<i64, i64> = HashMap::new();
                for p in &placed {
                    *seen.entry(p.1).or_insert(0) += 1;
                }
                seen.values().map(|count| count - 1).sum()
            }
            ActivityGoalShape::OffsetStability => {
                let offsets: BTreeSet<i64> = placed.iter().map(|p| p.1).collect();
                (offsets.len() as i64 - 1).max(0)
            }
            ActivityGoalShape::StartsWithinOffsets {
                start_offset,
                end_offset,
                min,
                max,
            } => {
                let inside = placed
                    .iter()
                    .filter(|p| *start_offset <= p.1 && p.1 < *end_offset)
                    .count() as i64;
                (inside - max).max(0) + (min - inside).max(0)
            }
            ActivityGoalShape::Proximity { mode } => {
                let mut total = 0;
                for (i, a) in placed.iter().enumerate() {
                    for b in &placed[i + 1..] {
                        if a.0 != b.0 || a.3 == b.3 {
                            continue;
                        }
                        let touching = a.2 == b.1 || b.2 == a.1;
                        total += match mode {
                            Proximity::AvoidAdjacent => i64::from(touching),
                            Proximity::AvoidSameBucket => 1,
                            Proximity::PreferAdjacent => i64::from(!touching),
                        };
                    }
                }
                total
            }
            ActivityGoalShape::BlockAtEdges { min_length, edges } => {
                let mut per_bucket: HashMap<usize, Vec<(i64, i64)>> = HashMap::new();
                for p in &placed {
                    per_bucket.entry(p.0).or_default().push((p.1, p.2));
                }
                let mut stray = 0;
                for (index, mut intervals) in per_bucket {
                    intervals.sort_unstable();
                    let length = self.ranges[index].end - self.ranges[index].start;
                    let mut blocks: Vec<(i64, i64)> = Vec::new();
                    for (start, end) in intervals {
                        match blocks.last_mut() {
                            Some(last) if start <= last.1 => last.1 = last.1.max(end),
                            _ => blocks.push((start, end)),
                        }
                    }
                    for (start, end) in blocks {
                        let at_edge = start == 0
                            || end >= length
                            || edges.iter().any(|&edge| start == edge || end == edge);
                        if end - start >= *min_length && !at_edge {
                            stray += 1;
                        }
                    }
                }
                stray
            }
        }
    }
}

impl Objective for ActivityGoalObjective {
    fn name(&self) -> &str {
        "ActivityGoal"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        penalty(self.weight, self.count(assignment))
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        bound(self.weight)
    }
}

/// The room choice of a task: fixed rooms (`None`) and candidates with a selection variable.
pub(crate) type RoomOptions = Vec<(ResourceId, Option<VariableId>)>;

fn chosen(options: &RoomOptions, assignment: &HashMap<VariableId, i64>) -> Vec<ResourceId> {
    options
        .iter()
        .filter(|(_, presence)| {
            presence.is_none_or(|presence| assignment.get(&presence) == Some(&1))
        })
        .map(|(resource, _)| *resource)
        .collect()
}

/// What choosing a room costs (timbra plan 69, package C): per task the cost of the chosen
/// rooms; a room not listed costs `default_cost`.
#[derive(Debug)]
pub(crate) struct ResourceCostObjective {
    tasks: Vec<RoomOptions>,
    costs: HashMap<ResourceId, i64>,
    default_cost: i64,
    weight: i64,
    scope: Vec<VariableId>,
}

impl ResourceCostObjective {
    pub(crate) fn new(
        tasks: Vec<RoomOptions>,
        costs: HashMap<ResourceId, i64>,
        default_cost: i64,
        weight: i64,
    ) -> Self {
        let mut scope: Vec<VariableId> = tasks
            .iter()
            .flatten()
            .filter_map(|(_, presence)| *presence)
            .collect();
        scope.sort_unstable();
        scope.dedup();
        Self {
            tasks,
            costs,
            default_cost,
            weight,
            scope,
        }
    }
}

impl Objective for ResourceCostObjective {
    fn name(&self) -> &str {
        "ResourceCost"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let total: i64 = self
            .tasks
            .iter()
            .flat_map(|options| chosen(options, assignment))
            .map(|resource| {
                self.costs
                    .get(&resource)
                    .copied()
                    .unwrap_or(self.default_cost)
            })
            .sum();
        penalty(self.weight, total)
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        if self.weight < 0 || self.costs.values().any(|cost| *cost < 0) || self.default_cost < 0 {
            i64::MAX
        } else {
            0
        }
    }
}

/// A task of a holder with its room choice.
#[derive(Debug, Clone)]
pub(crate) struct RoomTask {
    pub(crate) start: VariableId,
    pub(crate) duration: i64,
    /// Whether the holder has this task at all (candidate pool); `None` = certain.
    pub(crate) presence: Option<VariableId>,
    pub(crate) rooms: RoomOptions,
}

/// Room changes and travel times of a holder (timbra plan 69, package C): in every bucket, each
/// two consecutive tasks in different rooms cost `change_weight`; if the rooms are at different
/// locations and less than `travel_gap` is free in between, that additionally costs
/// `travel_weight`.
#[derive(Debug)]
pub(crate) struct RoomChangeObjective {
    tasks: Vec<RoomTask>,
    ranges: Vec<BucketRange>,
    locations: HashMap<ResourceId, u32>,
    travel_gap: i64,
    change_weight: i64,
    travel_weight: i64,
    scope: Vec<VariableId>,
}

impl RoomChangeObjective {
    pub(crate) fn new(
        tasks: Vec<RoomTask>,
        mut ranges: Vec<BucketRange>,
        locations: HashMap<ResourceId, u32>,
        travel_gap: i64,
        change_weight: i64,
        travel_weight: i64,
    ) -> Self {
        ranges.sort_unstable_by_key(|range| range.start);
        let mut scope = Vec::new();
        for task in &tasks {
            scope.push(task.start);
            scope.extend(task.presence);
            scope.extend(task.rooms.iter().filter_map(|(_, presence)| *presence));
        }
        scope.sort_unstable();
        scope.dedup();
        Self {
            tasks,
            ranges,
            locations,
            travel_gap,
            change_weight,
            travel_weight,
            scope,
        }
    }
}

impl Objective for RoomChangeObjective {
    fn name(&self) -> &str {
        "RoomChange"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let mut per_range: Vec<Vec<(i64, i64, Option<ResourceId>)>> =
            vec![Vec::new(); self.ranges.len()];
        for task in &self.tasks {
            if task
                .presence
                .is_some_and(|presence| assignment.get(&presence) != Some(&1))
            {
                continue;
            }
            let Some(&start) = assignment.get(&task.start) else {
                continue;
            };
            let Some(index) = self
                .ranges
                .iter()
                .position(|range| range.start <= start && start < range.end)
            else {
                continue;
            };
            let room = chosen(&task.rooms, assignment).first().copied();
            per_range[index].push((start, start + task.duration.max(1), room));
        }
        let (mut changes, mut travels) = (0i64, 0i64);
        for day in &mut per_range {
            day.sort_unstable_by_key(|entry| entry.0);
            for pair in day.windows(2) {
                let (Some(before), Some(after)) = (pair[0].2, pair[1].2) else {
                    continue;
                };
                if before == after {
                    continue;
                }
                changes += 1;
                let (Some(from), Some(to)) =
                    (self.locations.get(&before), self.locations.get(&after))
                else {
                    continue;
                };
                if from != to && pair[1].0 - pair[0].1 < self.travel_gap {
                    travels += 1;
                }
            }
        }
        penalty(self.change_weight, changes).saturating_add(penalty(self.travel_weight, travels))
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        bound(self.change_weight.min(self.travel_weight))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(starts: &[(u64, i64)]) -> (Track, HashMap<VariableId, i64>) {
        let tasks = starts
            .iter()
            .map(|&(var, _)| BucketedTask::new(VariableId(var as u32), 1, 1))
            .collect();
        let ranges = vec![BucketRange::new(0, 6, 0), BucketRange::new(6, 12, 1)];
        let assignment = starts
            .iter()
            .map(|&(var, start)| (VariableId(var as u32), start))
            .collect();
        (Track::new(tasks, ranges), assignment)
    }

    fn count(kind: BucketRuleKind, starts: &[(u64, i64)]) -> u32 {
        let (track, assignment) = track(starts);
        BucketRule::new(track, kind).violations(&assignment)
    }

    #[test]
    fn minimum_load_counts_only_active_days() {
        // Day 0: two units, day 1: none — only day 0 counts, and it is one short.
        assert_eq!(
            count(BucketRuleKind::MinimumLoad { limit: 3 }, &[(0, 0), (1, 1)]),
            1
        );
    }

    #[test]
    fn span_measures_first_start_to_last_end() {
        // 0 to 5: span 5, 3 allowed.
        assert_eq!(
            count(BucketRuleKind::MaximumSpan { limit: 3 }, &[(0, 0), (1, 4)]),
            2
        );
    }

    #[test]
    fn gap_between_days_adds_evening_and_morning() {
        // Occupied until 6 in the evening (end 6), from 6 in the morning: no break; 2 required.
        assert_eq!(
            count(
                BucketRuleKind::MinimumGapBetween { limit: 2 },
                &[(0, 5), (1, 6)]
            ),
            2
        );
        // Evening end 4 (2 free), morning from 7 (1 free): enough.
        assert_eq!(
            count(
                BucketRuleKind::MinimumGapBetween { limit: 2 },
                &[(0, 3), (1, 7)]
            ),
            0
        );
    }

    #[test]
    fn free_run_looks_inside_the_window() {
        let lunch = BucketRuleKind::FreeRunWithin {
            start_offset: 2,
            end_offset: 5,
            min_run: 2,
        };
        // Occupied 2 and 4: in the window [2, 5) only 3 stays free — one unit short.
        assert_eq!(count(lunch, &[(0, 2), (1, 4)]), 1);
        // Occupied 0 and 5: the window is entirely free.
        assert_eq!(count(lunch, &[(0, 0), (1, 5)]), 0);
    }

    #[test]
    fn occupancy_at_offsets_sums_over_days() {
        let last = BucketRuleKind::MaximumAtOffsets {
            start_offset: 5,
            end_offset: 6,
            limit: 1,
        };
        assert_eq!(count(last, &[(0, 5), (1, 11)]), 1);
    }
}
