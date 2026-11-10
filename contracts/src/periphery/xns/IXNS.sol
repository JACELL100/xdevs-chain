// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title IENSRegistry
/// @notice The ENS registry interface (EIP-137), implemented verbatim by the XNS registry so that existing
///         ENS tooling (ethers `EnsResolver`, viem `getEnsAddress` with a custom registry, etc.) works.
interface IENSRegistry {
    event NewOwner(bytes32 indexed node, bytes32 indexed label, address owner);
    event Transfer(bytes32 indexed node, address owner);
    event NewResolver(bytes32 indexed node, address resolver);
    event NewTTL(bytes32 indexed node, uint64 ttl);
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);

    function setRecord(bytes32 node, address owner, address resolver, uint64 ttl) external;
    function setSubnodeRecord(bytes32 node, bytes32 label, address owner, address resolver, uint64 ttl)
        external;
    function setSubnodeOwner(bytes32 node, bytes32 label, address owner) external returns (bytes32);
    function setResolver(bytes32 node, address resolver) external;
    function setOwner(bytes32 node, address owner) external;
    function setTTL(bytes32 node, uint64 ttl) external;
    function setApprovalForAll(address operator, bool approved) external;
    function owner(bytes32 node) external view returns (address);
    function resolver(bytes32 node) external view returns (address);
    function ttl(bytes32 node) external view returns (uint64);
    function recordExists(bytes32 node) external view returns (bool);
    function isApprovedForAll(address owner, address operator) external view returns (bool);
}

/// @notice ENSIP-1 `addr(bytes32)` resolver profile. interfaceId 0x3b3b57de.
interface IAddrResolver {
    event AddrChanged(bytes32 indexed node, address a);

    function addr(bytes32 node) external view returns (address payable);
}

/// @notice ENSIP-9 / ENSIP-11 multicoin resolver profile. interfaceId 0xf1cb7e06.
interface IAddressResolver {
    event AddressChanged(bytes32 indexed node, uint256 coinType, bytes newAddress);

    function addr(bytes32 node, uint256 coinType) external view returns (bytes memory);
}

/// @notice ENSIP-5 text records. interfaceId 0x59d1d43c.
interface ITextResolver {
    event TextChanged(bytes32 indexed node, string indexed indexedKey, string key, string value);

    function text(bytes32 node, string calldata key) external view returns (string memory);
}

/// @notice ENSIP-3 reverse `name` profile. interfaceId 0x691f3431.
interface INameResolver {
    event NameChanged(bytes32 indexed node, string name);

    function name(bytes32 node) external view returns (string memory);
}

/// @notice ENS `IVersionableResolver` (clearRecords). interfaceId 0xd700ff33.
interface IVersionableResolver {
    event VersionChanged(bytes32 indexed node, uint64 newVersion);

    function recordVersions(bytes32 node) external view returns (uint64);
}

/// @title IXNSRegistrar
/// @notice `.xdevs` second-level registrar + reverse registrar + governance surface of the XNS predeploy.
interface IXNSRegistrar {
    event NameRegistered(
        string label, bytes32 indexed labelhash, address indexed owner, uint256 expires, uint256 fee
    );
    event NameRenewed(string label, bytes32 indexed labelhash, uint256 expires, uint256 fee);
    event ReverseClaimed(address indexed addr, bytes32 indexed node);
    event FeeChanged(uint256 feePerYear);
    event TreasuryChanged(address treasury);
    event GovernanceChanged(address governance);
    event ReservedChanged(bytes32 indexed labelhash, string label, bool reserved);
    event Initialized(address governance, address treasury, uint256 feePerYear);

    function register(string calldata label, address owner, uint256 duration)
        external
        payable
        returns (bytes32 node, uint256 expires);
    function renew(string calldata label, uint256 duration) external payable returns (uint256 expires);
    function available(string calldata label) external view returns (bool);
    function valid(string calldata label) external pure returns (bool);
    function nameExpires(uint256 id) external view returns (uint256);
    function rentPrice(uint256 duration) external view returns (uint256);

    function setName(string calldata name) external returns (bytes32);
    function claim(address owner) external returns (bytes32);
    function node(address addr) external pure returns (bytes32);
    function primaryName(address addr) external view returns (string memory);
}
