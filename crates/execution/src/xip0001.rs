//! XIP-0001 (EOA lock) enforcement inside the executor (`docs/specs/xip-0001-eoa-lock.md`).
//!
//! * Rules (a) and (c) run before a transaction is handed to revm, against the overlay
//!   state, which at that point is exactly `S₀(tx)` (the previous transaction is
//!   committed, this one is not).
//! * Rule (b) uses the **pre-filter** option of spec §9: every EIP-7702 authorization is
//!   recovered here; when its authority is `LOCKED` or `SEALED` at `S₀(tx)` the tuple is
//!   passed to revm with its authority marked unrecoverable
//!   ([`RecoveredAuthority::Invalid`]), so revm skips it at EIP-7702 step 3, *before* step
//!   4 (warming) — no code change, no nonce bump, no refund, not warmed. The list length,
//!   and therefore the intrinsic gas, is unchanged.

use crate::db::{StateDbError, StateOverlay};
use revm::context_interface::{
    either::Either,
    transaction::{RecoveredAuthority, RecoveredAuthorization, SignedAuthorization},
};
use xdevs_primitives::{
    xip0001::{eoa_lock_status, LOCK_SLOT},
    Address, EoaLockStatus,
};

/// revm's authorization list element.
pub(crate) type AuthItem = Either<SignedAuthorization, RecoveredAuthorization>;

/// `status(address, S)` read from the overlay (one account read; one slot read only for
/// accounts delegated to the xDEVS Smart Account).
pub(crate) fn status(
    db: &mut StateOverlay<'_>,
    address: Address,
) -> Result<EoaLockStatus, StateDbError> {
    let code_hash = db.account(address)?.map(|a| a.code_hash);
    eoa_lock_status(code_hash, || db.slot(address, LOCK_SLOT))
}

/// Rule (b): recover every authorization and neutralise the ones whose authority is
/// locked or sealed at the current overlay state (`S₀` of the transaction).
pub(crate) fn filter_authorizations(
    db: &mut StateOverlay<'_>,
    list: &mut [AuthItem],
) -> Result<(), StateDbError> {
    for item in list.iter_mut() {
        let recovered = match item {
            Either::Left(signed) => signed.clone().into_recovered(),
            Either::Right(recovered) => recovered.clone(),
        };
        let skip = match recovered.authority() {
            Some(authority) => status(db, authority)?.is_restricted(),
            None => false,
        };
        *item = Either::Right(if skip {
            let (inner, _) = recovered.into_parts();
            RecoveredAuthorization::new_unchecked(inner, RecoveredAuthority::Invalid)
        } else {
            recovered
        });
    }
    Ok(())
}
