// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {XdevsLightClient} from "../../src/bridge/XdevsLightClient.sol";
import {ExecutionHeaderStore} from "../../src/bridge/ExecutionHeaderStore.sol";
import {EthProofs} from "../../src/bridge/lib/EthProofs.sol";
import {MerklePatricia} from "../../src/bridge/lib/MerklePatricia.sol";

contract ProofHarness {
    function receipt(bytes32 root, uint256 i, bytes[] calldata proof) external pure returns (bytes memory) {
        return EthProofs.provenReceipt(root, i, proof);
    }
}

/// Real xDEVS testnet (Enclave, chain 717171) commit certificates verified on an "Ethereum" light client, plus
/// dev-key fixtures for validator-set rotation and equivocation (the testnet has not activated staking sets).
contract XdevsLightClientTest is Test {
    string constant VALIDATOR_T = "JsonValidator(bytes32 key,uint64 power)";
    string constant SIG_T = "JsonSignature(uint256 validatorIndex,bytes32 r,bytes32 s)";

    string real;
    string dev;
    XdevsLightClient lc;
    XdevsLightClient.Validator[] set;

    function setUp() public {
        real = vm.readFile("test/bridge/fixtures/xdevs-testnet.json");
        dev = vm.readFile("test/bridge/fixtures/xdevs-dev.json");
        XdevsLightClient.Validator[] memory s = _validators(real, ".validators");
        for (uint256 i; i < s.length; ++i) {
            set.push(s[i]);
        }
        lc = new XdevsLightClient(717171, 3600, type(uint64).max, address(0x1000), 0, s);
    }

    function _validators(string memory json, string memory key)
        internal
        pure
        returns (XdevsLightClient.Validator[] memory)
    {
        return abi.decode(vm.parseJsonTypeArray(json, key, VALIDATOR_T), (XdevsLightClient.Validator[]));
    }

    function _sigs(string memory json, string memory key)
        internal
        pure
        returns (XdevsLightClient.Signature[] memory)
    {
        return abi.decode(vm.parseJsonTypeArray(json, key, SIG_T), (XdevsLightClient.Signature[]));
    }

    function _commit(uint256 i, string memory sigsKey) internal returns (uint256 gas) {
        string memory c = string.concat(".commits[", vm.toString(i), "]");
        uint64 height = uint64(vm.parseJsonUint(real, string.concat(c, ".height")));
        uint32 round = uint32(vm.parseJsonUint(real, string.concat(c, ".round")));
        bytes32 hash = vm.parseJsonBytes32(real, string.concat(c, ".blockHash"));
        XdevsLightClient.Signature[] memory sigs = _sigs(real, string.concat(c, ".", sigsKey));
        uint256 g = gasleft();
        lc.submitCommit(height, round, hash, set, sigs);
        gas = g - gasleft();
    }

    // ------------------------------------------------------------------------------------------------------------
    // Real testnet data
    // ------------------------------------------------------------------------------------------------------------

    function test_real_certificate_header_and_receipt() public {
        uint256 gas = _commit(0, "quorumSigs");
        emit log_named_uint("submitCommit gas (3 of 4 real signatures)", gas);
        bytes32 hash = vm.parseJsonBytes32(real, ".commits[0].blockHash");
        assertEq(lc.certifiedBlock(67), hash);
        assertEq(lc.trusted(hash), 68);

        bytes memory rlp = vm.parseJsonBytes(real, ".commits[0].headerRlp");
        uint256 g = gasleft();
        lc.submitHeader(rlp);
        emit log_named_uint("submitHeader gas", g - gasleft());
        (bytes32 s, bytes32 r) = lc.verifiedRoots(67);
        assertEq(s, vm.parseJsonBytes32(real, ".commits[0].stateRoot"));
        assertEq(r, vm.parseJsonBytes32(real, ".commits[0].receiptsRoot"));

        // The testnet's only transaction so far: its receipt is proven against the certified receipts root.
        bytes[] memory proof = vm.parseJsonBytesArray(real, ".commits[0].receiptProof");
        bytes memory receipt = new ProofHarness().receipt(r, 0, proof);
        assertEq(uint8(receipt[0]), 2); // EIP-1559 receipt

        // The parent of block 67 is now trusted: the relayer can walk back without more certificates.
        assertGt(lc.latestNumber(), 0);
    }

    function test_real_certificate_all_signatures() public {
        uint256 gas = _commit(1, "allSigs");
        emit log_named_uint("submitCommit gas (4 of 4 real signatures)", gas);
        lc.submitHeader(vm.parseJsonBytes(real, ".commits[1].headerRlp"));
    }

    function test_rejects_forged_signature() public {
        XdevsLightClient.Signature[] memory sigs = _sigs(real, ".commits[0].quorumSigs");
        sigs[1].s ^= bytes32(uint256(1) << 8);
        bytes32 hash = vm.parseJsonBytes32(real, ".commits[0].blockHash");
        vm.expectRevert(
            abi.encodeWithSelector(XdevsLightClient.InvalidSignature.selector, sigs[1].validatorIndex)
        );
        lc.submitCommit(67, 0, hash, set, sigs);
    }

    function test_rejects_other_block_hash_or_height() public {
        XdevsLightClient.Signature[] memory sigs = _sigs(real, ".commits[0].quorumSigs");
        bytes32 hash = vm.parseJsonBytes32(real, ".commits[0].blockHash");
        vm.expectRevert();
        lc.submitCommit(67, 0, keccak256("other"), set, sigs);
        vm.expectRevert();
        lc.submitCommit(68, 0, hash, set, sigs);
        vm.expectRevert();
        lc.submitCommit(67, 1, hash, set, sigs);
    }

    function test_rejects_insufficient_power() public {
        XdevsLightClient.Signature[] memory all = _sigs(real, ".commits[0].allSigs");
        XdevsLightClient.Signature[] memory two = new XdevsLightClient.Signature[](2);
        two[0] = all[0];
        two[1] = all[1];
        bytes32 hash = vm.parseJsonBytes32(real, ".commits[0].blockHash");
        uint64 p = set[0].power;
        vm.expectRevert(abi.encodeWithSelector(XdevsLightClient.InsufficientPower.selector, 2 * p, 4 * p));
        lc.submitCommit(67, 0, hash, set, two);
    }

    function test_rejects_duplicate_or_unsorted_signers() public {
        XdevsLightClient.Signature[] memory all = _sigs(real, ".commits[0].allSigs");
        XdevsLightClient.Signature[] memory dup = new XdevsLightClient.Signature[](3);
        dup[0] = all[0];
        dup[1] = all[0];
        dup[2] = all[1];
        bytes32 hash = vm.parseJsonBytes32(real, ".commits[0].blockHash");
        vm.expectRevert(XdevsLightClient.SignaturesNotSorted.selector);
        lc.submitCommit(67, 0, hash, set, dup);
    }

    function test_rejects_wrong_validator_set() public {
        XdevsLightClient.Signature[] memory sigs = _sigs(real, ".commits[0].quorumSigs");
        bytes32 hash = vm.parseJsonBytes32(real, ".commits[0].blockHash");
        XdevsLightClient.Validator[] memory forged = set;
        forged[3].power = 1_000_000_000; // inflate a signer's power
        vm.expectRevert(XdevsLightClient.WrongValidatorSet.selector);
        lc.submitCommit(67, 0, hash, forged, sigs);
    }

    function test_rejects_untrusted_header_and_bad_receipt_proof() public {
        bytes memory rlp = vm.parseJsonBytes(real, ".commits[0].headerRlp");
        vm.expectRevert(abi.encodeWithSelector(ExecutionHeaderStore.UntrustedHeader.selector, keccak256(rlp)));
        lc.submitHeader(rlp);
        vm.expectRevert(abi.encodeWithSelector(ExecutionHeaderStore.UnknownBlock.selector, 67));
        lc.verifiedRoots(67);

        _commit(0, "quorumSigs");
        lc.submitHeader(rlp);
        (, bytes32 r) = lc.verifiedRoots(67);
        bytes[] memory proof = vm.parseJsonBytesArray(real, ".commits[0].receiptProof");
        ProofHarness h = new ProofHarness();
        proof[0][proof[0].length - 1] ^= 0x01;
        vm.expectRevert(MerklePatricia.BadProof.selector);
        h.receipt(r, 0, proof);
    }

    function test_replayed_certificate_is_a_noop() public {
        _commit(0, "quorumSigs");
        _commit(0, "allSigs");
        assertFalse(lc.frozen());
    }

    // ------------------------------------------------------------------------------------------------------------
    // Dev fixtures: epoch rotation from a Staking storage proof, equivocation freeze
    // ------------------------------------------------------------------------------------------------------------

    function _devClient() internal returns (XdevsLightClient d) {
        d = new XdevsLightClient(717171, 10, 1, address(0x1000), 0, _validators(dev, ".genesisSet"));
    }

    function _stakingProof() internal view returns (XdevsLightClient.StakingSetProof memory p) {
        p.operators = vm.parseJsonAddressArray(dev, ".staking.operators");
        p.keys = vm.parseJsonBytes32Array(dev, ".staking.keys");
        p.powersWei = vm.parseJsonUintArray(dev, ".staking.powersWei");
        p.accountProof = vm.parseJsonBytesArray(dev, ".staking.accountProof");
        p.epochWordProof = vm.parseJsonBytesArray(dev, ".staking.epochWordProof");
        p.setHashProof = vm.parseJsonBytesArray(dev, ".staking.setHashProof");
    }

    function test_dev_validator_set_rotation() public {
        XdevsLightClient d = _devClient();
        XdevsLightClient.Validator[] memory g = _validators(dev, ".genesisSet");
        XdevsLightClient.Validator[] memory n = _validators(dev, ".newSet");
        bytes memory h10 = vm.parseJsonBytes(dev, ".headers.h10");
        bytes memory h11 = vm.parseJsonBytes(dev, ".headers.h11");
        assertEq(d.validatorSetBoundary(10), 0);
        assertEq(d.validatorSetBoundary(11), 10);

        d.submitCommit(10, 0, keccak256(h10), g, _sigs(dev, ".commit10"));
        d.submitHeader(h10);
        // Height 11 is governed by the boundary-10 set, which is not proven yet.
        XdevsLightClient.Signature[] memory s11 = _sigs(dev, ".commit11");
        vm.expectRevert(abi.encodeWithSelector(XdevsLightClient.UnknownValidatorSet.selector, 10));
        d.submitCommit(11, 0, keccak256(h11), n, s11);

        XdevsLightClient.StakingSetProof memory p = _stakingProof();
        uint256 gas = gasleft();
        d.submitValidatorSet(10, p);
        emit log_named_uint("submitValidatorSet gas (4 Staking members)", gas - gasleft());
        (bytes32 h, uint64 total) = d.validatorSets(10);
        assertEq(h, keccak256(abi.encode(n)));
        assertEq(total, 51); // 30 + 20 + 1; the invalid key is dropped like the node does

        // The old set no longer certifies height 11; the new one does.
        XdevsLightClient.Signature[] memory old11 = _sigs(dev, ".commit11Old");
        vm.expectRevert(XdevsLightClient.WrongValidatorSet.selector);
        d.submitCommit(11, 0, keccak256(h11), g, old11);
        d.submitCommit(11, 0, keccak256(h11), n, s11);
        d.submitHeader(h11);
    }

    function test_dev_rotation_rejects_bad_proofs() public {
        XdevsLightClient d = _devClient();
        bytes memory h10 = vm.parseJsonBytes(dev, ".headers.h10");
        XdevsLightClient.StakingSetProof memory p = _stakingProof();
        // Header of the boundary block not stored yet.
        vm.expectRevert(abi.encodeWithSelector(ExecutionHeaderStore.UnknownBlock.selector, 10));
        d.submitValidatorSet(10, p);
        d.submitCommit(10, 0, keccak256(h10), _validators(dev, ".genesisSet"), _sigs(dev, ".commit10"));
        d.submitHeader(h10);
        // Not an epoch boundary.
        vm.expectRevert(abi.encodeWithSelector(XdevsLightClient.NotEpochBoundary.selector, 9));
        d.submitValidatorSet(9, p);
        // Arrays that do not hash to the proven validatorSetHash.
        p.powersWei[0] += 1;
        vm.expectRevert(XdevsLightClient.BadStakingProof.selector);
        d.submitValidatorSet(10, p);
        p.powersWei[0] -= 1;
        // Tampered storage proof.
        p.setHashProof[p.setHashProof.length - 1][4] ^= 0x01;
        vm.expectRevert(MerklePatricia.BadProof.selector);
        d.submitValidatorSet(10, p);
    }

    function test_dev_conflicting_certificates_freeze() public {
        XdevsLightClient d = _devClient();
        XdevsLightClient.Validator[] memory g = _validators(dev, ".genesisSet");
        bytes memory h10 = vm.parseJsonBytes(dev, ".headers.h10");
        d.submitCommit(10, 0, keccak256(h10), g, _sigs(dev, ".commit10"));
        d.submitHeader(h10);
        // A second quorum certificate for another block at height 10: > 1/3 equivocated.
        d.submitCommit(
            10, 1, vm.parseJsonBytes32(dev, ".conflict10.blockHash"), g, _sigs(dev, ".conflict10.sigs")
        );
        assertTrue(d.frozen());
        vm.expectRevert(ExecutionHeaderStore.IsFrozen.selector);
        d.verifiedRoots(10);
        vm.expectRevert(ExecutionHeaderStore.IsFrozen.selector);
        d.submitHeader(vm.parseJsonBytes(dev, ".headers.h9"));
    }

    function test_dev_ancestor_walk() public {
        XdevsLightClient d = _devClient();
        bytes memory h10 = vm.parseJsonBytes(dev, ".headers.h10");
        bytes memory h9 = vm.parseJsonBytes(dev, ".headers.h9");
        d.submitCommit(10, 0, keccak256(h10), _validators(dev, ".genesisSet"), _sigs(dev, ".commit10"));
        d.submitHeader(h10);
        d.submitHeader(h9); // trusted as the parent of a certified block
        assertEq(d.blockHashAt(9), keccak256(h9));
    }
}
