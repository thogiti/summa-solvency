use ethers::types::{Bytes, U256};
use halo2_proofs::{
    arithmetic::Field,
    halo2curves::bn256::{Bn256, Fr as Fp, G1Affine},
    halo2curves::group::Curve,
    plonk::{AdviceSingle, ProvingKey, VerifyingKey},
    poly::{
        kzg::commitment::{KZGCommitmentScheme, ParamsKZG},
        Coeff,
    },
};
use serde::{Deserialize, Serialize};
use std::error::Error;

use crate::contracts::signer::SummaSigner;
use summa_solvency::{
    circuits::{univariate_grand_sum::UnivariateGrandSum, utils::generate_setup_artifacts},
    entry::Entry,
    utils::{
        amortized_kzg::{commit_kzg, create_naive_kzg_proof, verify_kzg_proof},
        big_uint_to_fp,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KZGInclusionProof {
    public_inputs: Vec<U256>,
    proof_calldata: Bytes,
}

impl KZGInclusionProof {
    pub fn get_public_inputs(&self) -> &Vec<U256> {
        &self.public_inputs
    }

    pub fn get_proof(&self) -> &Bytes {
        &self.proof_calldata
    }
}

/// The `Round` struct represents a single operational cycle within the Summa Solvency protocol.
///
/// # Type Parameters
///
/// * `N_CURRENCIES`: The number of currencies for which solvency is verified in this round.
/// * `N_POINTS`: The number of points in the `UnivariateGrandSum` circuit, which is `N_CURRENCIES + 1`.
/// * `N_USERS`: The number of users involved in this round of the protocol.
///
/// These parameters are used for initializing the `UnivariateGrandSum` circuit within the `Snapshot` struct.
///
/// # Fields
///
/// * `timestamp`: A Unix timestamp marking the initiation of this round. It serves as a temporal reference point
///   for the operations carried out in this phase of the protocol.
/// * `snapshot`: A `Snapshot` struct capturing the round's state, including user identities and balances.
/// * `signer`: A reference to a `SummaSigner`, the entity responsible for signing transactions with the Summa contract in this round.
pub struct Round<'a, const N_CURRENCIES: usize, const N_POINTS: usize, const N_USERS: usize> {
    timestamp: u64,
    snapshot: Snapshot<N_CURRENCIES, N_POINTS, N_USERS>,
    signer: &'a SummaSigner,
}

impl<const N_CURRENCIES: usize, const N_POINTS: usize, const N_USERS: usize>
    Round<'_, N_CURRENCIES, N_POINTS, N_USERS>
where
    [usize; N_CURRENCIES + 1]: Sized,
{
    pub fn new<'a>(
        signer: &'a SummaSigner,
        advice_polys: AdviceSingle<G1Affine, Coeff>,
        entries: Vec<Entry<N_CURRENCIES>>,
        params_path: &str,
        timestamp: u64,
    ) -> Result<Round<'a, N_CURRENCIES, N_POINTS, N_USERS>, Box<dyn Error>> {
        Ok(Round {
            timestamp,
            snapshot: Snapshot::<N_CURRENCIES, N_POINTS, N_USERS>::new(
                advice_polys,
                entries,
                params_path,
            )
            .unwrap(),
            signer: &signer,
        })
    }

    pub fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    // TODO: What will be the commit on the V2?
    pub async fn dispatch_commitment(&mut self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    pub fn get_proof_of_inclusion(&self, user_index: u16) -> Result<KZGInclusionProof, &'static str>
    where
        [(); N_CURRENCIES + 1]: Sized,
    {
        // Iterate unblinded advice polynomials evaluate balances array
        Ok(self
            .snapshot
            .generate_proof_of_inclusion(user_index, &self.snapshot.entries)
            .unwrap())
    }
}

/// The `Snapshot` struct represents the state of database that contains users balance on holds by Custodians at a specific moment.
///
/// # Fields
///
/// * `advice_polys`: Composed of the unblinded advice polynomial, `advice_poly`, and the polynomials of blind factors, `advice_blind`.
/// * `user_balances`: A 2D array of user identity and balances.
/// * `trusted_setup`: The trusted setup artifacts generated from the `UnivariateGrandSum` circuit.
///
pub struct Snapshot<const N_CURRENCIES: usize, const N_POINTS: usize, const N_USERS: usize> {
    advice_polys: AdviceSingle<G1Affine, Coeff>,
    entries: Vec<Entry<N_CURRENCIES>>,
    trusted_setup: (
        ParamsKZG<Bn256>,
        ProvingKey<G1Affine>,
        VerifyingKey<G1Affine>,
    ),
}

impl<const N_CURRENCIES: usize, const N_POINTS: usize, const N_USERS: usize>
    Snapshot<N_CURRENCIES, N_POINTS, N_USERS>
where
    [usize; N_CURRENCIES + 1]: Sized,
{
    pub fn new(
        advice_polys: AdviceSingle<G1Affine, Coeff>,
        entries: Vec<Entry<N_CURRENCIES>>,
        params_path: &str,
    ) -> Result<Snapshot<N_CURRENCIES, N_POINTS, N_USERS>, Box<dyn Error>> {
        let univariate_grand_sum_circuit: UnivariateGrandSum<N_USERS, N_CURRENCIES> =
            UnivariateGrandSum::<N_USERS, N_CURRENCIES>::init_empty();

        // get k from ptau file name
        let parts: Vec<&str> = params_path.split('-').collect();
        let last_part = parts.last().unwrap();
        let k = last_part.parse::<u32>().unwrap();

        let univariant_grand_sum_setup_artifcats =
            generate_setup_artifacts(k, Some(params_path), &univariate_grand_sum_circuit).unwrap();

        Ok(Snapshot {
            advice_polys,
            entries,
            trusted_setup: univariant_grand_sum_setup_artifcats,
        })
    }

    pub fn generate_proof_of_inclusion(
        &self,
        user_index: u16,
        entries: &[Entry<N_CURRENCIES>],
    ) -> Result<KZGInclusionProof, &'static str>
    where
        [(); N_CURRENCIES + 1]: Sized, // TODO: check is this necessary to compile?
    {
        let (params, _, vk) = &self.trusted_setup;
        let omega: halo2_proofs::halo2curves::grumpkin::Fq = vk.get_domain().get_omega();

        let column_range = 0..N_CURRENCIES + 1;
        let mut opening_proofs = Vec::new();
        for column_index in column_range {
            let f_poly = self.advice_polys.advice_polys.get(column_index).unwrap();
            let kzg_commitment = commit_kzg(&params, f_poly);

            let challenge = omega.pow_vartime([user_index as u64]);

            let mut z: Fp = Fp::zero();
            let user_entry = entries.get(user_index as usize).unwrap();
            if column_index == 0 {
                z = big_uint_to_fp(user_entry.username_as_big_uint());
            } else {
                let user_balances = user_entry.balances();
                z = big_uint_to_fp(user_balances.get(column_index - 1).unwrap());
            }

            let kzg_proof = create_naive_kzg_proof::<KZGCommitmentScheme<Bn256>>(
                &params,
                vk.get_domain(),
                f_poly,
                challenge,
                z,
            );

            assert!(
                verify_kzg_proof(&params, kzg_commitment, kzg_proof, &challenge, &z),
                "KZG proof verification failed for user {}",
                user_index
            );

            // Convert to affine point and serialize to bytes
            let kzg_proof_affine = kzg_proof.to_affine();
            let mut kzg_proof_affine_x = kzg_proof_affine.x.to_bytes();
            let mut kzg_proof_affine_y = kzg_proof_affine.y.to_bytes();
            kzg_proof_affine_x.reverse();
            kzg_proof_affine_y.reverse();

            opening_proofs.push([kzg_proof_affine_x, kzg_proof_affine_y].concat());
        }

        Ok(KZGInclusionProof {
            proof_calldata: Bytes::from(opening_proofs.concat()),
            public_inputs: Vec::<U256>::new(),
        })
    }
}
