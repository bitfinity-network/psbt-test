#[macro_use]
extern crate log;

mod psbt;
mod rpc_client;
mod signer;
mod taproot;
mod utils;

use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::opcodes::all::{OP_CHECKSIG, OP_ENDIF, OP_IF};
use bitcoin::opcodes::{OP_0, OP_FALSE};
use bitcoin::script::Builder as ScriptBuilder;
use bitcoin::transaction::Version;
use bitcoin::{
    secp256k1::{All, Secp256k1},
    Address, Amount, PrivateKey, PublicKey, Txid,
};
use bitcoin::{Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use ord_rs::Inscription as _;
use ord_rs::{transaction::TxInput, OrdError};

use crate::utils::bytes_to_push_bytes;

/// tb1qzc8dhpkg5e4t6xyn4zmexxljc4nkje59dg3ark
const SENDER_ADDRESS_WIF: &str = "cVkWbHmoCx6jS8AyPNQqvFr8V9r2qzDHJLaxGDQgDJfxT73w6fuU";
/// tb1qax89amll2uas5k92tmuc8rdccmqddqw94vrr86
const RECIPIENT_ADDRESS_WIF: &str = "cVpCkgJsvYyCuARLa35jkxmXJoJW2gCbuV5eMNeEESpWz1pUhyub";
/// tb1qcwflhw3252daxhj6d40wxpuard5c05lzqptdx7
const MARKETPLACE_ADDRESS_WIF: &str = "cR1HwkKsYpMxbAzJNzrW8hpBBRV11G5zb5RQfKzD4bCHkSwrZw7x";

const COMMIT_FEE: u64 = 2_500;
const REVEAL_FEE: u64 = 4_700;
const POSTAGE: u64 = 333;

#[derive(Debug, Clone)]
pub struct Account {
    pub address: Address,
    pub public_key: PublicKey,
    pub private_key: PrivateKey,
}

impl Account {
    pub fn from_wif(secp: &Secp256k1<All>, wif: &str) -> anyhow::Result<Self> {
        let private_key = PrivateKey::from_wif(wif)?;
        let public_key = private_key.public_key(secp);
        let address = Address::p2wpkh(&public_key, bitcoin::Network::Testnet)?;

        Ok(Self {
            address,
            public_key,
            private_key,
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

    // input to use
    let tx_input = TxInput {
        id: Txid::from_str("14a7109b642b4fca7f10cd9bee89db73770c5a2d107f6a51c6bd7625dcdc2aed")
            .unwrap(),
        index: 0,
        amount: Amount::from_sat(8_000),
    };
    // inscription
    let inscription = ord_rs::brc20::Brc20::deploy("omar", 8_888_000, Some(1_000), None);

    // prepare commit
    let (p2tr_keypair, p2tr_pubkey) = taproot::generate_keypair(&secp);

    // calc balance
    // exceeding amount of transaction to send to leftovers recipient
    let leftover_amount = tx_input
        .amount
        .to_sat()
        .checked_sub(POSTAGE)
        .and_then(|v| v.checked_sub(COMMIT_FEE))
        .and_then(|v| v.checked_sub(REVEAL_FEE))
        .ok_or(OrdError::InsufficientBalance)?;
    debug!("leftover_amount: {leftover_amount}");

    let reveal_balance = POSTAGE + REVEAL_FEE;

    // prepare redeem script
    let redeem_script = ScriptBuilder::new()
        .push_slice(bytes_to_push_bytes(&p2tr_pubkey.serialize())?.as_push_bytes())
        .push_opcode(OP_CHECKSIG)
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF)
        .push_slice(b"ord")
        .push_slice(b"\x01")
        .push_slice(bytes_to_push_bytes(inscription.content_type().as_bytes())?.as_push_bytes())
        .push_opcode(OP_0)
        .push_slice(inscription.data()?.as_push_bytes())
        .push_opcode(OP_ENDIF)
        .into_script();

    // make taproot payload
    let taproot_payload = taproot::TaprootPayload::build(
        &secp,
        p2tr_keypair,
        p2tr_pubkey,
        &redeem_script,
        reveal_balance,
        Network::Testnet,
    )?;

    // make txout
    let tx_out = vec![
        TxOut {
            value: Amount::from_sat(reveal_balance),
            script_pubkey: taproot_payload.address.script_pubkey(),
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
    let partially_signed_tx = psbt::sign_partially(
        &secp,
        unsigned_tx,
        &[sender.clone(), recipient],
        TxOut {
            value: tx_input.amount,
            script_pubkey: sender.address.script_pubkey(),
        },
        &redeem_script,
    )?;
    let mut signer = signer::Signer::new(&marketplace.private_key, &secp, partially_signed_tx);
    let signed_tx = signer.sign_commit_transaction(&[tx_input], &sender.address.script_pubkey())?;
    debug!("signed_tx: {signed_tx:?}");

    // broadcast transaction
    let txid = rpc_client::broadcast_transaction(&signed_tx, Network::Testnet).await?;
    rpc_client::wait_for_tx(&txid, Network::Testnet).await?;
    println!("Commit tx: https://mempool.space/testnet/tx/{txid}");

    // make reveal and broadcast it
    // ...

    Ok(())
}
