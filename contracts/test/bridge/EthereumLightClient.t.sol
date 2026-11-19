// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {EthereumLightClient} from "../../src/bridge/EthereumLightClient.sol";
import {ExecutionHeaderStore} from "../../src/bridge/ExecutionHeaderStore.sol";
import {BLS12381} from "../../src/bridge/lib/BLS12381.sol";
import {EthProofs} from "../../src/bridge/lib/EthProofs.sol";
import {MerklePatricia} from "../../src/bridge/lib/MerklePatricia.sol";

contract EthProofHarness {
    function log(bytes32 root, uint256 i, bytes[] calldata proof, uint256 logIndex)
        external
        pure
        returns (EthProofs.Log memory)
    {
        return EthProofs.receiptLog(EthProofs.provenReceipt(root, i, proof), logIndex);
    }

    function storageAt(
        bytes32 stateRoot,
        address account,
        bytes[] calldata accountProof,
        bytes32 slot,
        bytes[] calldata storageProof
    ) external pure returns (uint256) {
        return EthProofs.storageValue(
            EthProofs.storageRoot(stateRoot, account, accountProof), slot, storageProof
        );
    }
}

/// Real Sepolia (Gloas) light-client data: sync committees of periods P-1 and P, a rotation update signed by
/// committee P-1, the finality update signed by committee P, then the finalized execution header with a receipt
/// and a storage proof in it.
contract EthereumLightClientTest is Test {
    string fx;
    EthereumLightClient lc;
    uint256 period;

    function setUp() public {
        fx = vm.readFile("test/bridge/fixtures/sepolia.json");
        period = vm.parseJsonUint(fx, ".checkpointPeriod");
        uint256[] memory epochs = vm.parseJsonUintArray(fx, ".forkEpochs");
        string[] memory versions = vm.parseJsonStringArray(fx, ".forkVersions");
        uint64[] memory e = new uint64[](epochs.length);
        bytes4[] memory v = new bytes4[](epochs.length);
        for (uint256 i; i < epochs.length; ++i) {
            e[i] = uint64(epochs[i]);
            v[i] = bytes4(vm.parseBytes(versions[i]));
        }
        lc = new EthereumLightClient(
            vm.parseJsonBytes32(fx, ".genesisValidatorsRoot"),
            vm.parseJsonUint(fx, ".genesisTime"),
            e,
            v,
            uint64(vm.parseJsonUint(fx, ".gloasForkEpoch") * 32),
            period,
            vm.parseJsonBytes32(fx, ".checkpointCommitteeRoot")
        );
        // The clock just after the finality update's signature slot.
        vm.warp(vm.parseJsonUint(fx, ".genesisTime") + (vm.parseJsonUint(fx, ".signatureSlot") + 1) * 12);
    }

    function _committee(string memory key) internal view returns (BLS12381.G1[] memory) {
        return abi.decode(vm.parseJsonBytes(fx, key), (BLS12381.G1[]));
    }

    function _update(string memory key)
        internal
        view
        returns (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n)
    {
        (u, n) = abi.decode(
            vm.parseJsonBytes(fx, key), (EthereumLightClient.Update, EthereumLightClient.NonSigner[])
        );
    }

    function _loadAndRotate() internal {
        BLS12381.G1[] memory c1 = _committee(".committeeP1");
        uint256 g = gasleft();
        lc.loadCommittee(period, c1);
        emit log_named_uint("loadCommittee gas (512 keys)", g - gasleft());
        (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n) =
            _update(".rotationUpdate");
        g = gasleft();
        lc.submitUpdate(u, n);
        emit log_named_uint(
            string.concat("submitUpdate gas, rotation (", vm.toString(n.length), " non-signers)"),
            g - gasleft()
        );
        assertEq(lc.committeeRoot(period + 1), vm.parseJsonBytes32(fx, ".nextCommitteeRoot"));
        lc.loadCommittee(period + 1, _committee(".committeeP2"));
    }

    function test_real_sepolia_end_to_end() public {
        _loadAndRotate();
        (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n) =
            _update(".finalityUpdate");
        uint256 g = gasleft();
        lc.submitUpdate(u, n);
        emit log_named_uint(
            string.concat("submitUpdate gas, finality (", vm.toString(n.length), " non-signers)"),
            g - gasleft()
        );
        bytes32 execHash = vm.parseJsonBytes32(fx, ".executionBlockHash");
        assertEq(lc.finalizedSlot(), vm.parseJsonUint(fx, ".finalizedSlot"));
        assertGt(lc.trusted(execHash), 0);

        bytes memory rlp = vm.parseJsonBytes(fx, ".executionHeaderRlp");
        lc.submitHeader(rlp);
        uint256 number = vm.parseJsonUint(fx, ".executionBlockNumber");
        (bytes32 stateRoot, bytes32 receiptsRoot) = lc.verifiedRoots(number);
        assertEq(stateRoot, vm.parseJsonBytes32(fx, ".stateRoot"));
        assertEq(receiptsRoot, vm.parseJsonBytes32(fx, ".receiptsRoot"));

        // A real receipt and log of that finalized Sepolia block.
        EthProofHarness h = new EthProofHarness();
        EthProofs.Log memory log = h.log(
            receiptsRoot,
            vm.parseJsonUint(fx, ".receipt.txIndex"),
            vm.parseJsonBytesArray(fx, ".receipt.proof"),
            0
        );
        assertEq(log.emitter, vm.parseJsonAddress(fx, ".receipt.emitter"));
        assertEq(log.topics[0], vm.parseJsonBytes32(fx, ".receipt.topic0"));
        assertEq(log.data, vm.parseJsonBytes(fx, ".receipt.data"));

        // A real eth_getProof storage proof (Sepolia USDC balance slot) against the verified state root.
        uint256 value = h.storageAt(
            stateRoot,
            vm.parseJsonAddress(fx, ".storage.account"),
            vm.parseJsonBytesArray(fx, ".storage.accountProof"),
            vm.parseJsonBytes32(fx, ".storage.slot"),
            vm.parseJsonBytesArray(fx, ".storage.storageProof")
        );
        assertEq(value, vm.parseJsonUint(fx, ".storage.value"));
    }

    function test_rejects_unknown_committee() public {
        (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n) =
            _update(".rotationUpdate");
        vm.expectRevert(abi.encodeWithSelector(EthereumLightClient.CommitteeUnknown.selector, period));
        lc.submitUpdate(u, n);
        vm.expectRevert(abi.encodeWithSelector(EthereumLightClient.CommitteeUnknown.selector, period + 1));
        lc.loadCommittee(period + 1, _committee(".committeeP2"));
    }

    function test_rejects_wrong_committee_keys() public {
        BLS12381.G1[] memory c = _committee(".committeeP1");
        (c[0], c[1]) = (c[1], c[0]);
        if (keccak256(abi.encode(c[0])) == keccak256(abi.encode(c[1]))) return; // duplicate key, nothing swapped
        vm.expectRevert(EthereumLightClient.BadCommittee.selector);
        lc.loadCommittee(period, c);
        // The next period's committee is not the checkpoint's.
        vm.expectRevert(EthereumLightClient.BadCommittee.selector);
        lc.loadCommittee(period, _committee(".committeeP2"));
    }

    function test_rejects_forged_signature() public {
        lc.loadCommittee(period, _committee(".committeeP1"));
        (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n) =
            _update(".rotationUpdate");
        (EthereumLightClient.Update memory f,) = _update(".finalityUpdate");
        u.signature = f.signature; // a valid G2 point, but another committee's signature over another header
        vm.expectRevert(EthereumLightClient.BadSignature.selector);
        lc.submitUpdate(u, n);
    }

    function test_rejects_tampered_attested_header() public {
        lc.loadCommittee(period, _committee(".committeeP1"));
        (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n) =
            _update(".rotationUpdate");
        u.attested.proposerIndex += 1; // signed root no longer matches
        vm.expectRevert(EthereumLightClient.BadSignature.selector);
        lc.submitUpdate(u, n);
    }

    function test_rejects_bad_branches() public {
        lc.loadCommittee(period, _committee(".committeeP1"));
        (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n) =
            _update(".rotationUpdate");
        bytes32 saved = u.finalityBranch[0];
        u.finalityBranch[0] = bytes32(uint256(saved) ^ 1);
        vm.expectRevert(EthereumLightClient.BadFinalityBranch.selector);
        lc.submitUpdate(u, n);
        u.finalityBranch[0] = saved;
        u.finalizedExecutionBlockHash = keccak256("fake execution block");
        vm.expectRevert(EthereumLightClient.BadExecutionBranch.selector);
        lc.submitUpdate(u, n);
        (u,) = _update(".rotationUpdate");
        u.nextSyncCommitteeRoot = keccak256("attacker committee");
        vm.expectRevert(EthereumLightClient.BadNextCommitteeBranch.selector);
        lc.submitUpdate(u, n);
    }

    function test_rejects_bad_participation_and_non_signers() public {
        lc.loadCommittee(period, _committee(".committeeP1"));
        (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n) =
            _update(".rotationUpdate");
        // Fewer than 2/3 participants.
        bytes32[2] memory bits = u.syncCommitteeBits;
        u.syncCommitteeBits = [bits[0], bytes32(0)];
        vm.expectRevert(
            abi.encodeWithSelector(EthereumLightClient.InsufficientParticipation.selector, _popcount(bits[0]))
        );
        lc.submitUpdate(u, n);
        u.syncCommitteeBits = bits;
        if (n.length == 0) return;
        // A non-signer whose key proof does not match the loaded committee.
        n[0].pubkey.xLo ^= 1;
        vm.expectRevert(EthereumLightClient.BadNonSigners.selector);
        lc.submitUpdate(u, n);
        (, n) = _update(".rotationUpdate");
        // Hiding a non-signer (claiming everyone signed) changes the aggregate key: the count check catches it.
        EthereumLightClient.NonSigner[] memory fewer = new EthereumLightClient.NonSigner[](n.length - 1);
        for (uint256 i = 1; i < n.length; ++i) {
            fewer[i - 1] = n[i];
        }
        vm.expectRevert(EthereumLightClient.BadNonSigners.selector);
        lc.submitUpdate(u, fewer);
    }

    function test_rejects_future_and_stale_updates() public {
        _loadAndRotate();
        (EthereumLightClient.Update memory u, EthereumLightClient.NonSigner[] memory n) =
            _update(".finalityUpdate");
        vm.warp(vm.parseJsonUint(fx, ".genesisTime") + uint256(u.signatureSlot) * 12 - 12);
        vm.expectRevert(abi.encodeWithSelector(EthereumLightClient.FutureSlot.selector, u.signatureSlot));
        lc.submitUpdate(u, n);
        vm.warp(block.timestamp + 24);
        lc.submitUpdate(u, n);
        vm.expectRevert(EthereumLightClient.StaleUpdate.selector);
        lc.submitUpdate(u, n); // replay
    }

    function test_rejects_untrusted_execution_header() public {
        bytes memory rlp = vm.parseJsonBytes(fx, ".executionHeaderRlp");
        vm.expectRevert(abi.encodeWithSelector(ExecutionHeaderStore.UntrustedHeader.selector, keccak256(rlp)));
        lc.submitHeader(rlp);
    }

    function test_bad_receipt_and_storage_proofs() public {
        EthProofHarness h = new EthProofHarness();
        bytes32 receiptsRoot = vm.parseJsonBytes32(fx, ".receiptsRoot");
        bytes[] memory proof = vm.parseJsonBytesArray(fx, ".receipt.proof");
        uint256 txIndex = vm.parseJsonUint(fx, ".receipt.txIndex");
        // Proof for another index.
        vm.expectRevert();
        h.log(receiptsRoot, txIndex + 1, proof, 0);
        // Tampered leaf.
        proof[proof.length - 1][40] ^= 0x01;
        vm.expectRevert(MerklePatricia.BadProof.selector);
        h.log(receiptsRoot, txIndex, proof, 0);
        bytes[] memory acc = vm.parseJsonBytesArray(fx, ".storage.accountProof");
        vm.expectRevert(MerklePatricia.BadProof.selector);
        h.storageAt(
            keccak256("wrong state root"),
            vm.parseJsonAddress(fx, ".storage.account"),
            acc,
            vm.parseJsonBytes32(fx, ".storage.slot"),
            vm.parseJsonBytesArray(fx, ".storage.storageProof")
        );
    }

    function _popcount(bytes32 w) internal pure returns (uint256 c) {
        uint256 x = uint256(w);
        while (x != 0) {
            x &= x - 1;
            ++c;
        }
    }
}
