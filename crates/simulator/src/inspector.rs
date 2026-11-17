//! [`SafetyInspector`]: records what a simulated transaction does, frame by frame.
//!
//! Every call / create frame collects its own events (native value moves, logs, contract
//! creations, self-destructs). When a frame ends successfully its events are appended to
//! the parent frame; when it reverts or halts they are dropped, exactly like the EVM drops
//! the frame's state changes. What survives the outermost frame is the transaction's
//! effect, in execution order.

use revm::{
    interpreter::{CallInputs, CallOutcome, CreateInputs, CreateOutcome},
    Inspector,
};
use std::collections::BTreeSet;
use xdevs_primitives::{safety::SMART_ACCOUNT_IMPL, Address, Log, U256};

/// Events recorded per simulation at most; more set [`SafetyInspector::truncated`].
pub const MAX_TRACE_EVENTS: usize = 10_000;

/// One effect of a simulated transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceEvent {
    /// Native value moved by a `CALL` / `CALLCODE` (incl. the transaction itself) or a
    /// `CREATE` endowment.
    Native {
        /// Payer.
        from: Address,
        /// Payee.
        to: Address,
        /// Amount (wei, non-zero).
        value: U256,
    },
    /// A log that survived (its frame and every parent frame succeeded).
    Log(Log),
    /// A contract was created.
    Created {
        /// New contract.
        address: Address,
        /// Deployer.
        creator: Address,
    },
    /// `SELFDESTRUCT` moved a balance.
    SelfDestruct {
        /// Self-destructing contract.
        contract: Address,
        /// Receiver of its balance.
        beneficiary: Address,
        /// Amount (wei, non-zero).
        value: U256,
    },
}

/// revm inspector behind `xdevs_simulateTransaction`. It never changes the execution
/// (every hook returns `None` / leaves the outcome untouched).
#[derive(Clone, Debug, Default)]
pub struct SafetyInspector {
    /// Open frames, innermost last.
    frames: Vec<Vec<TraceEvent>>,
    /// Effects of finished outermost frames.
    done: Vec<TraceEvent>,
    recorded: usize,
    truncated: bool,
    smart_accounts: BTreeSet<Address>,
}

impl SafetyInspector {
    /// Empty inspector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Surviving effects in execution order (complete once the transaction finished).
    pub fn events(&self) -> &[TraceEvent] {
        &self.done
    }

    /// Consume the inspector, returning the surviving effects.
    pub fn into_events(self) -> Vec<TraceEvent> {
        self.done
    }

    /// `true` if more than [`MAX_TRACE_EVENTS`] events were seen (later ones are missing).
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Addresses that ran the xDEVS Smart Account implementation (`0x…1005`) as their own
    /// code: ERC-1967 proxies delegating to it, and EIP-7702 delegated accounts when the
    /// EVM reports the implementation as the code address.
    pub const fn smart_accounts(&self) -> &BTreeSet<Address> {
        &self.smart_accounts
    }

    fn push(&mut self, event: TraceEvent) {
        if self.recorded >= MAX_TRACE_EVENTS {
            self.truncated = true;
            return;
        }
        self.recorded += 1;
        match self.frames.last_mut() {
            Some(frame) => frame.push(event),
            None => self.done.push(event),
        }
    }

    fn close(&mut self, success: bool) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        if !success {
            return;
        }
        match self.frames.last_mut() {
            Some(parent) => parent.extend(frame),
            None => self.done.extend(frame),
        }
    }
}

impl<CTX> Inspector<CTX> for SafetyInspector {
    fn log(&mut self, _context: &mut CTX, log: Log) {
        self.push(TraceEvent::Log(log));
    }

    fn call(&mut self, _context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.bytecode_address == SMART_ACCOUNT_IMPL
            && inputs.target_address != SMART_ACCOUNT_IMPL
        {
            self.smart_accounts.insert(inputs.target_address);
        }
        self.frames.push(Vec::new());
        if let Some(value) = inputs.transfer_value().filter(|v| !v.is_zero()) {
            self.push(TraceEvent::Native {
                from: inputs.transfer_from(),
                to: inputs.transfer_to(),
                value,
            });
        }
        None
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, outcome: &mut CallOutcome) {
        self.close(outcome.result.is_ok());
    }

    fn create(&mut self, _context: &mut CTX, _inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        self.frames.push(Vec::new());
        None
    }

    fn create_end(
        &mut self,
        _context: &mut CTX,
        inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        let success = outcome.result.is_ok();
        if let (true, Some(address), Some(frame)) =
            (success, outcome.address, self.frames.last_mut())
        {
            // The creation and its endowment come before the constructor's effects.
            let mut head = vec![TraceEvent::Created {
                address,
                creator: inputs.caller(),
            }];
            if !inputs.value().is_zero() {
                head.push(TraceEvent::Native {
                    from: inputs.caller(),
                    to: address,
                    value: inputs.value(),
                });
            }
            frame.splice(0..0, head);
        }
        self.close(success);
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        if !value.is_zero() {
            self.push(TraceEvent::SelfDestruct {
                contract,
                beneficiary: target,
                value,
            });
        }
    }
}
