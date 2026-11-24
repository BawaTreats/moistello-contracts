#![cfg(test)]
//! Tests for the vote-started check in `cancel_proposal` and the cursor/index
//! arithmetic in the proposal listings (issue #698).

use crate::types::{GovernanceConfig, GovernanceError, ProposalAction, ProposalStatus, VoteType};
use crate::{Governance, GovernanceClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env, Symbol, Vec};

fn config() -> GovernanceConfig {
    GovernanceConfig {
        proposal_deposit: 100i128,
        voting_period_seconds: 604800u64,
        timelock_seconds: 86400u64,
        quorum_votes: 1u32,
        pass_threshold_bps: 5000u32,
        min_proposal_deposit: 100i128,
    }
}

fn setup(env: &Env) -> (GovernanceClient<'_>, Address) {
    let admin = Address::generate(env);
    let contract_id = env.register(Governance, ());
    let client = GovernanceClient::new(env, &contract_id);
    env.mock_all_auths();
    client.init(&admin, &config());
    (client, admin)
}

fn noop_action(env: &Env, target: &Address) -> ProposalAction {
    ProposalAction {
        target_contract: target.clone(),
        method: Symbol::new(env, "noop"),
        args: Vec::new(env),
    }
}

fn propose(env: &Env, client: &GovernanceClient<'_>, admin: &Address, tag: u8) -> u64 {
    client.create_proposal(
        admin,
        &config().proposal_deposit,
        &noop_action(env, admin),
        &BytesN::from_array(env, &[tag; 32]),
    )
}

#[test]
fn cancel_without_any_votes_succeeds() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let id = propose(&env, &client, &admin, 1);

    client.cancel_proposal(&admin, &id);

    assert_eq!(client.get_proposal(&id).status, ProposalStatus::Cancelled);
}

#[test]
fn cancel_is_rejected_once_any_kind_of_vote_exists() {
    let kinds = [VoteType::For, VoteType::Against, VoteType::Abstain];
    for (i, kind) in kinds.iter().enumerate() {
        let env = Env::default();
        let (client, admin) = setup(&env);
        let id = propose(&env, &client, &admin, i as u8 + 1);
        let voter = Address::generate(&env);

        client.cast_vote(&voter, &id, kind);

        assert_eq!(
            client.try_cancel_proposal(&admin, &id),
            Err(Ok(GovernanceError::VotingAlreadyStarted)),
            "a {:?} vote must block cancellation",
            kind
        );
        assert_eq!(client.get_proposal(&id).status, ProposalStatus::Active);
    }
}

#[test]
fn metadata_page_with_cursor_beyond_total_is_empty_and_does_not_overflow() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    propose(&env, &client, &admin, 1);
    propose(&env, &client, &admin, 2);

    let page = client.get_proposal_metadata_page(&u64::MAX, &10);
    assert_eq!(page.entries.len(), 0);
    assert_eq!(page.next_cursor, u64::MAX);
    assert_eq!(page.total, 2);
}

#[test]
fn metadata_page_walks_all_proposals_with_a_moving_cursor() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    for tag in 1..=5u8 {
        propose(&env, &client, &admin, tag);
    }

    let first = client.get_proposal_metadata_page(&0, &2);
    assert_eq!(first.entries.len(), 2);
    assert_eq!(first.next_cursor, 2);

    let second = client.get_proposal_metadata_page(&first.next_cursor, &2);
    assert_eq!(second.entries.len(), 2);
    assert_eq!(second.next_cursor, 4);

    let last = client.get_proposal_metadata_page(&second.next_cursor, &2);
    assert_eq!(last.entries.len(), 1);
    assert_eq!(last.next_cursor, 5);
    assert_eq!(last.total, 5);
}

#[test]
fn proposals_by_status_respects_the_limit() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    for tag in 1..=4u8 {
        propose(&env, &client, &admin, tag);
    }

    assert_eq!(client.get_proposals(&ProposalStatus::Active, &2).len(), 2);
    assert_eq!(client.get_proposals(&ProposalStatus::Active, &10).len(), 4);
    assert_eq!(client.get_proposals(&ProposalStatus::Active, &0).len(), 0);
}
