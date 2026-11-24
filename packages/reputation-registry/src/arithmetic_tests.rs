#![cfg(test)]
//! Boundary tests for the checked/saturating arithmetic in `storage` and
//! `scoring` (issues #695 and #699): counters must saturate at `u32::MAX`
//! instead of panicking, and score updates must floor at 0.

use crate::types::DataKey;
use crate::{scoring, storage, ReputationRegistry};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(ReputationRegistry, ());
    let member = Address::generate(&env);
    let circle = Address::generate(&env);
    (env, contract_id, member, circle)
}

// ── #699: counters in storage.rs ─────────────────────────────────────────

#[test]
fn increment_streak_adds_one() {
    let (env, id, member, circle) = setup();
    env.as_contract(&id, || {
        storage::increment_streak(&env, &member, &circle);
        storage::increment_streak(&env, &member, &circle);
        assert_eq!(storage::get_streak(&env, &member, &circle), 2);
    });
}

#[test]
fn increment_streak_saturates_at_u32_max() {
    let (env, id, member, circle) = setup();
    env.as_contract(&id, || {
        let key = DataKey::Streak(member.clone(), circle.clone());
        env.storage().persistent().set(&key, &u32::MAX);
        storage::increment_streak(&env, &member, &circle);
        assert_eq!(storage::get_streak(&env, &member, &circle), u32::MAX);
    });
}

#[test]
fn increment_completions_and_defaults_add_one() {
    let (env, id, member, _circle) = setup();
    env.as_contract(&id, || {
        storage::increment_completions(&env, &member);
        storage::increment_defaults(&env, &member);
        storage::increment_defaults(&env, &member);
        assert_eq!(storage::get_completions(&env, &member), 1);
        assert_eq!(storage::get_defaults(&env, &member), 2);
    });
}

#[test]
fn increment_completions_and_defaults_saturate_at_u32_max() {
    let (env, id, member, _circle) = setup();
    env.as_contract(&id, || {
        env.storage()
            .persistent()
            .set(&DataKey::Completions(member.clone()), &u32::MAX);
        env.storage()
            .persistent()
            .set(&DataKey::Defaults(member.clone()), &u32::MAX);
        storage::increment_completions(&env, &member);
        storage::increment_defaults(&env, &member);
        assert_eq!(storage::get_completions(&env, &member), u32::MAX);
        assert_eq!(storage::get_defaults(&env, &member), u32::MAX);
    });
}

// ── #695: scoring.rs ─────────────────────────────────────────────────────

#[test]
fn consecutive_rounds_build_a_streak() {
    let (env, id, member, circle) = setup();
    env.as_contract(&id, || {
        scoring::record_on_time_payment(&env, &member, &circle, 0, 1);
        scoring::record_on_time_payment(&env, &member, &circle, 0, 2);
        scoring::record_on_time_payment(&env, &member, &circle, 0, 3);
        assert_eq!(storage::get_streak(&env, &member, &circle), 3);
    });
}

#[test]
fn skipped_round_resets_the_streak_to_one() {
    let (env, id, member, circle) = setup();
    env.as_contract(&id, || {
        scoring::record_on_time_payment(&env, &member, &circle, 0, 1);
        scoring::record_on_time_payment(&env, &member, &circle, 0, 2);
        scoring::record_on_time_payment(&env, &member, &circle, 0, 5);
        assert_eq!(storage::get_streak(&env, &member, &circle), 1);
    });
}

#[test]
fn last_round_at_u32_max_does_not_overflow() {
    let (env, id, member, circle) = setup();
    env.as_contract(&id, || {
        storage::set_last_round(&env, &member, &circle, u32::MAX);
        // `last_round + 1` would overflow here; the streak must just reset.
        scoring::record_on_time_payment(&env, &member, &circle, 0, 1);
        assert_eq!(storage::get_streak(&env, &member, &circle), 1);
        assert_eq!(storage::get_last_round(&env, &member, &circle), 1);
    });
}

#[test]
fn streak_at_u32_max_saturates_and_score_stays_capped() {
    let (env, id, member, circle) = setup();
    env.as_contract(&id, || {
        env.storage()
            .persistent()
            .set(&DataKey::Streak(member.clone(), circle.clone()), &u32::MAX);
        storage::set_last_round(&env, &member, &circle, 7);
        let score = scoring::record_on_time_payment(&env, &member, &circle, 0, 8);
        assert_eq!(storage::get_streak(&env, &member, &circle), u32::MAX);
        // A streak past 10 earns the flat 50 bonus: 10 base + 50 = 60.
        assert_eq!(score, 60);
    });
}

#[test]
fn streak_bonus_is_five_per_round_up_to_ten_then_flat() {
    let (env, id, member, circle) = setup();
    env.as_contract(&id, || {
        // Round 1: base 10 + streak 1 * 5 = 15.
        assert_eq!(scoring::record_on_time_payment(&env, &member, &circle, 0, 1), 15);
        // Round 2: +10 base + 2 * 5 = 15 + 20 = 35.
        assert_eq!(scoring::record_on_time_payment(&env, &member, &circle, 0, 2), 35);
    });
}

#[test]
fn record_default_subtracts_200_and_floors_at_zero() {
    let (env, id, member, _circle) = setup();
    env.as_contract(&id, || {
        storage::set_score(&env, &member, 1000);
        assert_eq!(scoring::record_default(&env, &member), 800);

        storage::set_score(&env, &member, 201);
        assert_eq!(scoring::record_default(&env, &member), 1);

        storage::set_score(&env, &member, 200);
        assert_eq!(scoring::record_default(&env, &member), 0);

        storage::set_score(&env, &member, 150);
        assert_eq!(scoring::record_default(&env, &member), 0);

        storage::set_score(&env, &member, 0);
        assert_eq!(scoring::record_default(&env, &member), 0);
    });
}

#[test]
fn inactivity_decay_is_five_points_per_30_days_and_floors_at_zero() {
    let (env, id, member, _circle) = setup();
    env.as_contract(&id, || {
        storage::set_score(&env, &member, 100);
        assert_eq!(scoring::apply_inactivity_decay(&env, &member, 29), 100);
        assert_eq!(scoring::apply_inactivity_decay(&env, &member, 30), 95);
        // 300 days = 10 months = 50 points off the remaining 95.
        assert_eq!(scoring::apply_inactivity_decay(&env, &member, 300), 45);

        storage::set_score(&env, &member, 5);
        assert_eq!(scoring::apply_inactivity_decay(&env, &member, 30), 0);
        assert_eq!(scoring::apply_inactivity_decay(&env, &member, 3000), 0);
    });
}

#[test]
fn inactivity_decay_with_enormous_days_clears_the_score_instead_of_wrapping() {
    let (env, id, member, _circle) = setup();
    env.as_contract(&id, || {
        // The old `as u32` cast truncated this decay to an arbitrary small
        // number; it must saturate and floor the score at 0.
        storage::set_score(&env, &member, 1000);
        assert_eq!(scoring::apply_inactivity_decay(&env, &member, u64::MAX), 0);
    });
}
