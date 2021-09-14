use std::convert::TryFrom;

use crate::{blake2b_hash::Blake2bHash, util, Digest};
use blake2::{
    digest::{Update, VariableOutput},
    VarBlake2b,
};
use itertools::Itertools;
use serde::{Deserialize, Serialize};

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum MerkleVerificationError {
    #[error("Index out of bounds. Count: {count}, index: {index}")]
    IndexOutOfBounds { count: u64, index: u64 },
    #[error(
        "Unexpected proof length. Count: {count}, index: {index}, \
         expected proof length: {expected_proof_length}, \
         actual proof length: {actual_proof_length}"
    )]
    UnexpectedProofLength {
        count: u64,
        index: u64,
        expected_proof_length: u64,
        actual_proof_length: usize,
    },
}

#[derive(thiserror::Error, Debug, Eq, PartialEq)]
pub enum MerkleConstructionError {
    #[error("Could not construct Merkle proof. Empty proof must have index 0. Index: {index}")]
    EmptyProofMustHaveIndex { index: u64 },
    #[error(
        "Could not construct Merkle proof. Index out of bounds.  Count: {count}, index: {index}"
    )]
    IndexOutOfBounds { count: u64, index: u64 },
    #[error("The chunk has incorrect proof")]
    IncorrectChunkProof,
    #[error("The idexed merkle proof is incorrect")]
    IncorrectIndexedMerkleProof,
}

#[cfg_attr(
    feature = "std",
    derive(serde::Deserialize,),
    serde(deny_unknown_fields)
)]
pub struct IndexedMerkleProofDeserializeValidator {
    index: u64,
    count: u64,
    merkle_proof: Vec<Blake2bHash>,
}

impl TryFrom<IndexedMerkleProofDeserializeValidator> for IndexedMerkleProof {
    type Error = MerkleConstructionError;
    fn try_from(value: IndexedMerkleProofDeserializeValidator) -> Result<Self, Self::Error> {
        let candidate = Self {
            index: value.index,
            count: value.count,
            merkle_proof: value.merkle_proof,
        };

        if candidate.index > candidate.count
            || candidate.merkle_proof.len() as u64 != candidate.compute_expected_proof_length()
        {
            return Err(MerkleConstructionError::IncorrectIndexedMerkleProof);
        }
        Ok(candidate)
    }
}

#[cfg_attr(
    feature = "std",
    derive(
        PartialEq,
        Debug,
        schemars::JsonSchema,
        serde::Serialize,
        serde::Deserialize,
    ),
    serde(
        deny_unknown_fields,
        try_from = "IndexedMerkleProofDeserializeValidator"
    )
)]
pub struct IndexedMerkleProof {
    index: u64,
    count: u64,
    merkle_proof: Vec<Blake2bHash>,
}

impl IndexedMerkleProof {
    pub(crate) fn new<I>(
        leaves: I,
        index: u64,
    ) -> Result<IndexedMerkleProof, MerkleConstructionError>
    where
        I: IntoIterator<Item = Blake2bHash>,
    {
        enum HashOrProof {
            Hash(Blake2bHash),
            Proof(Vec<Blake2bHash>),
        }
        use HashOrProof::{Hash, Proof};

        let maybe_count_and_proof = leaves
            .into_iter()
            .enumerate()
            .map(|(i, hash)| {
                if i as u64 == index {
                    (1u64, Proof(vec![hash]))
                } else {
                    (1u64, Hash(hash))
                }
            })
            .tree_fold1(|(count_x, x), (count_y, y)| match (x, y) {
                (Hash(hash_x), Hash(hash_y)) => {
                    (count_x + count_y, Hash(util::hash_pair(&hash_x, &hash_y)))
                }
                (Hash(hash), Proof(mut proof)) | (Proof(mut proof), Hash(hash)) => {
                    proof.push(hash);
                    (count_x + count_y, Proof(proof))
                }
                (Proof(_), Proof(_)) => unreachable!(),
            });
        match maybe_count_and_proof {
            None => {
                if index != 0 {
                    Err(MerkleConstructionError::EmptyProofMustHaveIndex { index })
                } else {
                    Ok(IndexedMerkleProof {
                        index: 0,
                        count: 0,
                        merkle_proof: Vec::new(),
                    })
                }
            }
            Some((count, Hash(_))) => {
                Err(MerkleConstructionError::IndexOutOfBounds { count, index })
            }
            Some((count, Proof(merkle_proof))) => Ok(IndexedMerkleProof {
                index,
                count,
                merkle_proof,
            }),
        }
    }

    pub(crate) fn root_hash(&self) -> Blake2bHash {
        let IndexedMerkleProof {
            index: _,
            count,
            merkle_proof,
        } = self;

        let mut hashes = merkle_proof.into_iter();
        let raw_root = if let Some(leaf_hash) = hashes.next().cloned() {
            // Compute whether to hash left or right for the elements of the Merkle proof.
            // This gives a path to the value with the specified index.
            // We represent this path as a sequence of 64 bits. 1 here means "hash right".
            let mut path: u64 = 0;
            let mut n = self.count;
            let mut i = self.index;
            while n > 1 {
                path <<= 1;
                let pivot = 1u64 << (63 - (n - 1).leading_zeros());
                if i < pivot {
                    n = pivot;
                } else {
                    path |= 1;
                    n -= pivot;
                    i -= pivot;
                }
            }

            // Compute the raw Merkle root by hashing the proof from leaf hash up.
            let mut acc = leaf_hash;

            for hash in hashes {
                let mut hasher = VarBlake2b::new(Digest::LENGTH).unwrap();
                if (path & 1) == 1 {
                    hasher.update(hash);
                    hasher.update(&acc);
                } else {
                    hasher.update(&acc);
                    hasher.update(hash);
                }
                hasher.finalize_variable(|slice| {
                    acc.0.copy_from_slice(slice);
                });
                path >>= 1;
            }
            acc
        } else {
            util::SENTINEL2
        };

        // The Merkle root is the hash of the count with the raw root.
        util::hash_pair(count.to_le_bytes(), raw_root)
    }

    pub fn index(&self) -> u64 {
        self.index
    }
    pub fn count(&self) -> u64 {
        self.count
    }

    pub(crate) fn merkle_proof(&self) -> &[Blake2bHash] {
        &self.merkle_proof
    }

    #[cfg(test)]
    fn inject_merkle_proof(&mut self, merkle_proof: Vec<Blake2bHash>) {
        use crate::blake2b_hash::Blake2bHash;

        self.merkle_proof = merkle_proof;
    }

    // Proof lengths are never bigger than 65, so we can use a u8 here
    // The reason they are never bigger than 65 is because we are using 64 bit counts
    fn compute_expected_proof_length(&self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let mut l = 1;
        let mut n = self.count;
        let mut i = self.index;
        while n > 1 {
            let pivot = 1u64 << (63 - (n - 1).leading_zeros());
            if i < pivot {
                n = pivot;
            } else {
                n -= pivot;
                i -= pivot;
            }
            l += 1;
        }
        l
    }

    fn verify(&self) -> Result<(), MerkleVerificationError> {
        if !((self.count == 0 && self.index == 0) || self.index < self.count) {
            return Err(MerkleVerificationError::IndexOutOfBounds {
                count: self.count,
                index: self.index,
            });
        }
        let expected_proof_length = self.compute_expected_proof_length();
        if self.merkle_proof.len() != expected_proof_length as usize {
            return Err(MerkleVerificationError::UnexpectedProofLength {
                count: self.count,
                index: self.index,
                expected_proof_length,
                actual_proof_length: self.merkle_proof.len(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use proptest::prelude::{prop_assert, prop_assert_eq};
    use proptest_attr_macro::proptest;
    use rand::Rng;

    use crate::{blake2b_hash::Blake2bHash, util::blake2b_hash};

    use super::*;

    #[test]
    fn test_merkle_proofs() {
        let mut rng = rand::thread_rng();
        for _ in 0..20 {
            let leaf_count: u64 = rng.gen_range(1..100);
            let index = rng.gen_range(0..leaf_count);
            let leaves: Vec<Blake2bHash> = (0..leaf_count)
                .map(|i| blake2b_hash(i.to_le_bytes()))
                .collect();
            let root = util::hash_merkle_tree(leaves.iter().cloned());
            let indexed_merkle_proof =
                IndexedMerkleProof::new(leaves.iter().cloned(), index).unwrap();
            assert_eq!(
                indexed_merkle_proof.compute_expected_proof_length(),
                indexed_merkle_proof.merkle_proof().len() as u64
            );
            assert_eq!(indexed_merkle_proof.verify(), Ok(()));
            assert_eq!(leaf_count, indexed_merkle_proof.count);
            assert_eq!(leaves[index as usize], indexed_merkle_proof.merkle_proof[0]);
            assert_eq!(root, indexed_merkle_proof.root_hash());
        }
    }

    #[test]
    fn out_of_bounds_index() {
        let out_of_bounds_indexed_merkle_proof = IndexedMerkleProof {
            index: 23,
            count: 4,
            merkle_proof: vec![Blake2bHash([0u8; 32]); 3],
        };
        assert_eq!(
            out_of_bounds_indexed_merkle_proof.verify(),
            Err(MerkleVerificationError::IndexOutOfBounds {
                count: 4,
                index: 23
            })
        )
    }

    #[test]
    fn unexpected_proof_length() {
        let out_of_bounds_indexed_merkle_proof = IndexedMerkleProof {
            index: 1235,
            count: 5647,
            merkle_proof: vec![Blake2bHash([0u8; 32]); 13],
        };
        assert_eq!(
            out_of_bounds_indexed_merkle_proof.verify(),
            Err(MerkleVerificationError::UnexpectedProofLength {
                count: 5647,
                index: 1235,
                expected_proof_length: 14,
                actual_proof_length: 13
            })
        )
    }

    #[test]
    fn empty_unexpected_proof_length() {
        let out_of_bounds_indexed_merkle_proof = IndexedMerkleProof {
            index: 0,
            count: 0,
            merkle_proof: vec![Blake2bHash([0u8; 32]); 3],
        };
        assert_eq!(
            out_of_bounds_indexed_merkle_proof.verify(),
            Err(MerkleVerificationError::UnexpectedProofLength {
                count: 0,
                index: 0,
                expected_proof_length: 0,
                actual_proof_length: 3
            })
        )
    }

    #[test]
    fn empty_out_of_bounds_index() {
        let out_of_bounds_indexed_merkle_proof = IndexedMerkleProof {
            index: 23,
            count: 0,
            merkle_proof: vec![],
        };
        assert_eq!(
            out_of_bounds_indexed_merkle_proof.verify(),
            Err(MerkleVerificationError::IndexOutOfBounds {
                count: 0,
                index: 23
            })
        )
    }

    #[test]
    fn deep_proof_doesnt_kill_stack() {
        const PROOF_LENGTH: usize = 63;
        let indexed_merkle_proof = IndexedMerkleProof {
            index: 42,
            count: 1 << (PROOF_LENGTH - 1),
            merkle_proof: vec![Blake2bHash([0u8; Digest::LENGTH]); PROOF_LENGTH],
        };
        let _hash = indexed_merkle_proof.root_hash();
    }

    #[test]
    fn empty_proof() {
        let empty_merkle_root = util::hash_merkle_tree(vec![]);
        assert_eq!(
            empty_merkle_root,
            util::hash_pair(0u64.to_le_bytes(), util::SENTINEL2)
        );
        let indexed_merkle_proof = IndexedMerkleProof {
            index: 0,
            count: 0,
            merkle_proof: vec![],
        };
        assert_eq!(indexed_merkle_proof.verify(), Ok(()));
        assert_eq!(indexed_merkle_proof.root_hash(), empty_merkle_root);
    }

    #[proptest]
    fn expected_proof_length_le_65(index: u64, count: u64) {
        let indexed_merkle_proof = IndexedMerkleProof {
            index,
            count,
            merkle_proof: vec![],
        };
        prop_assert!(indexed_merkle_proof.compute_expected_proof_length() <= 65);
    }

    fn reference_root_from_proof(index: u64, count: u64, proof: &[Blake2bHash]) -> Blake2bHash {
        fn compute_raw_root_from_proof(
            index: u64,
            leaf_count: u64,
            proof: &[Blake2bHash],
        ) -> Blake2bHash {
            if leaf_count == 0 {
                return util::SENTINEL2;
            }
            if leaf_count == 1 {
                return proof[0].clone();
            }
            let half = 1u64 << (63 - (leaf_count - 1).leading_zeros());
            let last = proof.len() - 1;
            if index < half {
                let left = compute_raw_root_from_proof(index, half, &proof[..last]);
                util::hash_pair(&left, &proof[last])
            } else {
                let right =
                    compute_raw_root_from_proof(index - half, leaf_count - half, &proof[..last]);
                util::hash_pair(&proof[last], &right)
            }
        }

        let raw_root = compute_raw_root_from_proof(index, count, proof);
        util::hash_pair(count.to_le_bytes(), raw_root)
    }

    /// Construct an `IndexedMerkleProof` with a proof of zero Blake2bHashes.
    fn test_indexed_merkle_proof(index: u64, count: u64) -> IndexedMerkleProof {
        let mut indexed_merkle_proof = IndexedMerkleProof {
            index,
            count,
            merkle_proof: vec![],
        };
        let expected_proof_length = indexed_merkle_proof.compute_expected_proof_length();
        indexed_merkle_proof.merkle_proof = std::iter::repeat_with(|| Blake2bHash([0u8; 32]))
            .take(expected_proof_length as usize)
            .collect();
        indexed_merkle_proof
    }

    #[proptest]
    fn root_from_proof_agrees_with_recursion(index: u64, count: u64) {
        let indexed_merkle_proof = test_indexed_merkle_proof(index, count);
        prop_assert_eq!(
            indexed_merkle_proof.root_hash(),
            reference_root_from_proof(
                indexed_merkle_proof.index,
                indexed_merkle_proof.count,
                indexed_merkle_proof.merkle_proof(),
            ),
            "Result did not agree with reference implementation.",
        );
    }

    #[test]
    fn root_from_proof_agrees_with_recursion_2147483648_4294967297() {
        let indexed_merkle_proof = test_indexed_merkle_proof(2147483648, 4294967297);
        assert_eq!(
            indexed_merkle_proof.root_hash(),
            reference_root_from_proof(
                indexed_merkle_proof.index,
                indexed_merkle_proof.count,
                indexed_merkle_proof.merkle_proof(),
            ),
            "Result did not agree with reference implementation.",
        );
    }

    #[test]
    fn validates_indexed_merkle_proof_after_deserialization() {
        let indexed_merkle_proof = test_indexed_merkle_proof(10, 10);

        let json = serde_json::to_string(&indexed_merkle_proof).unwrap();
        assert_eq!(
            indexed_merkle_proof,
            serde_json::from_str::<IndexedMerkleProof>(&json)
                .expect("should deserialize correctly")
        );

        // Check that proof with index greated than count fails to deserialize
        let mut indexed_merkle_proof = test_indexed_merkle_proof(10, 10);
        indexed_merkle_proof.index += 1;
        let json = serde_json::to_string(&indexed_merkle_proof).unwrap();
        serde_json::from_str::<IndexedMerkleProof>(&json)
            .expect_err("shoud not deserialize correctly");

        // Check that proof with incorrect length fails to deserialize
        let mut indexed_merkle_proof = test_indexed_merkle_proof(10, 10);
        indexed_merkle_proof.merkle_proof.push(blake2b_hash("XXX"));
        let json = serde_json::to_string(&indexed_merkle_proof).unwrap();
        serde_json::from_str::<IndexedMerkleProof>(&json)
            .expect_err("shoud not deserialize correctly");
    }
}
