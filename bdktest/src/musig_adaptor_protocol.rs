use crate::{ProtocolRole, TestWallet, P_A_STRING, Q_A_STRING};
use anyhow::anyhow;
use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::absolute::LockTime;
use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::{Address, Amount, FeeRate, Psbt, Sequence, Txid, Weight};
use bdk_wallet::bitcoin::{KnownHrp, PublicKey, ScriptBuf};
use bdk_wallet::coin_selection::BranchAndBoundCoinSelection;
use bdk_wallet::{SignOptions, TxBuilder};
use musig2::KeyAggContext;
use secp::{Point, Scalar};

use bdk_wallet::bitcoin::key::Secp256k1;
use bdk_wallet::miniscript::ToPublicKey;
use std::str::FromStr;

/**
This is not only testing code.
It also shows how the Java FSM is supposed to call this library

Of course the tests do not need to be replicated in Java. Nor the Nigiri code.
*/
#[test]
fn test_musig() -> anyhow::Result<()> {
    println!("running...");
    crate::nigiri::check_start();
    let mut alice_funds = TestWallet::new()?;
    // crate::nigiri::funded_wallet();
    let bob_funds = TestWallet::new()?; //crate::nigiri::funded_wallet();
    // crate::nigiri::fund_wallet(&mut alice_funds);
    let seller_amount = &Amount::from_btc(1.4)?;
    let buyer_amount = &Amount::from_btc(0.2)?;

    let mut alice = BMPContext::new(alice_funds, ProtocolRole::Seller, seller_amount, buyer_amount)?;
    let mut bob = BMPContext::new(bob_funds, ProtocolRole::Buyer, seller_amount, buyer_amount)?;

    // Round 1--------
    let alice_response = alice.round1()?;
    let bob_response = bob.round1()?;

    // Round2 -------
    let alice_r2 = alice.round2(bob_response)?;
    let bob_r2 = bob.round2(alice_response)?;

    println!("P2TR P' {}", alice.p_tik.get_agg_adr()?.to_string());
    println!("P2TR Q' {}", alice.q_tik.get_agg_adr()?.to_string());

    assert!(alice.get_p_tik_agg() == bob.get_p_tik_agg());
    assert!(alice.q_tik.agg_point == bob.q_tik.agg_point);

    // Round 3 ----------
    // let alice_r3 = alice.round3(bob_r2)?;
    // let bob_r3 = bob.round3(alice_r2)?;

    Ok(())
}
pub struct Round1Parameter {
    p_a: Point,
    q_a: Point,
}
pub(crate) struct Round2Parameter {
    pub p_agg: Point,
    pub q_agg: Point,
}
pub(crate) struct Round3Parameter {}
/**
this context is for the whole process and need to be persisted by the caller
*/
pub struct BMPContext<'a> {
    // first of all, everything which is general to the protcol itself
    funds: TestWallet,
    role: ProtocolRole,
    seller_amount: &'a Amount,
    buyer_amount: &'a Amount,
    round: u8, // which round are we in.
    //-----
    p_tik: AggKey, // Point securing Seller deposit and trade amount
    q_tik: AggKey, // Point securing Buyer deposit
}

impl BMPContext<'_> {
    pub(crate) fn new<'a>(funds: TestWallet, role: ProtocolRole, seller_amount: &'a Amount, buyer_amount: &'a Amount) -> anyhow::Result<BMPContext<'a>> {
        Ok(BMPContext { funds, role, seller_amount, buyer_amount, round: 0, p_tik: AggKey::new()?, q_tik: AggKey::new()? })
    }

    pub(crate) fn round1(&mut self) -> anyhow::Result<Round1Parameter> {
        self.check_round(1);

        Ok(Round1Parameter {
            p_a: self.p_tik.get_point(),
            q_a: self.q_tik.get_point(),
        })
    }

    pub(crate) fn round2(&mut self, par: Round1Parameter) -> anyhow::Result<Round2Parameter> {
        self.check_round(2);
        assert_ne!(par.p_a, par.q_a, "Bob is sending the same point for P' and Q'.");

        self.p_tik.aggregate_key(par.p_a)?;
        self.q_tik.aggregate_key(par.q_a)?;
        // now we have the aggregated key


        Ok(Round2Parameter {
            p_agg: self.p_tik.agg_point.unwrap(),
            q_agg: self.q_tik.agg_point.unwrap(),
        })
    }
    pub(crate) fn round3(&mut self, par: Round2Parameter) -> anyhow::Result<Round3Parameter> {
        Err(anyhow!("not implemented"))
    }

    fn check_round(&mut self, round: u8) {
        if self.round != round - 1 {
            panic!("round already done");
        }
        self.round = round;
    }

    // ------- Debug --------
    pub(crate) fn get_p_tik_agg(&self) -> Address {
        let r = &(*self).p_tik;
        r.get_agg_adr().unwrap()
    }
}
/**
MuSig2 interaction, it represents the Key but only our side of the equation
*/

#[derive(PartialEq)]
struct AggKey {
    sec: Scalar,
    other_sec: Option<Scalar>,
    agg_sec: Option<Scalar>,
    pub_point: Point,
    other_point: Option<Point>,
    agg_point: Option<Point>,
}

impl AggKey {
    fn new() -> anyhow::Result<AggKey> {
        let sec: Scalar = Scalar::random(&mut rand::thread_rng());
        let point = sec.base_point_mul();
        // let pubkey = PublicKey::from_slice(&*(point.serialize()))?;
        Ok(AggKey { sec, other_sec: None, agg_sec: None, pub_point: point, other_point: None, agg_point: None })
    }

    fn get_point(&self) -> Point {
        self.pub_point
    }

    fn aggregate_key(&mut self, point_from_bob: Point) -> anyhow::Result<Point> {
        assert_ne!(point_from_bob, self.pub_point, "Bob is sending my point back.");
        let pubkeys = if self.pub_point < point_from_bob {
            [self.pub_point, point_from_bob]
        } else {
            [point_from_bob, self.pub_point]
        };
        let result = KeyAggContext::new(pubkeys)?.aggregated_pubkey();
        self.agg_point = Some(result);
        Ok(result)
    }

    // check https://bitcoin.stackexchange.com/questions/116384/what-are-the-steps-to-convert-a-private-key-to-a-taproot-address
    fn get_agg_adr(&self) -> anyhow::Result<Address> {
        let pubkey = PublicKey::from_slice(&(self.agg_point.unwrap().serialize()))?.to_x_only_pubkey();
        let secp = Secp256k1::new();
        let adr = Address::p2tr(&secp, pubkey, None, KnownHrp::Regtest);
        Ok(adr)
    }
}

pub struct MusigProtocol<'a> {
    funds: TestWallet,
    role: ProtocolRole,
    // Alice pubkey for the seller multisig
    p_pub: Address,
    // Alice pubkey for the buyer multisig
    q_pub: Address,
    seller_amount: &'a Amount,
    buyer_amount: &'a Amount,
}

impl MusigProtocol<'_> {
    pub(crate) fn new<'a>(funds: TestWallet, role: ProtocolRole, seller_amount: &'a Amount, buyer_amount: &'a Amount) -> anyhow::Result<MusigProtocol<'a>> {
        let p_pub = Address::from_str(P_A_STRING)?.assume_checked(); // TODO replace by ephemeral key generating Point
        let q_pub = Address::from_str(Q_A_STRING)?.assume_checked();
        Ok(MusigProtocol { funds, role, p_pub, q_pub, seller_amount, buyer_amount })
    }

    pub(crate) fn generate_part_tx(&mut self) -> anyhow::Result<Psbt> {
        let (funded_by_me, amount) = match self.role {
            ProtocolRole::Seller => (&self.p_pub, self.seller_amount),
            ProtocolRole::Buyer => (&self.q_pub, self.buyer_amount),
        };
        // create and fund a (virtual) transaction which funds Alice part of the Deposit Tx
        let mut builder = self.funds.wallet.build_tx();
        builder.add_recipient(
            funded_by_me.script_pubkey(), *amount,
        );
        builder.fee_rate(FeeRate::from_sat_per_vb(20).unwrap()); // TODO feerates shall come from pricenodes
        let pbst = builder.finish()?;
        // dbg!(&pbst.unsigned_tx.output);
        Ok(pbst)
    }

    pub(crate) fn build_and_merge_tx(&mut self, my_psbt: &Psbt, other_psbt: &Psbt) -> anyhow::Result<Psbt> {
        //sanity check that Bob doesn't send UTXOs owned by alice.
        for pbst_input in other_psbt.inputs.iter() {
            let scriptbuf = pbst_input.clone().witness_utxo.unwrap().script_pubkey;
            if self.funds.wallet.is_mine(scriptbuf.clone()) {
                // bob is trying to trick me.
                panic!("Fraud detected. Bob send me my own scriptbuf {:?}", scriptbuf)
            }
        }
        // TODO sanity check if bobs transaction actually calculate the fee correctly, otherwise he could save on fess at the expense of alice

        // recreate combined ty from scratch
        let mut builder = self.funds.wallet.build_tx();
        builder.manually_selected_only(); // only use inputs we have already identified.
        builder.set_exact_sequence(Sequence::MAX); // no RBF, RBF disabled for foreign utxos anyway.
        // keep track of the fees, as total fees is not equal to sum of fess from alice and bob psbts.
        let mut total: i64 = 0; // measured in sats
        // add deposit outputs first.
        builder.add_recipient(self.p_pub.script_pubkey(), *self.seller_amount);
        total = total - self.seller_amount.to_sat() as i64;

        builder.add_recipient(self.q_pub.script_pubkey(), *self.buyer_amount); // TODO make amounts variable
        total = total - self.buyer_amount.to_sat() as i64;

        let disregard_scripts = &[self.p_pub.script_pubkey(), self.q_pub.script_pubkey()];
        total = builder.merge(my_psbt, false, total, disregard_scripts)?;
        total = builder.merge(other_psbt, true, total, disregard_scripts)?;

        builder.fee_absolute(Amount::from_sat(total as u64));
        builder.nlocktime(LockTime::ZERO); // TODO RBF disabled anyway, so this value can be disregarded.

        // Attempt to finish and return the merged PSBT
        let mut merged_psbt = builder
            .finish()
            .map_err(|e| anyhow!("Failed to build transaction: {:?}", e))?;

        // We need to sort the order of inputs and output to make the TXid of alice nd bod equal
        // TODO come up with a randomized sort to preserve privacy
        // TODO use builder.ordering() with custom alg.
        sort(&mut merged_psbt);

        // sign my psbt
        self.funds.wallet.sign(&mut merged_psbt, SignOptions::default())?;
        Ok(merged_psbt)
    }

    // bobs psbt should be same as mine but have bob's sig
    pub(crate) fn transfer_sig_and_broadcast(&mut self, my_psbt: &Psbt, other_psbt: &Psbt) -> anyhow::Result<Txid> {
        let mut my_psbt = my_psbt.clone();
        // I expect to find all sigs missing in psbt_alice to be in psbt_bob
        // also I expect that both psbts are the same exect for the sigs.
        dbg!(&my_psbt.unsigned_tx);
        dbg!(&other_psbt.unsigned_tx);
        assert!(my_psbt.unsigned_tx == other_psbt.unsigned_tx);

        for (i, alice_input) in my_psbt.inputs.iter_mut().enumerate() {
            if alice_input.final_script_witness.is_none() {
                alice_input.final_script_witness =
                    Some(other_psbt.inputs[i].final_script_witness.clone().unwrap()); //must exist
            }
        }
        let tx = my_psbt.extract_tx()?;
        // TODO both, alice and bob will broadcast, is that a bug or a feature?
        self.funds.client.transaction_broadcast(&tx)?;
        Ok(tx.compute_txid())
    }
}

/*
why do i have to sort the inputs and outputs?
Alice and Bob both create the transaction, if the transaction hasn't the exact same Txid, the trnasactions will not be viewed as the same.
And for the Txid the ordering of inputs and output does count.
Note: This sort algo needs some care to make it privacy preserving.
*/
fn sort(psbt: &mut Psbt) {
    // need to sort unconfirmed tx as well
    let psbt2 = psbt.clone();
    // Sort the inputs in `psbt.inputs` and `psbt.unsigned_tx.input` while ensuring their indexes stay aligned
    let mut input_pairs: Vec<_> = psbt2.inputs.iter().zip(psbt2.unsigned_tx.input.iter()).collect();
    input_pairs.sort_by_key(|(_, tx_input)| tx_input.previous_output); // TODO sort criteria should be random for more privacy

    // Reassign the sorted pairs back to their respective components
    psbt.inputs = input_pairs.iter().map(|(input, _)| (*input).clone()).collect();
    psbt.unsigned_tx.input = input_pairs.iter().map(|(_, tx_input)| (*tx_input).clone()).collect();

    // Sort the output in `psbt.inputs` and `psbt.unsigned_tx.output` while ensuring their indexes stay aligned
    let mut output_pair: Vec<_> = psbt2.outputs.iter().zip(psbt2.unsigned_tx.output.iter()).collect();
    output_pair.sort_by_key(|(_, tx_output)| tx_output.script_pubkey.clone()); // TODO sort criteria should be random for more privacy

    // Reassign the sorted pairs back to their respective components
    psbt.outputs = output_pair.iter().map(|(output, _)| (*output).clone()).collect();
    psbt.unsigned_tx.output = output_pair.iter().map(|(_, tx_output)| (*tx_output).clone()).collect();
}

trait Merge {
    fn merge(&mut self, psbt: &Psbt, foreign: bool, total: i64, disregard_scripts: &[ScriptBuf]) -> anyhow::Result<i64>;
}

impl Merge for TxBuilder<'_, BranchAndBoundCoinSelection> {
    fn merge(&mut self, psbt: &Psbt, foreign: bool, mut total: i64, disregard_scripts: &[ScriptBuf]) -> anyhow::Result<i64> {
        for (index, psbt_input) in psbt.inputs.iter().enumerate() {
            let op = psbt.unsigned_tx.input[index].previous_output; // yes, you are seeing right, index in tx and psbt_input must match
            if foreign {
                self.add_foreign_utxo(op, psbt_input.clone(), Weight::from_wu(3))?;
                // TODO: how to calculate the satisfaction weight?
            } else {
                self.add_utxo(op)?;
            }
            let utxo = psbt_input.clone().witness_utxo.expect("witness_utxo missing in psbt"); // Dump if not present! TODO error handling?
            total = total + utxo.value.to_sat() as i64;
        }

        // find the change TxOut from Bob and add them
        for txout in psbt.unsigned_tx.output.iter() {
            let scriptbuf = txout.script_pubkey.clone();
            if !disregard_scripts.contains(&scriptbuf) {
                self.add_recipient(scriptbuf, txout.value);
                total = total - txout.value.to_sat() as i64;
            }
        }
        Ok(total)
    }
}

struct MusigAdaptor {
    sec: Scalar,
    pubkey: PublicKey,
    address: Address,
}