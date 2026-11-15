//! Quota decisions: pure, deterministic, no I/O.
//!
//! The service polls usage, the engine decides. Keeping the decision separate
//! from the polling loop is what makes the "allow vs block" boundary testable
//! without a firewall, a clock, or a network.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use netquota_core::policy::Enforcement;
use netquota_core::quota::Evaluation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    None,
    /// Crossed the limit during this call.
    Exhausted,
    /// Limit no longer exceeded (period reset or credit granted).
    Recovered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub block: bool,
    pub transition: Transition,
    /// True once per exhaustion, so the user gets one notification, not one per
    /// poll (the service polls every second).
    pub notify: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct AppState {
    window_start_ms: Option<i64>,
    exhausted: bool,
    /// Set while we hold a rule, so a policy change can release it.
    blocking: bool,
}

#[derive(Debug, Default)]
pub struct QuotaEngine {
    states: HashMap<String, AppState>,
}

impl QuotaEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn forget(&mut self, app_key: &str) {
        self.states.remove(app_key);
    }

    /// Drop bookkeeping for apps that disappeared, so the map cannot grow
    /// without bound on a long-running service.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.states.retain(|key, _| keep(key));
    }

    pub fn is_exhausted(&self, app_key: &str) -> bool {
        self.states.get(app_key).is_some_and(|s| s.exhausted)
    }

    pub fn is_blocking(&self, app_key: &str) -> bool {
        self.states.get(app_key).is_some_and(|s| s.blocking)
    }

    /// Decide for one application.
    ///
    /// `background` is the background-policy verdict from the caller (the
    /// engine only owns quota logic). `paused` means the user explicitly
    /// suspended enforcement for this app.
    pub fn decide(
        &mut self,
        app_key: &str,
        window_start_ms: i64,
        evaluation: &Evaluation,
        enforcement: Enforcement,
        background_blocked: bool,
        paused: bool,
    ) -> Decision {
        let state = self.states.entry(app_key.to_string()).or_default();
        let was_exhausted = state.exhausted;

        // A new period starts clean: old exhaustion does not leak into it.
        if state.window_start_ms != Some(window_start_ms) {
            state.window_start_ms = Some(window_start_ms);
            state.exhausted = false;
        }

        let mut transition = Transition::None;

        if evaluation.exhausted.is_some() && !was_exhausted {
            state.exhausted = true;
            transition = Transition::Exhausted;
        } else if evaluation.exhausted.is_none() && was_exhausted {
            state.exhausted = false;
            transition = Transition::Recovered;
        }

        let quota_block = state.exhausted
            && !paused
            && matches!(
                enforcement,
                Enforcement::Block | Enforcement::BlockBackground
            )
            && (enforcement == Enforcement::Block || background_blocked);

        let block = !paused && (quota_block || background_blocked);
        state.blocking = block;

        Decision {
            block,
            transition,
            notify: transition == Transition::Exhausted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use netquota_core::quota::{evaluate, Limit, Usage};

    const MB: u64 = 1_000_000;
    const WINDOW: i64 = 1_700_000_000_000;

    fn eval(used: u64) -> Evaluation {
        evaluate(
            Usage { down: used, up: 0 },
            Limit { total: Some(100 * MB), download: None, upload: None },
            None,
            0,
            0.8,
        )
    }

    #[test]
    fn blocks_at_the_boundary_not_before() {
        let mut engine = QuotaEngine::new();
        let under = engine.decide("a", WINDOW, &eval(99 * MB), Enforcement::Block, false, false);
        assert!(!under.block);

        let at = engine.decide("a", WINDOW, &eval(100 * MB), Enforcement::Block, false, false);
        assert!(at.block);
        assert_eq!(at.transition, Transition::Exhausted);
        assert!(at.notify);

        // Second poll stays blocked but does not notify twice.
        let again = engine.decide("a", WINDOW, &eval(101 * MB), Enforcement::Block, false, false);
        assert!(again.block);
        assert!(!again.notify);
        assert_eq!(again.transition, Transition::None);
    }

    #[test]
    fn new_period_releases_the_block() {
        let mut engine = QuotaEngine::new();
        engine.decide("a", WINDOW, &eval(120 * MB), Enforcement::Block, false, false);
        let reset = engine.decide("a", WINDOW + 86_400_000, &eval(0), Enforcement::Block, false, false);
        assert!(!reset.block);
        assert_eq!(reset.transition, Transition::Recovered);
    }

    #[test]
    fn credit_grant_reaches_the_user() {
        let mut engine = QuotaEngine::new();
        engine.decide("a", WINDOW, &eval(100 * MB), Enforcement::Block, false, false);
        // The caller passes an evaluation that already includes the credit.
        let with_credit = evaluate(
            Usage { down: 100 * MB, up: 0 },
            Limit { total: Some(100 * MB), download: None, upload: None },
            Some(&netquota_core::quota::Credit {
                extra_total: 50 * MB,
                extra_down: 0,
                extra_up: 0,
                until_ms: None,
                reason: "top-up".into(),
            }),
            0,
            0.8,
        );
        let after = engine.decide("a", WINDOW, &with_credit, Enforcement::Block, false, false);
        assert!(!after.block);
        assert_eq!(after.transition, Transition::Recovered);
    }

    #[test]
    fn notify_only_never_blocks() {
        let mut engine = QuotaEngine::new();
        let d = engine.decide("a", WINDOW, &eval(500 * MB), Enforcement::NotifyOnly, false, false);
        assert!(!d.block);
        assert_eq!(d.transition, Transition::Exhausted);
        assert!(d.notify, "the user is told even when blocking is off");
    }

    #[test]
    fn block_background_blocks_only_in_background() {
        let mut engine = QuotaEngine::new();
        let fg = engine.decide("a", WINDOW, &eval(500 * MB), Enforcement::BlockBackground, false, false);
        assert!(!fg.block);
        let bg = engine.decide("a", WINDOW, &eval(500 * MB), Enforcement::BlockBackground, true, false);
        assert!(bg.block);
    }

    #[test]
    fn background_policy_applies_below_quota() {
        let mut engine = QuotaEngine::new();
        let d = engine.decide("a", WINDOW, &eval(MB), Enforcement::BlockBackground, true, false);
        assert!(d.block, "background rule is independent of quota");
        assert_eq!(d.transition, Transition::None);
    }

    #[test]
    fn paused_app_is_never_blocked() {
        let mut engine = QuotaEngine::new();
        engine.decide("a", WINDOW, &eval(500 * MB), Enforcement::Block, false, false);
        let d = engine.decide("a", WINDOW, &eval(500 * MB), Enforcement::Block, true, true);
        assert!(!d.block);
    }

    #[test]
    fn state_is_per_application() {
        let mut engine = QuotaEngine::new();
        engine.decide("a", WINDOW, &eval(500 * MB), Enforcement::Block, false, false);
        let other = engine.decide("b", WINDOW, &eval(0), Enforcement::Block, false, false);
        assert!(!other.block);
        assert!(engine.is_exhausted("a"));
        assert!(!engine.is_exhausted("b"));
    }
}
