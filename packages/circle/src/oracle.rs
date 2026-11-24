
// src/oracle.rs

//! Oracle integration for the Circle contract.
//!
//! The Circle contract reads a `yield_rate` (in basis points) from a
//! configurable oracle contract before each round resolves.
//!
//! Resolution strategy:
//! 1. If no primary oracle is configured, return zero yield.
//! 2. Attempt to retrieve the yield rate from the primary oracle.
//! 3. If the primary oracle fails, attempt the configured fallback.
//! 4. If the fallback succeeds, emit `OracleFallbackUsed`.
//! 5. If both oracle calls fail, return `CircleError::OracleUnavailable`.
//!
//! Storage keys:
//! - `DataKey::OracleContract`: Primary oracle address.
//! - `DataKey::FallbackOracle`: Fallback oracle address.
//!
//! IMPORTANT:
//! Authorization must be enforced by the contract entry points
//! that call the setter and clearer functions in this module.

use soroban_sdk::{symbol_short, Address, Env, Error, IntoVal};

use crate::types::{CircleError, DataKey, OracleFallbackUsed};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Oracle contract function name.
///
/// The oracle must expose:
/// `yield_rate(round: u32) -> i128`
const ORACLE_YIELD_FUNCTION: soroban_sdk::Symbol = symbol_short!("yld_rate");

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Attempts to retrieve a yield rate from an oracle contract.
///
/// Returns `Err(())` if:
/// - The oracle contract cannot be invoked.
/// - The oracle function does not exist.
/// - The oracle returns a contract error.
/// - The invocation fails.
///
/// Using `try_invoke_contract` prevents an oracle invocation failure from
/// immediately aborting the Circle contract execution.
fn call_oracle(
    env: &Env,
    oracle: &Address,
    round: u32,
) -> Result<i128, ()> {
    let args = (round,).into_val(env);

    env.try_invoke_contract::<i128, Error>(
        oracle,
        &ORACLE_YIELD_FUNCTION,
        args,
    )
    .map_err(|_| ())?
    .map_err(|_| ())
}

/// Loads the primary oracle address from instance storage.
fn load_primary_oracle(env: &Env) -> Option<Address> {
    env.storage()
        .instance()
        .get(&DataKey::OracleContract)
}

/// Loads the fallback oracle address from instance storage.
fn load_fallback_oracle(env: &Env) -> Option<Address> {
    env.storage()
        .instance()
        .get(&DataKey::FallbackOracle)
}

/// Emits an event indicating that the fallback oracle was used.
fn emit_fallback_event(
    env: &Env,
    round: u32,
    primary_oracle: Address,
    fallback_oracle: Address,
) {
    env.events().publish(
        (
            env.current_contract_address(),
            symbol_short!("orc_fall"),
        ),
        OracleFallbackUsed {
            round,
            primary_oracle,
            fallback_oracle,
        },
    );
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Resolves the yield rate for a given round.
///
/// Resolution order:
///
/// 1. No primary oracle configured:
///    Returns `Ok(0)`.
///
/// 2. Primary oracle succeeds:
///    Returns the primary oracle's yield rate.
///
/// 3. Primary oracle fails and fallback succeeds:
///    Emits `OracleFallbackUsed` and returns the fallback yield rate.
///
/// 4. Primary and fallback both fail, or no fallback is configured:
///    Returns `Err(CircleError::OracleUnavailable)`.
///
/// # Arguments
///
/// - `env`: Soroban execution environment.
/// - `round`: Current Circle round.
///
/// # Returns
///
/// The yield rate in basis points, or an oracle availability error.
pub fn get_yield_rate(
    env: &Env,
    round: u32,
) -> Result<i128, CircleError> {
    // No primary oracle means the Circle proceeds with zero yield.
    let primary_oracle = match load_primary_oracle(env) {
        Some(address) => address,
        None => return Ok(0),
    };

    // ---------------------------------------------------------
    // 1. Attempt primary oracle
    // ---------------------------------------------------------

    if let Ok(rate) = call_oracle(env, &primary_oracle, round) {
        return Ok(rate);
    }

    // ---------------------------------------------------------
    // 2. Primary failed: attempt fallback oracle
    // ---------------------------------------------------------

    let fallback_oracle = match load_fallback_oracle(env) {
        Some(address) => address,
        None => {
            return Err(CircleError::OracleUnavailable);
        }
    };

    // Avoid making the same failed contract call twice.
    if fallback_oracle == primary_oracle {
        return Err(CircleError::OracleUnavailable);
    }

    match call_oracle(env, &fallback_oracle, round) {
        Ok(rate) => {
            emit_fallback_event(
                env,
                round,
                primary_oracle,
                fallback_oracle,
            );

            Ok(rate)
        }

        Err(_) => Err(CircleError::OracleUnavailable),
    }
}

// ---------------------------------------------------------------------------
// Oracle Configuration
// ---------------------------------------------------------------------------

/// Configures the primary oracle.
///
/// IMPORTANT:
/// This function does not perform authorization itself.
/// The calling contract entry point must enforce administrator
/// authorization before calling it.
pub fn set_primary_oracle(
    env: &Env,
    oracle: &Address,
) {
    env.storage()
        .instance()
        .set(&DataKey::OracleContract, oracle);
}

/// Configures the fallback oracle.
///
/// IMPORTANT:
/// This function does not perform authorization itself.
/// The calling contract entry point must enforce administrator
/// authorization before calling it.
pub fn set_fallback_oracle(
    env: &Env,
    oracle: &Address,
) {
    env.storage()
        .instance()
        .set(&DataKey::FallbackOracle, oracle);
}

/// Removes the configured primary oracle.
///
/// The calling contract entry point must enforce administrator
/// authorization before calling this function.
pub fn clear_primary_oracle(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::OracleContract);
}

/// Removes the configured fallback oracle.
///
/// The calling contract entry point must enforce administrator
/// authorization before calling this function.
pub fn clear_fallback_oracle(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::FallbackOracle);
}

// ---------------------------------------------------------------------------
// Oracle Configuration Getters
// ---------------------------------------------------------------------------

/// Retrieves the currently configured primary oracle.
pub fn get_primary_oracle(env: &Env) -> Option<Address> {
    load_primary_oracle(env)
}

/// Retrieves the currently configured fallback oracle.
pub fn get_fallback_oracle(env: &Env) -> Option<Address> {
    load_fallback_oracle(env)
}

/// Returns whether a primary oracle has been configured.
pub fn has_primary_oracle(env: &Env) -> bool {
    env.storage()
        .instance()
        .has(&DataKey::OracleContract)
}

/// Returns whether a fallback oracle has been configured.
pub fn has_fallback_oracle(env: &Env) -> bool {
    env.storage()
        .instance()
        .has(&DataKey::FallbackOracle)
}
