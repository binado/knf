//! Per-path merge strategies.
//!
//! A [`Rules`] set narrows alongside the merge's own descent: one lookup per
//! level, and `None` short-circuits an entire subtree, so a document with three
//! rules pays almost nothing.
//!
//! The map is a [`BTreeMap`] and the contrast with [`Map`](crate::Map) is the
//! point. The document map is an `IndexMap` because input order is meaningful;
//! the rule map is a `BTreeMap` because rule order must be *meaningless*. The
//! whole set is validated by [`Rules::build`] in one pass rather than rule by
//! rule, so the same rules always produce the same result — and the same errors,
//! in the same order — whatever order they arrived in.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::render_path;

/// What to do where a layer supplies a value for a path that already has one.
///
/// Every strategy is *terminal*: it consumes the overlay whole and never
/// recurses, so no rule below one can ever fire. The default merge is the
/// absence of a rule, not a variant here.
///
/// The order of the variants is the tie-break used when reporting conflicts, so
/// that the message does not depend on the order rules were given in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Strategy {
    /// Concatenate base ++ overlay. Both sides must be arrays.
    Append,
    /// Assign wholesale, no recursion, even object over object.
    Replace,
    /// Error. The first layer to define the path pins it.
    Fail,
}

impl fmt::Display for Strategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Append => "append",
            Self::Replace => "replace",
            Self::Fail => "fail",
        })
    }
}

/// A validated set of per-path strategies, shaped as a trie.
///
/// Exact paths only: two rules can meet at one node at most once, so "most
/// specific wins" never has to arbitrate. The trie shape is what makes adding
/// globs later a change to lookup rather than to the data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    strategy: Option<Strategy>,
    children: BTreeMap<String, Rules>,
}

impl Rules {
    /// No rules at all: the default merge everywhere.
    pub const EMPTY: Self = Self {
        strategy: None,
        children: BTreeMap::new(),
    };

    /// Validates a whole rule set at once and builds the trie.
    ///
    /// Validating the finished set rather than each insertion is what makes
    /// order-independence structural rather than a property to be maintained:
    /// unreachability can be created from either direction (`db` then
    /// `db.plugins`, or the reverse) and both are caught without an insert-time
    /// symmetry argument.
    ///
    /// A duplicate path with the *same* strategy is accepted — scripts
    /// accumulate flags. Only differing strategies conflict.
    pub fn build(
        rules: impl IntoIterator<Item = (Vec<String>, Strategy)>,
    ) -> Result<Self, RuleErrors> {
        let mut by_path: BTreeMap<Vec<String>, BTreeSet<Strategy>> = BTreeMap::new();
        for (path, strategy) in rules {
            by_path.entry(path).or_default().insert(strategy);
        }

        let mut errors = Vec::new();
        for (path, strategies) in &by_path {
            if strategies.len() > 1 {
                errors.push(RuleError::Conflict {
                    path: path.clone(),
                    strategies: strategies.clone(),
                });
            }
            if let Some((blocked_by, blocker)) = blocking_prefix(&by_path, path) {
                errors.push(RuleError::Unreachable {
                    path: path.clone(),
                    blocked_by,
                    blocker,
                });
            }
        }
        if !errors.is_empty() {
            errors.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
            return Err(RuleErrors(errors));
        }

        let mut root = Self::default();
        for (path, strategies) in by_path {
            let strategy = strategies
                .into_iter()
                .next()
                .expect("a path in the map has at least one strategy");
            root.insert(path, strategy);
        }
        Ok(root)
    }

    fn insert(&mut self, path: Vec<String>, strategy: Strategy) {
        let mut node = self;
        for segment in path {
            node = node.children.entry(segment).or_default();
        }
        node.strategy = Some(strategy);
    }

    /// The subtree of rules under `key`, or `None` if nothing is nested there.
    pub(crate) fn child(&self, key: &str) -> Option<&Self> {
        self.children.get(key)
    }

    /// The strategy at this node, if a rule names it exactly.
    pub(crate) fn strategy(&self) -> Option<Strategy> {
        self.strategy
    }
}

/// The shallowest rule strictly above `path`, if any.
///
/// Every strategy is terminal, so any ancestor rule blocks. Shallowest rather
/// than nearest because that is the rule that actually stops the walk first;
/// with `--replace a --fail a.b`, `a.b.c` is blocked by `a`. A conflicted
/// ancestor reports its lowest strategy — its conflict is a separate error.
fn blocking_prefix(
    by_path: &BTreeMap<Vec<String>, BTreeSet<Strategy>>,
    path: &[String],
) -> Option<(Vec<String>, Strategy)> {
    (0..path.len()).find_map(|depth| {
        let prefix = &path[..depth];
        let blocker = *by_path.get(prefix)?.iter().next()?;
        Some((prefix.to_vec(), blocker))
    })
}

/// Renders a strategy set as a backticked list, `a`, `b` and `c` — quoted to
/// match the `blocker` in [`RuleError::Unreachable`].
fn render_strategies(strategies: &BTreeSet<Strategy>) -> String {
    let quoted: Vec<String> = strategies.iter().map(|s| format!("`{s}`")).collect();
    match quoted.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Why a rule set was rejected.
///
/// Carries key paths and strategy names and nothing else — no flag names. The
/// caller knows what it called its flags.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuleError {
    /// One path, more than one strategy. The whole set is carried, and reported
    /// in [`Strategy`] order rather than the order the rules arrived in: a user
    /// given only two of three offending flags cannot tell how many to drop.
    #[error(
        "conflicting strategies at `{}`: {}",
        render_path(path),
        render_strategies(strategies)
    )]
    Conflict {
        path: Vec<String>,
        strategies: BTreeSet<Strategy>,
    },
    /// A rule beneath another rule. Every strategy is terminal, so it could
    /// never fire.
    #[error(
        "rule at `{}` can never fire: `{}` is `{blocker}`, which does not recurse",
        render_path(path),
        render_path(blocked_by)
    )]
    Unreachable {
        path: Vec<String>,
        blocked_by: Vec<String>,
        blocker: Strategy,
    },
}

impl RuleError {
    /// The path the rule set was rejected at.
    pub fn path(&self) -> &[String] {
        match self {
            Self::Conflict { path, .. } | Self::Unreachable { path, .. } => path,
        }
    }

    /// Sorted by path, then by kind, so a set's errors do not depend on the
    /// order its rules were given in.
    fn sort_key(&self) -> (&[String], u8) {
        match self {
            Self::Conflict { path, .. } => (path, 0),
            Self::Unreachable { path, .. } => (path, 1),
        }
    }
}

/// Every problem with a rule set, sorted.
///
/// All of them rather than the first: a rule set is given up front, so there is
/// no reason to make the user rediscover it one flag at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleErrors(Vec<RuleError>);

impl RuleErrors {
    pub fn errors(&self) -> &[RuleError] {
        &self.0
    }
}

impl std::error::Error for RuleErrors {}

impl fmt::Display for RuleErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(dotted: &str) -> Vec<String> {
        dotted.split('.').map(str::to_string).collect()
    }

    fn build(rules: &[(&str, Strategy)]) -> Result<Rules, RuleErrors> {
        Rules::build(rules.iter().map(|(p, s)| (path(p), *s)))
    }

    fn errors(rules: &[(&str, Strategy)]) -> Vec<RuleError> {
        build(rules).expect_err("rule set should be rejected").0
    }

    #[test]
    fn a_rule_is_found_at_its_own_path_only() {
        let rules = build(&[("a.b", Strategy::Append)]).expect("valid");
        let a = rules.child("a").expect("a exists");
        assert_eq!(a.strategy(), None);
        assert_eq!(
            a.child("b").expect("a.b exists").strategy(),
            Some(Strategy::Append)
        );
        assert_eq!(a.child("c"), None);
        assert_eq!(rules.child("b"), None);
    }

    #[test]
    fn duplicate_paths_with_one_strategy_are_accepted() {
        let rules = build(&[("db", Strategy::Replace), ("db", Strategy::Replace)]).expect("valid");
        assert_eq!(
            rules.child("db").expect("db exists").strategy(),
            Some(Strategy::Replace)
        );
    }

    /// Conflict detected from either direction, reporting the same set in the
    /// same order both times.
    #[test]
    fn one_path_two_strategies_conflicts_in_both_orders() {
        let forward = errors(&[("db", Strategy::Append), ("db", Strategy::Replace)]);
        let backward = errors(&[("db", Strategy::Replace), ("db", Strategy::Append)]);
        assert_eq!(forward, backward);
        assert_eq!(
            forward,
            [RuleError::Conflict {
                path: path("db"),
                strategies: BTreeSet::from([Strategy::Append, Strategy::Replace]),
            }]
        );
    }

    /// The whole set, not a pair: a message naming two of three flags leaves the
    /// user to rediscover the third on the next run.
    #[test]
    fn a_three_way_conflict_names_every_strategy() {
        let found = build(&[
            ("x", Strategy::Fail),
            ("x", Strategy::Append),
            ("x", Strategy::Replace),
        ])
        .expect_err("rejected");
        assert_eq!(
            found.to_string(),
            "conflicting strategies at `x`: `append`, `replace` and `fail`"
        );
    }

    /// The case from the plan: `--replace db --append db.plugins`, and the same
    /// set written the other way round.
    #[test]
    fn a_rule_under_a_terminal_rule_is_unreachable_in_both_orders() {
        let forward = errors(&[("db", Strategy::Replace), ("db.plugins", Strategy::Append)]);
        let backward = errors(&[("db.plugins", Strategy::Append), ("db", Strategy::Replace)]);
        assert_eq!(forward, backward);
        assert_eq!(
            forward,
            [RuleError::Unreachable {
                path: path("db.plugins"),
                blocked_by: path("db"),
                blocker: Strategy::Replace,
            }]
        );
    }

    /// A sibling is not below anything; only strict prefixes block.
    #[test]
    fn a_sibling_of_a_terminal_rule_is_reachable() {
        build(&[("a.b", Strategy::Replace), ("a.c", Strategy::Append)]).expect("valid");
    }

    /// The blocker is the outermost terminal rule, because that is the one the
    /// walk hits first.
    #[test]
    fn the_shallowest_terminal_rule_is_the_blocker() {
        let found = errors(&[
            ("a", Strategy::Replace),
            ("a.b", Strategy::Fail),
            ("a.b.c", Strategy::Append),
        ]);
        assert_eq!(
            found,
            [
                RuleError::Unreachable {
                    path: path("a.b"),
                    blocked_by: path("a"),
                    blocker: Strategy::Replace,
                },
                RuleError::Unreachable {
                    path: path("a.b.c"),
                    blocked_by: path("a"),
                    blocker: Strategy::Replace,
                },
            ]
        );
    }

    /// Every offender, sorted by path, identical whatever order the flags came in.
    #[test]
    fn multiple_errors_are_reported_sorted_and_order_independently() {
        let rules = [
            ("z.deep", Strategy::Append),
            ("a", Strategy::Fail),
            ("z", Strategy::Replace),
            ("a", Strategy::Append),
        ];
        let mut reversed = rules;
        reversed.reverse();

        let found = build(&rules).expect_err("rejected");
        assert_eq!(found, build(&reversed).expect_err("rejected"));
        assert_eq!(
            found.to_string(),
            "conflicting strategies at `a`: `append` and `fail`\n\
             rule at `z.deep` can never fire: `z` is `replace`, which does not recurse"
        );
    }

    /// The root is spelled as the empty path, like every other error in the crate.
    #[test]
    fn a_root_rule_blocks_everything_below_it() {
        let found = Rules::build([(vec![], Strategy::Replace), (path("a"), Strategy::Append)])
            .expect_err("rejected");
        assert_eq!(
            found.to_string(),
            "rule at `a` can never fire: `<root>` is `replace`, which does not recurse"
        );
    }
}
