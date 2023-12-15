use std::{collections::BTreeMap, str::FromStr};

use bitcoin::{
    bip32::Fingerprint,
    hashes::Hash,
    psbt::{GetKey, Input, KeyRequest, PsbtSighashType},
    secp256k1::{All, Context, Secp256k1},
    PrivateKey, Psbt, Transaction, TxOut, Witness,
};

use crate::Account;

pub fn sign_partially(
    secp: &Secp256k1<All>,
    unsigned_tx: Transaction,
    updater_account: &Account,
    sender_account: &Account,
    previous_output: TxOut,
) -> anyhow::Result<Transaction> {
    // Creator (https://github.com/rust-bitcoin/rust-bitcoin/blob/master/bitcoin/examples/ecdsa-psbt.rs)
    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx)?;

    // updater
    let mut input = Input {
        witness_utxo: Some(previous_output),
        ..Default::default()
    };

    let mut map = BTreeMap::new();
    let fingerprint_bytes = updater_account
        .public_key
        .pubkey_hash()
        .as_byte_array()
        .to_vec();
    let fingerprint: &[u8; 4] = fingerprint_bytes[..4].try_into().unwrap();
    let fingerprint = Fingerprint::from(fingerprint);
    map.insert(
        updater_account.private_key.inner.public_key(secp),
        (fingerprint, updater_account.path.clone()),
    );
    input.bip32_derivation = map;

    let ty = PsbtSighashType::from_str("SIGHASH_ALL")?;
    input.sighash_type = Some(ty);

    psbt.inputs = vec![input];
    debug!("unsigned psbt: {psbt:#?}");

    // sign
    for account in vec![sender_account] {
        let key = KeyRetriever(account.private_key.clone());
        match psbt.sign(&key, secp) {
            Ok(keys) if keys.len() == 1 => {}
            Ok(_) => anyhow::bail!("unexpected number of keys"),
            Err(_) => anyhow::bail!("signing failed"),
        }
    }

    debug!("signed psbt: {psbt:#?}");

    // Push witness
    let sigs: Vec<_> = psbt.inputs[0].partial_sigs.iter().collect();
    let script_witness = Witness::p2wpkh(&sigs[0].1, &sigs[0].0.inner);

    // FINALIZER
    psbt.inputs[0].final_script_witness = Some(script_witness);

    // Clear all the data fields as per the spec.
    debug!("finalized psbt: {psbt:#?}");
    psbt.inputs[0].partial_sigs = BTreeMap::new();
    psbt.inputs[0].sighash_type = None;
    psbt.inputs[0].redeem_script = None;
    psbt.inputs[0].witness_script = None;
    psbt.inputs[0].bip32_derivation = BTreeMap::new();

    Ok(psbt.extract_tx_fee_rate_limit()?)
}

pub struct KeyRetriever(PrivateKey);

impl GetKey for KeyRetriever {
    type Error = anyhow::Error;

    fn get_key<C: Context>(
        &self,
        _fingerprint: KeyRequest,
        _secp: &Secp256k1<C>,
    ) -> Result<Option<PrivateKey>, Self::Error> {
        Ok(Some(self.0.clone()))
    }
}
