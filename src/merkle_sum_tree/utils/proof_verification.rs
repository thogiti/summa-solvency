use crate::merkle_sum_tree::utils::{
    create_middle_node::create_middle_node,
    big_int_to_fp,
};
use crate::merkle_sum_tree::{MerkleProof, Node};

pub fn verify_proof(proof: &MerkleProof) -> bool {
    let mut node = proof.entry.compute_leaf();
    let mut balance = big_int_to_fp(proof.entry.balance());

    for i in 0..proof.sibling_hashes.len() {
        let sibling_node = Node {
            hash: proof.sibling_hashes[i],
            balance: proof.sibling_sums[i],
        };

        if proof.path_indices[i] == 0.into() {
            node = create_middle_node(&node, &sibling_node);
        } else {
            node = create_middle_node(&sibling_node, &node);
        }

        balance += sibling_node.balance;
    }

    proof.root_hash == node.hash && balance == node.balance
}
