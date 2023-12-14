#[macro_use]
extern crate log;

mod psbt;
mod rpc_client;
mod signer;
mod taproot;
mod utils;

use std::str::FromStr;

use bip39::Mnemonic;
use bitcoin::absolute::LockTime;
use bitcoin::bip32::ChildNumber;
use bitcoin::bip32::DerivationPath;
use bitcoin::bip32::Xpriv;
use bitcoin::bip32::Xpub;
use bitcoin::transaction::Version;
use bitcoin::PrivateKey;
use bitcoin::{
    secp256k1::{All, Secp256k1},
    Address, Amount, PublicKey, Txid,
};
use bitcoin::{Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use ord_rs::{transaction::TxInput, OrdError};

/// tb1qzc8dhpkg5e4t6xyn4zmexxljc4nkje59dg3ark
const SENDER_ADDRESS_WIF: &str = "cVkWbHmoCx6jS8AyPNQqvFr8V9r2qzDHJLaxGDQgDJfxT73w6fuU";
/// tb1qdnsst69uf5gg0mfyp0ercw3k5gew9qc5m8xp96
const RECIPIENT_ADDRESS_WIF: &str = "cVonwZaHFNWck31inrgNUEFdSE3rSagWKSZLaYqRvfhVELbQsBH7";
/// tb1qpwhuavg7ht5mwsrkwfn2sgderlkwrp5eumtjc6
const MARKETPLACE_ADDRESS_WIF: &str = "cMaoE2kaUQEp6BiAZ143UdH8LZycvF3nn17mR7Si9TssAczJ5d6V";

const AMOUNT: u64 = 1_000;
const FEE: u64 = 3_000;

#[derive(Debug, Clone)]
pub struct Account {
    pub address: Address,
    pub public_key: PublicKey,
    pub private_key: PrivateKey,
    pub path: DerivationPath,
}

impl Account {
    pub fn from_wif(secp: &Secp256k1<All>, wif: &str) -> anyhow::Result<Self> {
        let private_key = PrivateKey::from_wif(wif)?;
        let public_key = private_key.public_key(secp);
        let address = Address::p2wpkh(&public_key, private_key.network)?;

        // derive child xpub
        let path = DerivationPath::from_str("m")?;

        Ok(Self {
            address,
            public_key,
            private_key,
            path,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let secp = Secp256k1::new();
    // setup accounts
    let sender = Account::from_wif(&secp, SENDER_ADDRESS_WIF)?;
    let recipient = Account::from_wif(&secp, RECIPIENT_ADDRESS_WIF)?;
    let marketplace = Account::from_wif(&secp, MARKETPLACE_ADDRESS_WIF)?;

    debug!("sender: {}", sender.address);
    debug!("recipient: {}", recipient.address);
    debug!("marketplace: {}", marketplace.address);

    // input to use
    let tx_input = TxInput {
        id: Txid::from_str("679b42bcb193e7b85ed5b2378aaa2d0cb1b9c600d1c22475598d8bafa171f8d9")
            .unwrap(),
        index: 1,
        amount: Amount::from_sat(8_000),
    };

    // calc balance
    // exceeding amount of transaction to send to leftovers recipient
    let leftover_amount = tx_input
        .amount
        .to_sat()
        .checked_sub(AMOUNT)
        .and_then(|v| v.checked_sub(FEE))
        .ok_or(OrdError::InsufficientBalance)?;
    debug!("leftover_amount: {leftover_amount}");

    // make txout
    let tx_out = vec![
        TxOut {
            value: Amount::from_sat(AMOUNT),
            script_pubkey: recipient.address.script_pubkey(),
        },
        TxOut {
            value: Amount::from_sat(leftover_amount),
            script_pubkey: sender.address.script_pubkey(),
        },
    ];

    // make txin
    let tx_in = vec![TxIn {
        previous_output: OutPoint {
            txid: tx_input.id,
            vout: tx_input.index,
        },
        script_sig: ScriptBuf::new(),
        sequence: Sequence::from_consensus(0xffffffff),
        witness: Witness::new(),
    }];

    // make transaction and sign it
    let unsigned_tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: tx_in,
        output: tx_out,
    };
    // https://github.com/bitcoin/bitcoin/blob/master/doc/psbt.md
    let partially_signed_tx = psbt::sign_partially(
        &secp,
        unsigned_tx,
        &marketplace,
        &sender,
        TxOut {
            value: tx_input.amount,
            script_pubkey: sender.address.script_pubkey(),
        },
    )?;
    debug!("partially_signed_tx: {partially_signed_tx:?}");

    // broadcast transaction
    let txid = rpc_client::broadcast_transaction(&partially_signed_tx, Network::Testnet).await?;
    rpc_client::wait_for_tx(&txid, Network::Testnet).await?;
    println!("Commit tx: https://mempool.space/testnet/tx/{txid}");

    // make reveal and broadcast it
    // ...

    Ok(())
}
