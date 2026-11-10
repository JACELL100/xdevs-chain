// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {GenesisInitializable} from "../common/GenesisInitializable.sol";
import {XNSLabel} from "./XNSLabel.sol";
import {
    IENSRegistry,
    IAddrResolver,
    IAddressResolver,
    ITextResolver,
    INameResolver,
    IVersionableResolver,
    IXNSRegistrar
} from "./IXNS.sol";

/// @title XNSRegistry - xDEVS Name Service (`alice.xdevs`)
/// @author THE xDEVS
/// @notice One predeploy (at `Predeploys.XNS_REGISTRY`, 0x…1004) that bundles:
///         1. an ENS-compatible registry (EIP-137 interface, namehash nodes),
///         2. the `.xdevs` registrar (1-year renewable registrations, 90-day grace, fee -> Treasury),
///         3. the default public resolver (addr / multicoin addr / text / name / clearRecords), and
///         4. the `addr.reverse` reverse registrar (primary names).
///         `resolver(node)` returns this same address for every name registered through the registrar,
///         so ENS libraries only need the registry address (`ensAddress` in ethers) to resolve names.
/// @dev    Safety-driven differences from ENS (all documented in docs/specs/xns-and-approvals.md):
///         - Labels are strictly `[a-z0-9-]`, 3..63 bytes; confusable skeletons are unique (see XNSLabel).
///         - An expired name (or any subname of it) stops resolving immediately: owner(), resolver(),
///           ttl() and every resolver record read as empty. Nothing resolves during the grace period.
///         - Re-registration by anyone after grace wipes all previous records and subnames (generation
///           bump), so a new owner never inherits the previous owner's addresses.
///         - The root and the `xdevs` / `reverse` / `addr.reverse` nodes are owned by this contract
///           forever; governance can only change the fee, treasury and reserved-label list.
///         - `setName(string)` (reverse) only accepts a name that forward-resolves to the caller, and
///           `primaryName(addr)` re-checks the forward record at read time.
contract XNSRegistry is
    GenesisInitializable,
    IENSRegistry,
    IAddrResolver,
    IAddressResolver,
    ITextResolver,
    INameResolver,
    IVersionableResolver,
    IXNSRegistrar
{
    // ------------------------------------------------------------------ constants

    /// @notice namehash("xdevs").
    bytes32 public constant TLD_NODE = 0xa058e3bd8a205b95bca4815c553cac26b3228e39fce9d34f8417f3665c626edd;
    /// @notice keccak256("xdevs").
    bytes32 public constant TLD_LABELHASH =
        0x4fe9f039dd86bf3b3f58d219c03685d6321ec5aa93251afde3eea66315a4b56d;
    /// @notice namehash("reverse").
    bytes32 public constant REVERSE_NODE = 0xa097f6721ce401e757d1223a763fef49b8b5f90bb18567ddb86fd205dff71d34;
    /// @notice namehash("addr.reverse").
    bytes32 public constant ADDR_REVERSE_NODE =
        0x91d1777781884d03a6757a803996e38de2a42967fb37eeaca72729271025a9e2;

    uint256 public constant MIN_REGISTRATION_DURATION = 365 days;
    /// @notice A name can never be paid up more than this far into the future.
    uint256 public constant MAX_REGISTRATION_HORIZON = 10 * 365 days;
    uint256 public constant GRACE_PERIOD = 90 days;
    /// @notice SLIP-44 coin type used by ENSIP-1 `addr(bytes32)`.
    uint256 public constant COIN_TYPE_ETH = 60;

    // ------------------------------------------------------------------ errors

    error Unauthorized(bytes32 node, address caller);
    error NotGovernance(address caller);
    error ZeroAddress();
    error NameNotAvailable(bytes32 node, uint256 expires);
    error NameReserved(string label);
    error ConfusableWithExisting(string label, bytes32 existingNode);
    error InvalidDuration(uint256 duration);
    error WrongFee(uint256 expected, uint256 sent);
    error FeeTransferFailed();
    error NameNotRenewable(bytes32 node);
    error ReverseNameMismatch(string name, address resolvesTo);
    error InvalidAddressLength(uint256 coinType, uint256 length);

    // ------------------------------------------------------------------ storage

    struct Record {
        address owner;
        uint64 ttl;
        uint32 generation; // generation of `authority` this record was written in
        address resolver;
        bytes32 authority; // the governing `.xdevs` 2LD node, or 0 for protocol-owned trees
    }

    address public governance;
    address public treasury;
    /// @notice Registration fee in tXDS/XDS wei per 365 days (0 on testnet).
    uint256 public feePerYear;

    mapping(bytes32 node => Record) private _records;
    mapping(address owner => mapping(address operator => bool)) private _operators;
    /// @dev 2LD node => expiry timestamp.
    mapping(bytes32 node => uint256) private _expiries;
    /// @dev 2LD node => current registration generation (bumped on every fresh registration).
    mapping(bytes32 node => uint32) private _generations;
    /// @notice Confusable skeleton hash => the 2LD node currently holding it.
    mapping(bytes32 skeletonHash => bytes32 node) public skeletonHolder;
    /// @notice labelhash => reserved (registrable only by governance).
    mapping(bytes32 labelhash => bool) public reserved;

    // resolver
    mapping(bytes32 node => uint64) public recordVersions;
    mapping(bytes32 rkey => mapping(uint256 coinType => bytes)) private _addrs;
    mapping(bytes32 rkey => mapping(string key => string)) private _texts;
    mapping(bytes32 rkey => string) private _names;

    // ------------------------------------------------------------------ init

    /// @notice One-time setup (genesis builder via SYSTEM_ADDRESS, or the deployer).
    /// @param governance_ Address allowed to change fee / treasury / reserved labels (e.g. Predeploys.GOVERNANCE).
    /// @param treasury_ Recipient of registration fees (e.g. Predeploys.TREASURY).
    /// @param feePerYear_ Fee per 365 days in wei of the native coin (0 on testnet).
    /// @param reservedLabels Labels only governance may register (brand / impersonation protection).
    function initialize(
        address governance_,
        address treasury_,
        uint256 feePerYear_,
        string[] calldata reservedLabels
    ) external initializer {
        if (governance_ == address(0) || treasury_ == address(0)) revert ZeroAddress();
        governance = governance_;
        treasury = treasury_;
        feePerYear = feePerYear_;

        // Protocol-owned skeleton: root -> xdevs, root -> reverse -> addr. Authority 0 = never expires.
        _records[bytes32(0)].owner = address(this);
        _createProtocolNode(bytes32(0), TLD_LABELHASH, TLD_NODE);
        _createProtocolNode(bytes32(0), keccak256("reverse"), REVERSE_NODE);
        _createProtocolNode(REVERSE_NODE, keccak256("addr"), ADDR_REVERSE_NODE);

        for (uint256 i; i < reservedLabels.length; ++i) {
            _setReserved(reservedLabels[i], true);
        }
        emit Initialized(governance_, treasury_, feePerYear_);
    }

    function _createProtocolNode(bytes32 parent, bytes32 label, bytes32 child) private {
        Record storage r = _records[child];
        r.owner = address(this);
        r.resolver = address(this);
        emit NewOwner(parent, label, address(this));
        emit NewResolver(child, address(this));
    }

    // ------------------------------------------------------------------ modifiers / internal views

    modifier onlyGovernance() {
        if (msg.sender != governance) revert NotGovernance(msg.sender);
        _;
    }

    modifier authorised(bytes32 node_) {
        if (!_isAuthorised(node_, msg.sender)) revert Unauthorized(node_, msg.sender);
        _;
    }

    /// @dev A record is live if it is protocol-owned (authority 0) or belongs to the current, unexpired
    ///      generation of its governing 2LD.
    function _live(bytes32 node_) internal view returns (bool) {
        Record storage r = _records[node_];
        bytes32 a = r.authority;
        if (a == bytes32(0)) return true;
        return block.timestamp < _expiries[a] && r.generation == _generations[a];
    }

    function _isAuthorised(bytes32 node_, address caller) internal view returns (bool) {
        if (!_live(node_)) return false;
        address o = _records[node_].owner;
        return o != address(0) && (o == caller || _operators[o][caller]);
    }

    /// @dev Storage key for resolver records: changes on re-registration (generation) and clearRecords.
    function _rkey(bytes32 node_) internal view returns (bytes32) {
        return keccak256(abi.encode(node_, _records[node_].generation, recordVersions[node_]));
    }

    // ------------------------------------------------------------------ ENS registry (EIP-137)

    function setRecord(bytes32 node_, address owner_, address resolver_, uint64 ttl_)
        external
        authorised(node_)
    {
        _setResolverAndTTL(node_, resolver_, ttl_);
        _setOwner(node_, owner_);
    }

    function setSubnodeRecord(bytes32 node_, bytes32 label, address owner_, address resolver_, uint64 ttl_)
        external
        authorised(node_)
    {
        bytes32 sub = _setSubnodeOwner(node_, label, owner_);
        _setResolverAndTTL(sub, resolver_, ttl_);
    }

    function setSubnodeOwner(bytes32 node_, bytes32 label, address owner_)
        external
        authorised(node_)
        returns (bytes32)
    {
        return _setSubnodeOwner(node_, label, owner_);
    }

    function setResolver(bytes32 node_, address resolver_) external authorised(node_) {
        _records[node_].resolver = resolver_;
        emit NewResolver(node_, resolver_);
    }

    function setOwner(bytes32 node_, address owner_) external authorised(node_) {
        _setOwner(node_, owner_);
    }

    function setTTL(bytes32 node_, uint64 ttl_) external authorised(node_) {
        _records[node_].ttl = ttl_;
        emit NewTTL(node_, ttl_);
    }

    function setApprovalForAll(address operator, bool approved) external {
        _operators[msg.sender][operator] = approved;
        emit ApprovalForAll(msg.sender, operator, approved);
    }

    /// @notice Owner of `node_`; address(0) if it does not exist or its `.xdevs` name has expired.
    function owner(bytes32 node_) public view returns (address) {
        return _live(node_) ? _records[node_].owner : address(0);
    }

    /// @notice Resolver of `node_`; address(0) if it does not exist or its `.xdevs` name has expired.
    function resolver(bytes32 node_) external view returns (address) {
        return _live(node_) ? _records[node_].resolver : address(0);
    }

    function ttl(bytes32 node_) external view returns (uint64) {
        return _live(node_) ? _records[node_].ttl : 0;
    }

    function recordExists(bytes32 node_) external view returns (bool) {
        return _live(node_) && _records[node_].owner != address(0);
    }

    function isApprovedForAll(address owner_, address operator) external view returns (bool) {
        return _operators[owner_][operator];
    }

    function _setOwner(bytes32 node_, address owner_) private {
        _records[node_].owner = owner_;
        emit Transfer(node_, owner_);
    }

    function _setResolverAndTTL(bytes32 node_, address resolver_, uint64 ttl_) private {
        Record storage r = _records[node_];
        if (resolver_ != r.resolver) {
            r.resolver = resolver_;
            emit NewResolver(node_, resolver_);
        }
        if (ttl_ != r.ttl) {
            r.ttl = ttl_;
            emit NewTTL(node_, ttl_);
        }
    }

    function _setSubnodeOwner(bytes32 parent, bytes32 label, address owner_) private returns (bytes32 sub) {
        sub = keccak256(abi.encodePacked(parent, label));
        Record storage p = _records[parent];
        Record storage c = _records[sub];
        bytes32 authority = p.authority;
        uint32 gen = authority == bytes32(0) ? 0 : _generations[authority];
        if (c.authority != authority || c.generation != gen) {
            // Stale subname left over from a previous registration of the parent: start from scratch.
            c.resolver = address(0);
            c.ttl = 0;
            c.authority = authority;
            c.generation = gen;
        }
        c.owner = owner_;
        emit NewOwner(parent, label, owner_);
    }

    // ------------------------------------------------------------------ `.xdevs` registrar

    /// @notice Register `label`.xdevs for `owner_` for `duration` seconds (>= 365 days).
    /// @dev `msg.value` must equal `rentPrice(duration)` exactly; it is forwarded to the treasury.
    ///      The name's resolver is set to this contract and its ETH-type address record to `owner_`.
    function register(string calldata label, address owner_, uint256 duration)
        external
        payable
        whenInitialized
        returns (bytes32 node_, uint256 expires)
    {
        return _register(label, owner_, duration, false);
    }

    /// @notice Governance-only registration that may also take reserved labels (fee waived).
    function registerReserved(string calldata label, address owner_, uint256 duration)
        external
        onlyGovernance
        returns (bytes32 node_, uint256 expires)
    {
        return _register(label, owner_, duration, true);
    }

    function _register(string calldata label, address owner_, uint256 duration, bool privileged)
        private
        returns (bytes32 node_, uint256 expires)
    {
        if (owner_ == address(0)) revert ZeroAddress();
        if (duration < MIN_REGISTRATION_DURATION || duration > MAX_REGISTRATION_HORIZON) {
            revert InvalidDuration(duration);
        }
        bytes32 lh;
        (node_, lh) = _claimLabel(label, privileged);
        uint256 fee = privileged ? 0 : _checkFee(duration);

        expires = block.timestamp + duration;
        _expiries[node_] = expires;

        Record storage r = _records[node_];
        r.owner = owner_;
        r.ttl = 0;
        r.generation = ++_generations[node_]; // wipes old records and subnames
        r.resolver = address(this);
        r.authority = node_;

        emit NewOwner(TLD_NODE, lh, owner_);
        emit NewResolver(node_, address(this));
        _setAddr(node_, COIN_TYPE_ETH, abi.encodePacked(owner_));
        emit NameRegistered(label, lh, owner_, expires, fee);
        _payTreasury(fee); // interaction last (CEI)
    }

    /// @dev Validates `label`, checks availability / reservation / confusables, takes the skeleton.
    function _claimLabel(string calldata label, bool privileged) private returns (bytes32 node_, bytes32 lh) {
        XNSLabel.validate(label);
        lh = keccak256(bytes(label));
        if (reserved[lh] && !privileged) revert NameReserved(label);

        node_ = keccak256(abi.encodePacked(TLD_NODE, lh));
        uint256 oldExpiry = _expiries[node_];
        if (block.timestamp < oldExpiry + GRACE_PERIOD) revert NameNotAvailable(node_, oldExpiry);

        bytes32 sk = XNSLabel.skeleton(label);
        bytes32 holder = skeletonHolder[sk];
        if (holder != bytes32(0) && holder != node_ && block.timestamp < _expiries[holder] + GRACE_PERIOD) {
            revert ConfusableWithExisting(label, holder);
        }
        skeletonHolder[sk] = node_;
    }

    /// @notice Extend a name that is registered or in its grace period. Anyone may pay for it.
    // aderyn-fp-next-line(eth-send-unchecked-address) SA-F04: fee to the governance-set treasury
    function renew(string calldata label, uint256 duration) external payable returns (uint256 expires) {
        bytes32 lh = keccak256(bytes(label));
        bytes32 node_ = keccak256(abi.encodePacked(TLD_NODE, lh));
        uint256 oldExpiry = _expiries[node_];
        // slither-disable-next-line incorrect-equality (SA-F08: 0 = never registered sentinel)
        if (oldExpiry == 0 || block.timestamp >= oldExpiry + GRACE_PERIOD) revert NameNotRenewable(node_);
        if (duration < MIN_REGISTRATION_DURATION) revert InvalidDuration(duration);
        expires = oldExpiry + duration;
        if (expires > block.timestamp + MAX_REGISTRATION_HORIZON) revert InvalidDuration(duration);
        uint256 fee = _checkFee(duration);
        _expiries[node_] = expires;
        emit NameRenewed(label, lh, expires, fee);
        _payTreasury(fee);
    }

    function _checkFee(uint256 duration) private view returns (uint256 fee) {
        fee = rentPrice(duration);
        if (msg.value != fee) revert WrongFee(fee, msg.value);
    }

    function _payTreasury(uint256 fee) private {
        if (fee != 0) {
            // slither-disable-next-line arbitrary-send-eth (SA-F04: fee to the governance-set treasury)
            (bool ok,) = treasury.call{value: fee}("");
            if (!ok) revert FeeTransferFailed();
        }
    }

    /// @notice Fee (native wei) for `duration` seconds.
    function rentPrice(uint256 duration) public view returns (uint256) {
        return feePerYear * duration / 365 days;
    }

    /// @notice True iff `label` is normalized (see XNSLabel): 3..63 of [a-z0-9-], hyphen rules.
    function valid(string calldata label) external pure returns (bool) {
        return XNSLabel.isValid(label);
    }

    /// @notice True iff `register(label, …)` would currently succeed for a non-governance caller.
    function available(string calldata label) external view returns (bool) {
        if (!XNSLabel.isValid(label)) return false;
        bytes32 lh = keccak256(bytes(label));
        if (reserved[lh]) return false;
        bytes32 node_ = keccak256(abi.encodePacked(TLD_NODE, lh));
        if (block.timestamp < _expiries[node_] + GRACE_PERIOD) return false;
        bytes32 holder = skeletonHolder[XNSLabel.skeleton(label)];
        return holder == bytes32(0) || holder == node_ || block.timestamp >= _expiries[holder] + GRACE_PERIOD;
    }

    /// @notice Expiry of `uint256(keccak256(label))`.xdevs (ENS BaseRegistrar-compatible), 0 if never registered.
    function nameExpires(uint256 id) external view returns (uint256) {
        return _expiries[keccak256(abi.encodePacked(TLD_NODE, bytes32(id)))];
    }

    /// @notice Pure ENS namehash helper (no normalization).
    function namehash(string calldata name_) external pure returns (bytes32) {
        return XNSLabel.namehash(name_);
    }

    // ------------------------------------------------------------------ governance

    function setFeePerYear(uint256 feePerYear_) external onlyGovernance {
        feePerYear = feePerYear_;
        emit FeeChanged(feePerYear_);
    }

    function setTreasury(address treasury_) external onlyGovernance {
        if (treasury_ == address(0)) revert ZeroAddress();
        treasury = treasury_;
        emit TreasuryChanged(treasury_);
    }

    function setGovernance(address governance_) external onlyGovernance {
        if (governance_ == address(0)) revert ZeroAddress();
        governance = governance_;
        emit GovernanceChanged(governance_);
    }

    /// @notice Reserve / unreserve a label. Reserving does not affect a name that is already registered.
    function setReserved(string calldata label, bool isReserved) external onlyGovernance {
        _setReserved(label, isReserved);
    }

    function _setReserved(string calldata label, bool isReserved) private {
        bytes32 lh = keccak256(bytes(label));
        reserved[lh] = isReserved;
        emit ReservedChanged(lh, label, isReserved);
    }

    // ------------------------------------------------------------------ reverse registrar (addr.reverse)

    /// @notice Reverse node `<hex(addr)>.addr.reverse` for `addr`.
    function node(address addr_) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(ADDR_REVERSE_NODE, XNSLabel.hexAddressLabelhash(addr_)));
    }

    /// @notice Set the caller's primary name. `name_` must forward-resolve (`addr`) to the caller, or be
    ///         empty to clear it. Claims the caller's reverse node (owner = caller, resolver = this).
    function setName(string calldata name_) external returns (bytes32 rnode) {
        if (bytes(name_).length != 0) {
            address fwd = resolveAddr(XNSLabel.namehash(name_));
            if (fwd != msg.sender) revert ReverseNameMismatch(name_, fwd);
        }
        rnode = _claimReverse(msg.sender, msg.sender);
        _names[_rkey(rnode)] = name_;
        emit NameChanged(rnode, name_);
    }

    /// @notice Transfer control of the caller's reverse node to `owner_` (ENS ReverseRegistrar.claim).
    function claim(address owner_) external returns (bytes32) {
        return _claimReverse(msg.sender, owner_);
    }

    function _claimReverse(address addr_, address owner_) private returns (bytes32 rnode) {
        bytes32 label = XNSLabel.hexAddressLabelhash(addr_);
        rnode = keccak256(abi.encodePacked(ADDR_REVERSE_NODE, label));
        Record storage r = _records[rnode];
        if (r.owner != owner_) {
            r.owner = owner_;
            emit NewOwner(ADDR_REVERSE_NODE, label, owner_);
        }
        if (r.resolver != address(this)) {
            r.resolver = address(this);
            emit NewResolver(rnode, address(this));
        }
        emit ReverseClaimed(addr_, rnode);
    }

    /// @notice Verified primary name of `addr_`: the reverse `name` record, returned only if that name
    ///         currently forward-resolves back to `addr_` (wallets should display this, not `name(node)`).
    function primaryName(address addr_) external view returns (string memory) {
        bytes32 rnode = node(addr_);
        if (_records[rnode].resolver != address(this)) return "";
        string memory n = _names[_rkey(rnode)];
        if (bytes(n).length == 0 || resolveAddr(XNSLabel.namehash(n)) != addr_) return "";
        return n;
    }

    /// @notice Full forward resolution of `node_` the way a client does it: registry `resolver(node)`
    ///         (empty if expired), then that resolver's `addr(node)`. External resolvers are queried with
    ///         a STATICCALL; a reverting or malformed resolver resolves to address(0).
    function resolveAddr(bytes32 node_) public view returns (address) {
        if (!_live(node_)) return address(0);
        address r = _records[node_].resolver;
        if (r == address(this)) return addr(node_);
        if (r.code.length == 0) return address(0);
        try IAddrResolver(r).addr(node_) returns (address payable a) {
            return a;
        } catch {
            return address(0);
        }
    }

    // ------------------------------------------------------------------ public resolver

    function addr(bytes32 node_) public view returns (address payable) {
        if (!_live(node_)) return payable(address(0));
        bytes memory a = _addrs[_rkey(node_)][COIN_TYPE_ETH];
        if (a.length != 20) return payable(address(0));
        return payable(address(bytes20(a)));
    }

    function addr(bytes32 node_, uint256 coinType) external view returns (bytes memory) {
        if (!_live(node_)) return "";
        return _addrs[_rkey(node_)][coinType];
    }

    function setAddr(bytes32 node_, address a) external authorised(node_) {
        _setAddr(node_, COIN_TYPE_ETH, abi.encodePacked(a));
    }

    /// @notice ENSIP-9 setter. EVM coin types (60 and ENSIP-11 `0x80000000|chainId`) take 0 or 20 bytes.
    function setAddr(bytes32 node_, uint256 coinType, bytes calldata a) external authorised(node_) {
        _setAddr(node_, coinType, a);
    }

    function _setAddr(bytes32 node_, uint256 coinType, bytes memory a) private {
        if ((coinType == COIN_TYPE_ETH || coinType & 0x80000000 != 0) && a.length != 0 && a.length != 20) {
            revert InvalidAddressLength(coinType, a.length);
        }
        emit AddressChanged(node_, coinType, a);
        if (coinType == COIN_TYPE_ETH) {
            emit AddrChanged(node_, a.length == 20 ? address(bytes20(a)) : address(0));
        }
        _addrs[_rkey(node_)][coinType] = a;
    }

    function text(bytes32 node_, string calldata key) external view returns (string memory) {
        if (!_live(node_)) return "";
        return _texts[_rkey(node_)][key];
    }

    function setText(bytes32 node_, string calldata key, string calldata value) external authorised(node_) {
        _texts[_rkey(node_)][key] = value;
        emit TextChanged(node_, key, key, value);
    }

    /// @notice Raw reverse `name` record (unverified - prefer {primaryName}).
    function name(bytes32 node_) external view returns (string memory) {
        if (!_live(node_)) return "";
        return _names[_rkey(node_)];
    }

    function setName(bytes32 node_, string calldata name_) external authorised(node_) {
        _names[_rkey(node_)] = name_;
        emit NameChanged(node_, name_);
    }

    /// @notice Wipe every resolver record of `node_` (ENS IVersionableResolver).
    function clearRecords(bytes32 node_) external authorised(node_) {
        uint64 v = ++recordVersions[node_];
        emit VersionChanged(node_, v);
    }

    function supportsInterface(bytes4 interfaceId) external pure returns (bool) {
        return interfaceId == 0x01ffc9a7 // ERC-165
            || interfaceId == type(IAddrResolver).interfaceId // 0x3b3b57de
            || interfaceId == type(IAddressResolver).interfaceId // 0xf1cb7e06
            || interfaceId == type(ITextResolver).interfaceId // 0x59d1d43c
            || interfaceId == type(INameResolver).interfaceId // 0x691f3431
            || interfaceId == type(IVersionableResolver).interfaceId; // 0xd700ff33
    }
}
