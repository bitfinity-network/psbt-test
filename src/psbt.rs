use std::{collections::BTreeMap, str::FromStr};

use bitcoin::{
    psbt::{Input, PsbtSighashType},
    secp256k1::{All, Secp256k1},
    Psbt, ScriptBuf, Transaction, TxOut, Witness,
};

use crate::Account;

pub fn sign_partially(
    secp: &Secp256k1<All>,
    unsigned_tx: Transaction,
    updater_account: &Account,
    accounts: &[Account],
    previous_output: TxOut,
) -> anyhow::Result<Transaction> {
    // Creator (https://github.com/rust-bitcoin/rust-bitcoin/blob/master/bitcoin/examples/ecdsa-psbt.rs)
    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx)?;

    // updater
    let mut input = Input {
        witness_utxo: Some(previous_output),
        ..Default::default()
    };

    let pk = updater_account.input_xpub.to_pub();
    let wpkh = pk.wpubkey_hash().expect("a compressed pubkey");

    let redeem_script = ScriptBuf::new_p2wpkh(&wpkh);
    input.redeem_script = Some(redeem_script);

    let fingerprint = updater_account.private_key.fingerprint(secp);
    let mut map = BTreeMap::new();
    map.insert(pk.inner, (fingerprint, updater_account.path.clone()));
    input.bip32_derivation = map;

    let ty = PsbtSighashType::from_str("SIGHASH_ALL")?;
    input.sighash_type = Some(ty);

    psbt.inputs = vec![input];
    debug!("unsigned psbt: {psbt:#?}");

    // sign
    match psbt.sign(&updater_account.private_key, secp) {
        Ok(keys) if keys.len() == 1 => {}
        Ok(_) => anyhow::bail!("unexpected number of keys"),
        Err(_) => anyhow::bail!("signing failed"),
    }
    for account in accounts {
        match psbt.sign(&account.private_key, secp) {
            Ok(keys) if keys.len() == 1 => {}
            Ok(_) => anyhow::bail!("unexpected number of keys"),
            Err(_) => anyhow::bail!("signing failed"),
        }
    }

    debug!("signed psbt: {psbt:#?}");

    // Push witness
    let sigs: Vec<_> = psbt.inputs[0].partial_sigs.values().collect();
    let mut script_witness = Witness::new();
    script_witness.push(&sigs[0].to_vec());
    script_witness.push(updater_account.input_xpub.to_pub().to_bytes());

    // Clear all the data fields as per the spec.
    debug!("finalized psbt: {psbt:#?}");
    psbt.inputs[0].partial_sigs = BTreeMap::new();
    psbt.inputs[0].sighash_type = None;
    psbt.inputs[0].redeem_script = None;
    psbt.inputs[0].witness_script = None;
    psbt.inputs[0].bip32_derivation = BTreeMap::new();

    psbt.inputs[0].final_script_witness = Some(script_witness);

    Ok(psbt.extract_tx_fee_rate_limit()?)
}
