#![cfg(test)]
//! Boundary tests for the checked/saturating arithmetic in the circle
//! contract (issue #693): payout fee split, streak tracking, health-score
//! counters and contribution pagination.

use crate::contract::calculate_health_score;
use crate::types::*;
use crate::{Circle, CircleArgs, CircleClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String, Vec};

fn config(env: &Env, token: &Address) -> CircleConfig {
    CircleConfig {
        organizer: Address::generate(env),
        token: token.clone(),
        name: String::from_str(env, "Arithmetic Test Circle"),
        contribution_amount: 100_i128,
        max_members: 2,
        payout_type: PAYOUT_FIXED,
        total_rounds: 2,
        contribution_deadline_seconds: 60,
        min_moi_score: 0,
        collateral_amount: 0,
        penalty_bps: 500,
        grace_period_seconds: 0,
        max_strikes: 3,
        slug: String::from_str(env, "arithmetic-test"),
        max_withdrawal_per_tx: 0,
        daily_withdrawal_limit: 0,
        min_duration_seconds: 0,
    }
}

fn setup(env: &Env) -> (CircleClient<'_>, Address, Address, Address) {
    env.mock_all_auths_allowing_non_root_auth();
    let token_admin = Address::generate(env);
    let token = env.register_stellar_asset_contract_v2(token_admin).address();
    let cfg = config(env, &token);
    let admin = cfg.organizer.clone();
    let factory = Address::generate(env);
    let id = env.register(Circle, CircleArgs::__constructor(&admin, &factory, &cfg));
    (CircleClient::new(env, &id), admin, token, id)
}

fn mint(env: &Env, token: &Address, to: &Address, amount: i128) {
    soroban_sdk::token::StellarAssetClient::new(env, token).mint(to, &amount);
}

// ── batch_payout fee split ───────────────────────────────────────────────

#[test]
fn batch_payout_splits_each_amount_into_net_and_fee() {
    let env = Env::default();
    let (client, admin, token, _id) = setup(&env);
    let treasury = Address::generate(&env);
    client.set_treasury(&admin, &treasury);
    client.set_fee_bps(&admin, &500u32); // 5%

    let m1 = Address::generate(&env);
    let m2 = Address::generate(&env);
    client.join(&m1);
    client.join(&m2);
    mint(&env, &token, &m1, 100);
    mint(&env, &token, &m2, 100);
    client.contribute(&m1, &100_i128, &0_u32);
    client.contribute(&m2, &100_i128, &0_u32);

    let mut recipients = Vec::new(&env);
    recipients.push_back(m1.clone());
    recipients.push_back(m2.clone());
    let mut amounts = Vec::new(&env);
    amounts.push_back(80_i128);
    amounts.push_back(120_i128);

    client.batch_payout(&admin, &recipients, &amounts, &0_u32);

    let t = soroban_sdk::token::Client::new(&env, &token);
    assert_eq!(t.balance(&m1), 76); // 80 - 4
    assert_eq!(t.balance(&m2), 114); // 120 - 6
    assert_eq!(t.balance(&treasury), 10); // 4 + 6
}

// ── streak tracking ──────────────────────────────────────────────────────

#[test]
fn streak_grows_on_consecutive_rounds_and_resets_after_a_gap() {
    let env = Env::default();
    let (client, _admin, _token, _id) = setup(&env);
    let member = Address::generate(&env);

    client.update_streak(&member, &1);
    let s = client.get_member_streak(&member);
    assert_eq!((s.current_streak, s.longest_streak, s.last_round), (1, 1, 1));

    client.update_streak(&member, &2);
    let s = client.get_member_streak(&member);
    assert_eq!((s.current_streak, s.longest_streak, s.last_round), (2, 2, 2));

    client.update_streak(&member, &5); // gap
    let s = client.get_member_streak(&member);
    assert_eq!((s.current_streak, s.longest_streak, s.last_round), (1, 2, 5));
}

#[test]
fn streak_update_at_u32_max_round_does_not_overflow() {
    let env = Env::default();
    let (client, _admin, _token, _id) = setup(&env);
    let member = Address::generate(&env);

    client.update_streak(&member, &1);
    client.update_streak(&member, &u32::MAX); // a gap, so it resets
    let s = client.get_member_streak(&member);
    assert_eq!((s.current_streak, s.last_round), (1, u32::MAX));

    // `last_round + 1` would overflow for both of these.
    client.update_streak(&member, &u32::MAX);
    client.update_streak(&member, &1);
    let s = client.get_member_streak(&member);
    assert_eq!((s.current_streak, s.last_round), (1, u32::MAX));
}

#[test]
fn streak_counter_saturates_instead_of_overflowing() {
    let env = Env::default();
    let (client, _admin, _token, id) = setup(&env);
    let member = Address::generate(&env);

    let mut streaks: Vec<Streak> = Vec::new(&env);
    streaks.push_back(Streak {
        member: member.clone(),
        current_streak: u32::MAX,
        longest_streak: u32::MAX,
        last_round: 5,
    });
    env.as_contract(&id, || {
        env.storage().persistent().set(&DataKey::Streaks, &streaks);
    });

    client.update_streak(&member, &6);

    let s = client.get_member_streak(&member);
    assert_eq!(s.current_streak, u32::MAX);
    assert_eq!(s.longest_streak, u32::MAX);
    assert_eq!(s.last_round, 6);
}

// ── health-score counters ────────────────────────────────────────────────

fn member(env: &Env, status: u32) -> Member {
    Member {
        address: Address::generate(env),
        position: 0,
        joined_at: 0,
        strikes: 0,
        status,
        exited_at: 0,
        total_contributions: 0,
        total_received: 0,
    }
}

fn contribution(env: &Env, on_time: bool) -> Contribution {
    Contribution {
        member: Address::generate(env),
        round: 0,
        amount: 100,
        timestamp: 0,
        on_time,
        time_weight: 0,
    }
}

#[test]
fn health_score_of_an_empty_circle_reports_full_rates() {
    let env = Env::default();
    let members: Vec<Member> = Vec::new(&env);
    let contributions: Vec<Contribution> = Vec::new(&env);

    let r = calculate_health_score(&env, &members, &contributions);

    assert_eq!(r.on_time_rate_bps, 10_000);
    assert_eq!(r.completion_rate_bps, 10_000);
    assert_eq!(r.member_retention_bps, 10_000);
    assert!(r.score <= 100);
}

#[test]
fn health_score_rates_count_on_time_defaulted_and_exited() {
    let env = Env::default();
    let mut members: Vec<Member> = Vec::new(&env);
    members.push_back(member(&env, MEMBER_ACTIVE));
    members.push_back(member(&env, MEMBER_ACTIVE));
    members.push_back(member(&env, MEMBER_DEFAULTED));
    members.push_back(member(&env, MEMBER_EXITED));
    let mut contributions: Vec<Contribution> = Vec::new(&env);
    contributions.push_back(contribution(&env, true));
    contributions.push_back(contribution(&env, true));
    contributions.push_back(contribution(&env, true));
    contributions.push_back(contribution(&env, false));

    let r = calculate_health_score(&env, &members, &contributions);

    assert_eq!(r.on_time_rate_bps, 7_500); // 3 of 4
    assert_eq!(r.completion_rate_bps, 7_500); // 3 of 4 did not default
    assert_eq!(r.member_retention_bps, 7_500); // 3 of 4 did not exit
    assert!(r.score <= 100);
}

#[test]
fn health_score_when_every_member_defaulted_has_zero_completion() {
    let env = Env::default();
    let mut members: Vec<Member> = Vec::new(&env);
    members.push_back(member(&env, MEMBER_DEFAULTED));
    members.push_back(member(&env, MEMBER_DEFAULTED));
    let contributions: Vec<Contribution> = Vec::new(&env);

    let r = calculate_health_score(&env, &members, &contributions);

    assert_eq!(r.completion_rate_bps, 0);
    assert_eq!(r.member_retention_bps, 10_000);
}

// ── contribution pagination ──────────────────────────────────────────────

#[test]
fn get_contributions_with_huge_page_arguments_returns_empty_without_overflow() {
    let env = Env::default();
    let (client, _admin, token, _id) = setup(&env);
    let m = Address::generate(&env);
    client.join(&m);
    mint(&env, &token, &m, 100);
    client.contribute(&m, &100_i128, &0_u32);

    assert_eq!(client.get_contributions(&m, &u32::MAX, &u32::MAX).len(), 0);
    assert_eq!(client.get_contributions(&m, &0, &u32::MAX).len(), 1);
}
