// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {CommonBase} from "forge-std/Base.sol";
import {StdCheats} from "forge-std/StdCheats.sol";
import {StdUtils} from "forge-std/StdUtils.sol";

import {AccountTestBase, IXAccount} from "./utils/AccountTestBase.sol";
import {
    Call,
    InitParams,
    Signer,
    SignerType,
    VaultPolicy,
    SessionKeyConfig,
    SessionPermission,
    SessionTokenCap
} from "../../src/accounts/libraries/AccountTypes.sol";

// ================================================================================================================
// Guardian Vault: queued actions & delayed policy changes
// ================================================================================================================

contract VaultHandler is CommonBase, StdCheats, StdUtils {
    IXAccount public immutable acct;
    address public immutable owner;
    address public immutable guardian;
    address public immutable recipient;

    struct Ghost {
        uint64 executeAfter;
        uint256 amount; // native transfer amount (0 for policy changes)
        bool isPolicy;
        bool cancelled;
        bool executed;
    }

    mapping(uint256 => Ghost) public ghosts;
    mapping(uint256 => VaultPolicy) internal policyOf;
    uint256[] public ids;

    // current (effective) delay tracked independently of the contract
    uint64 public ghostDelay;
    VaultPolicy internal expected;

    uint256 public expectedRecipientBalance;
    uint256 public earlyExecutions;
    uint256 public cancelledExecutions;
    uint256 public doubleExecutions;
    uint256 public executeAfterMismatches;
    uint256 public successfulExecutions;
    uint256 public successfulCancels;

    constructor(
        IXAccount acct_,
        address owner_,
        address guardian_,
        address recipient_,
        VaultPolicy memory initial
    ) {
        acct = acct_;
        owner = owner_;
        guardian = guardian_;
        recipient = recipient_;
        expected = initial;
        ghostDelay = initial.delay;
    }

    function expectedPolicy() external view returns (VaultPolicy memory) {
        return expected;
    }

    function idCount() external view returns (uint256) {
        return ids.length;
    }

    function _record(uint256 id, uint256 amount, bool isPolicy) internal {
        uint64 executeAfter = uint64(block.timestamp) + ghostDelay;
        (uint64 onChain,,,) = acct.getAction(id);
        if (onChain != executeAfter) ++executeAfterMismatches;
        ghosts[id] = Ghost(executeAfter, amount, isPolicy, false, false);
        ids.push(id);
    }

    // ---- actions ----

    function queueTransfer(uint256 amount) external {
        amount = bound(amount, 0.01 ether, 3 ether);
        vm.prank(owner);
        uint256 id = acct.execute(recipient, amount, "");
        if (id == 0) {
            expectedRecipientBalance += amount; // executed instantly (within threshold / daily limit)
        } else {
            _record(id, amount, false);
        }
    }

    function queuePolicyChange(uint256 thresholdSeed, uint256 delaySeed) external {
        VaultPolicy memory p = expected;
        p.nativeThreshold = uint128(bound(thresholdSeed, 0, 2 ether));
        p.delay = uint64(bound(delaySeed, 1 hours, 7 days));
        vm.prank(owner);
        acct.setVaultPolicy(p);
        uint256 id = acct.lastActionId();
        policyOf[id] = p;
        _record(id, 0, true);
    }

    function cancel(uint256 idSeed, bool byGuardian) external {
        if (ids.length == 0) return;
        uint256 id = ids[idSeed % ids.length];
        vm.prank(byGuardian ? guardian : owner);
        try acct.cancel(id) {
            Ghost storage g = ghosts[id];
            if (!g.executed) g.cancelled = true;
            ++successfulCancels;
        } catch {}
    }

    function execute(uint256 idSeed) external {
        if (ids.length == 0) return;
        _tryExecute(ids[idSeed % ids.length]);
    }

    /// @dev Biases the campaign towards successful executions (oldest action not yet executed / cancelled).
    function executeOldestPending() external {
        for (uint256 i; i < ids.length; ++i) {
            Ghost storage g = ghosts[ids[i]];
            if (!g.executed && !g.cancelled) {
                _tryExecute(ids[i]);
                return;
            }
        }
    }

    function _tryExecute(uint256 id) internal {
        try acct.execute(id) {
            Ghost storage g = ghosts[id];
            if (block.timestamp < g.executeAfter) ++earlyExecutions;
            if (g.cancelled) ++cancelledExecutions;
            if (g.executed) ++doubleExecutions;
            g.executed = true;
            ++successfulExecutions;
            if (g.isPolicy) {
                expected = policyOf[id];
                ghostDelay = expected.delay;
            } else {
                expectedRecipientBalance += g.amount;
            }
        } catch {}
    }

    function warp(uint256 dt) external {
        vm.warp(block.timestamp + bound(dt, 0, 3 days));
    }
}

contract VaultInvariantTest is AccountTestBase {
    VaultHandler internal handler;
    IXAccount internal acct;
    address internal recipient = makeAddr("recipient");

    function setUp() public override {
        super.setUp();
        acct = _createAccount(_params(_one(_ecdsaSigner(owner)), true), 0);
        vm.deal(address(acct), 1_000_000 ether);
        handler = new VaultHandler(acct, owner, guardian1, recipient, acct.vaultPolicy());
        targetContract(address(handler));
    }

    /// @notice A queued vault action can never execute before `executeAfter`.
    function invariant_noEarlyExecution() public view {
        assertEq(handler.earlyExecutions(), 0);
        assertEq(handler.executeAfterMismatches(), 0);
    }

    /// @notice A cancelled action can never execute; an action never executes twice.
    function invariant_cancelledNeverExecutes() public view {
        assertEq(handler.cancelledExecutions(), 0);
        assertEq(handler.doubleExecutions(), 0);
    }

    /// @notice Policy changes never take effect before their delay: the on-chain policy always equals the last
    ///         policy whose queued change was executed (after its delay).
    function invariant_policyOnlyChangesThroughMaturedActions() public view {
        VaultPolicy memory onChain = acct.vaultPolicy();
        VaultPolicy memory exp = handler.expectedPolicy();
        assertEq(onChain.nativeThreshold, exp.nativeThreshold);
        assertEq(onChain.delay, exp.delay);
        assertEq(onChain.dailyLimit, exp.dailyLimit);
        assertTrue(onChain.enabled);
    }

    /// @notice Value only leaves the account through instant (policy-compliant) or matured, uncancelled actions.
    function invariant_recipientBalanceMatchesGhost() public view {
        assertEq(recipient.balance, handler.expectedRecipientBalance());
    }
}

// ================================================================================================================
// Session keys: cap & expiry
// ================================================================================================================

contract SessionHandler is CommonBase, StdCheats, StdUtils {
    IXAccount public immutable acct;
    address public immutable key;
    address public immutable shop;
    uint48 public immutable validUntil;

    uint256 public totalSpent;
    uint256 public lateSuccesses;

    constructor(IXAccount acct_, address key_, address shop_, uint48 validUntil_) {
        acct = acct_;
        key = key_;
        shop = shop_;
        validUntil = validUntil_;
    }

    function spend(uint256 amount) external {
        amount = bound(amount, 0, 1.5 ether);
        Call[] memory calls = new Call[](1);
        calls[0] = Call(shop, amount, "");
        vm.prank(key);
        try acct.executeWithSession(calls) {
            totalSpent += amount;
            if (block.timestamp > validUntil) ++lateSuccesses;
        } catch {}
    }

    function spendBatch(uint256 a, uint256 b) external {
        Call[] memory calls = new Call[](2);
        calls[0] = Call(shop, bound(a, 0, 1 ether), "");
        calls[1] = Call(shop, bound(b, 0, 1 ether), "");
        vm.prank(key);
        try acct.executeWithSession(calls) {
            totalSpent += calls[0].value + calls[1].value;
            if (block.timestamp > validUntil) ++lateSuccesses;
        } catch {}
    }

    function warp(uint256 dt) external {
        vm.warp(block.timestamp + bound(dt, 0, 12 hours));
    }
}

contract SessionInvariantTest is AccountTestBase {
    SessionHandler internal handler;
    IXAccount internal acct;
    address internal shop = makeAddr("shop");
    uint128 internal constant CAP = 3 ether;

    function setUp() public override {
        super.setUp();
        acct = _createAccount(_params(_one(_ecdsaSigner(owner)), false), 0);
        address key = makeAddr("session");
        SessionKeyConfig memory cfg;
        cfg.key = key;
        cfg.validAfter = uint48(block.timestamp);
        cfg.validUntil = uint48(block.timestamp + 2 days);
        cfg.nativeCap = CAP;
        cfg.permissions = new SessionPermission[](1);
        cfg.permissions[0] = SessionPermission(shop, bytes4(0));
        cfg.tokenCaps = new SessionTokenCap[](0);
        vm.prank(owner);
        acct.addSessionKey(cfg);
        handler = new SessionHandler(acct, key, shop, cfg.validUntil);
        targetContract(address(handler));
    }

    /// @notice A session key can never exceed its cap.
    function invariant_sessionNeverExceedsCap() public view {
        assertLe(shop.balance, CAP);
        assertEq(shop.balance, handler.totalSpent());
        (,,,,, uint128 nativeSpent) = acct.getSession(handler.key());
        assertEq(nativeSpent, shop.balance);
    }

    /// @notice A session key can never act after expiry.
    function invariant_sessionNeverActsAfterExpiry() public view {
        assertEq(handler.lateSuccesses(), 0);
    }
}

// ================================================================================================================
// Social recovery: threshold & delay
// ================================================================================================================

contract RecoveryHandler is CommonBase, StdCheats, StdUtils {
    IXAccount public immutable acct;
    address public immutable owner;
    address[3] internal guardians;
    Signer[][2] internal proposals;
    uint64 public immutable delay;

    // ghost: approvals of the current round (reset on cancel / execution)
    mapping(uint256 round => mapping(uint256 proposal => mapping(address => bool))) internal voted;
    mapping(uint256 round => mapping(uint256 proposal => uint256)) internal votes;
    mapping(uint256 round => mapping(uint256 proposal => uint256)) internal readyAt;
    uint256 public round;

    uint256 public invalidRecoveries;
    uint256 public recoveries;
    uint256 public lastRecovered = type(uint256).max;

    constructor(
        IXAccount acct_,
        address owner_,
        address[3] memory guardians_,
        Signer[] memory a,
        Signer[] memory b
    ) {
        acct = acct_;
        owner = owner_;
        guardians = guardians_;
        for (uint256 i; i < a.length; ++i) {
            proposals[0].push(a[i]);
        }
        for (uint256 i; i < b.length; ++i) {
            proposals[1].push(b[i]);
        }
        delay = acct_.recoveryDelay();
    }

    function proposal(uint256 p) external view returns (Signer[] memory) {
        return proposals[p];
    }

    function approve(uint256 gSeed, uint256 pSeed) external {
        _approve(guardians[gSeed % 3], pSeed % 2);
    }

    /// @dev Biases the campaign towards reaching the threshold.
    function approveTwo(uint256 gSeed, uint256 pSeed) external {
        _approve(guardians[gSeed % 3], pSeed % 2);
        _approve(guardians[(gSeed + 1) % 3], pSeed % 2);
    }

    function _approve(address g, uint256 p) internal {
        vm.prank(g);
        try acct.approveRecovery(proposals[p]) {
            if (!voted[round][p][g]) {
                voted[round][p][g] = true;
                if (++votes[round][p] == 2) readyAt[round][p] = block.timestamp + delay;
            }
        } catch {}
    }

    function execute(uint256 pSeed) external {
        uint256 p = pSeed % 2;
        try acct.executeRecovery(proposals[p]) {
            ++recoveries;
            if (votes[round][p] < 2 || readyAt[round][p] == 0 || block.timestamp < readyAt[round][p]) {
                ++invalidRecoveries;
            }
            lastRecovered = p;
            ++round;
        } catch {}
    }

    function ownerCancel() external {
        if (lastRecovered != type(uint256).max) return; // the original owner key is gone after a recovery
        vm.prank(owner);
        try acct.cancelRecovery() {
            ++round;
        } catch {}
    }

    function warp(uint256 dt) external {
        vm.warp(block.timestamp + bound(dt, 0, 3 days));
    }
}

contract RecoveryInvariantTest is AccountTestBase {
    RecoveryHandler internal handler;
    IXAccount internal acct;
    address internal newOwner1 = makeAddr("newOwner1");
    address internal newOwner2 = makeAddr("newOwner2");

    function setUp() public override {
        super.setUp();
        acct = _vaultAccount();
        handler = new RecoveryHandler(
            acct,
            owner,
            [guardian1, guardian2, guardian3],
            _one(_ecdsaSigner(newOwner1)),
            _one(_ecdsaSigner(newOwner2))
        );
        targetContract(address(handler));
    }

    /// @notice Recovery can never complete before the delay or without the guardian threshold.
    function invariant_recoveryRequiresThresholdAndDelay() public view {
        assertEq(handler.invalidRecoveries(), 0);
    }

    /// @notice The signer set only ever changes through a valid recovery.
    function invariant_signerSetConsistent() public view {
        Signer[] memory s = acct.getSigners();
        assertEq(s.length, 1);
        uint256 last = handler.lastRecovered();
        address expectedSigner = last == type(uint256).max ? owner : (last == 0 ? newOwner1 : newOwner2);
        assertEq(s[0].x, bytes32(uint256(uint160(expectedSigner))));
    }
}
