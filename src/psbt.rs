use std::{collections::BTreeMap, str::FromStr};

use bitcoin::{
    psbt::{Input, PsbtSighashType},
    secp256k1::{All, Secp256k1},
    Psbt, ScriptBuf, Transaction, TxOut,
};

use crate::Account;

pub fn sign_partially(
    secp: &Secp256k1<All>,
    unsigned_tx: Transaction,
    accounts: &[Account],
    previous_output: TxOut,
    redeem_script: &ScriptBuf,
) -> anyhow::Result<Transaction> {
    // Creator (https://github.com/rust-bitcoin/rust-bitcoin/blob/master/bitcoin/examples/ecdsa-psbt.rs)
    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx)?;
    debug!("unsigned psbt: {psbt:?}");

    // updater
    let mut input = Input {
        witness_utxo: Some(previous_output),
        ..Default::default()
    };

    input.redeem_script = Some(redeem_script.clone());

    let ty = PsbtSighashType::from_str("SIGHASH_ALL")?;
    input.sighash_type = Some(ty);

    psbt.inputs = vec![input];

    // sign
    let mut sigs = BTreeMap::new();
    for account in accounts {
        sigs.insert(account.public_key, account.private_key);
    }
    match psbt.sign(&sigs, secp) {
        Ok(keys) => assert_eq!(keys.len(), 1),
        Err((_, e)) => {
            anyhow::bail!("signing failed: {:?}", e);
        }
    };

    Ok(psbt.extract_tx_fee_rate_limit()?)
}
