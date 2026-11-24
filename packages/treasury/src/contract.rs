//! Treasury contract — all public handler functions follow the **uniform
//! Soroban-native `&Env`-based API pattern**: every function takes `env: &Env`
//! as its first parameter and returns `Result<_, TreasuryError>` (or a plain
//! value for pure reads).  There are no `ExecCtx`, `QueryCtx`, or
//! `MessageInfo` parameters; those belong to CosmWasm and must never appear
//! here.  All mutation functions perform access-control checks first, before
//! touching storage.

use soroban_sdk::{symbol_short, token, Address, Env, Vec};

use crate::types::*;
use common::pause;

fn load_admin(env: &Env) -> Result<Address, TreasuryError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(TreasuryError::NotInitialized)
}

fn require_admin(env: &Env, admin: &Address) -> Result<(), TreasuryError> {
    let stored = load_admin(env)?;
    if admin != &stored {
        return Err(TreasuryError::Unauthorized);
    }
    admin.require_auth();
    Ok(())
}

fn token_client<'a>(env: &'a Env) -> Result<token::Client<'a>, TreasuryError> {
    let token_addr: Address = env
        .storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(TreasuryError::NotInitialized)?;
    Ok(token::Client::new(env, &token_addr))
}

fn read_balance(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::Balance).unwrap_or(0)
}

fn debit_balance(
    env: &Env,
    amount: i128,
    underflow_err: TreasuryError,
) -> Result<i128, TreasuryError> {
    let current = read_balance(env);
    if current < amount {
        return Err(underflow_err);
    }
    let updated = current.checked_sub(amount).ok_or(underflow_err)?;
    env.storage().instance().set(&DataKey::Balance, &updated);
    Ok(updated)
}

/// Initializes the treasury contract with an admin and token address.
pub fn init(env: &Env, admin: &Address, token: &Address) -> Result<(), TreasuryError> {
    admin.require_auth();
 fix/governance-quorum-init-guards
    if env.storage().instance().has(&DataKey::Admin) {
        return Err(TreasuryError::AlreadyInitialized);
    }


 master
    env.storage().instance().set(&DataKey::Admin, admin);
    env.storage().instance().set(&DataKey::Token, token);
    env.storage().instance().set(&DataKey::Balance, &0i128);
    env.storage()
        .persistent()
        .set(&DataKey::Deposits, &Vec::<Deposit>::new(env));
    env.storage()
        .persistent()
        .set(&DataKey::Withdrawals, &Vec::<Withdrawal>::new(env));
    Ok(())
}

/// Deposits fees from a circle into the treasury.
pub fn deposit(
    env: &Env,
    from: &Address,
    amount: i128,
    circle_id: &Address,
) -> Result<(), TreasuryError> {
    pause::when_not_paused(env).map_err(|_| TreasuryError::ContractPaused)?;
    if from != circle_id {
        return Err(TreasuryError::Unauthorized);
    }
    from.require_auth();

    if amount <= 0 {
        return Err(TreasuryError::InvalidAmount);
    }

    let client = token_client(env)?;
    let current_bal = read_balance(env);
    let next_bal = current_bal
        .checked_add(amount)
        .ok_or(TreasuryError::InvalidAmount)?;

    let treasury = env.current_contract_address();
    client.transfer(from, &treasury, &amount);

    env.storage().instance().set(&DataKey::Balance, &next_bal);

    let mut deps: Vec<Deposit> = env
        .storage()
        .persistent()
        .get(&DataKey::Deposits)
        .unwrap_or_else(|| Vec::new(env));
    deps.push_back(Deposit {
        from: from.clone(),
        amount,
        circle_id: circle_id.clone(),
        timestamp: env.ledger().timestamp(),
    });
    env.storage().persistent().set(&DataKey::Deposits, &deps);

    env.events().publish(
        (treasury, symbol_short!("deposit")),
        FeeDeposited {
            from: from.clone(),
            amount,
            circle_id: circle_id.clone(),
        },
    );
    Ok(())
}

/// Withdraws funds from the treasury to a recipient address.
pub fn withdraw(
    env: &Env,
    admin: &Address,
    to: &Address,
    amount: i128,
) -> Result<(), TreasuryError> {
    pause::when_not_paused(env).map_err(|_| TreasuryError::ContractPaused)?;
    require_admin(env, admin)?;
    let _guard =
        common::reentrancy::ReentrancyGuard::new(env).map_err(|_| TreasuryError::Unauthorized)?;

    if amount <= 0 {
        return Err(TreasuryError::InvalidAmount);
    }

    let client = token_client(env)?;
    let current_bal = read_balance(env);
    if current_bal < amount {
        return Err(TreasuryError::InsufficientBalance);
    }

    let treasury = env.current_contract_address();
    client.transfer(&treasury, to, &amount);
    debit_balance(env, amount, TreasuryError::InsufficientBalance)?;

    let mut wds: Vec<Withdrawal> = env
        .storage()
        .persistent()
        .get(&DataKey::Withdrawals)
        .unwrap_or_else(|| Vec::new(env));
    wds.push_back(Withdrawal {
        admin: admin.clone(),
        to: to.clone(),
        amount,
        timestamp: env.ledger().timestamp(),
    });
    env.storage().persistent().set(&DataKey::Withdrawals, &wds);

    env.events().publish(
        (treasury, symbol_short!("withdraw")),
        FundsWithdrawn {
            to: to.clone(),
            amount,
        },
    );
    Ok(())
}

/// Returns the current balance of the treasury.
pub fn get_balance(env: &Env) -> i128 {
    read_balance(env)
}

/// Returns all deposit records.
pub fn get_deposits(env: &Env) -> Vec<Deposit> {
    env.storage()
        .persistent()
        .get(&DataKey::Deposits)
        .unwrap_or_else(|| Vec::new(env))
}

/// Rescues tokens that were sent to the treasury by mistake.
pub fn rescue_tokens(
    env: &Env,
    admin: &Address,
    to: &Address,
    token: &Address,
    amount: i128,
) -> Result<(), TreasuryError> {
    require_admin(env, admin)?;
    let _guard =
        common::reentrancy::ReentrancyGuard::new(env).map_err(|_| TreasuryError::Unauthorized)?;

    if !pause::is_paused(env) {
        return Err(TreasuryError::ContractNotPaused);
    }
    if amount <= 0 {
        return Err(TreasuryError::InvalidAmount);
    }

    let treasury_token: Address = env
        .storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(TreasuryError::NotInitialized)?;
    let is_treasury_token = treasury_token == *token;
    if is_treasury_token && read_balance(env) < amount {
        return Err(TreasuryError::InsufficientBalance);
    }

    let treasury = env.current_contract_address();
    let rescue_client = token::Client::new(env, token);
    rescue_client.transfer(&treasury, to, &amount);

    if is_treasury_token {
        debit_balance(env, amount, TreasuryError::InsufficientBalance)?;
    }

    env.events().publish(
        (treasury, symbol_short!("rescue")),
        TokensRescued {
            to: to.clone(),
            token: token.clone(),
            amount,
        },
    );
    Ok(())
}

/// Pauses the treasury, preventing deposits and withdrawals.
pub fn pause(env: &Env, a: &Address) -> Result<(), TreasuryError> {
    let stored_admin = load_admin(env)?;
    common::access::require_self_or_admin(env, a, &stored_admin)
        .map_err(|_| TreasuryError::Unauthorized)?;
    pause::pause(env, a).map_err(|_| TreasuryError::ContractPaused)
}

/// Unpauses the treasury, allowing deposits and withdrawals to resume.
pub fn unpause(env: &Env, a: &Address) -> Result<(), TreasuryError> {
    let stored_admin = load_admin(env)?;
    common::access::require_self_or_admin(env, a, &stored_admin)
        .map_err(|_| TreasuryError::Unauthorized)?;
    pause::unpause(env, a).map_err(|_| TreasuryError::ContractPaused)
}