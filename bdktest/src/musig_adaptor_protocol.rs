use crate::{ProtocolRole, TestWallet};
use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::absolute::LockTime;
use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::{Address, Amount, FeeRate, Psbt, Sequence, Txid, Weight};
use bdk_wallet::bitcoin::{KnownHrp, PublicKey, ScriptBuf};
use bdk_wallet::coin_selection::BranchAndBoundCoinSelection;
use bdk_wallet::{SignOptions, TxBuilder};
use musig2::KeyAggContext;
use secp::{Point, Scalar};

use bdk_core::bitcoin::Transaction;
use bdk_wallet::bitcoin::key::Secp256k1;
use bdk_wallet::miniscript::ToPublicKey;

/**
This is not only testing code.
It also shows how the Java FSM is supposed to call this library

Of course the tests do not need to be replicated in Java. Nor the Nigiri code.
*/
#[test]
fn test_musig() -> anyhow::Result<()> {
    println!("running...");
    crate::nigiri::check_start();
    let mut alice_funds = crate::nigiri::funded_wallet();
    //TestWallet::new()?;

    let bob_funds = crate::nigiri::funded_wallet();
    //TestWallet::new()?;
    crate::nigiri::fund_wallet(&mut alice_funds);
    let seller_amount = &Amount::from_btc(1.4)?;
    let buyer_amount = &Amount::from_btc(0.2)?;
    let alice_context = BMPContext::new(alice_funds, ProtocolRole::Seller, seller_amount.clone(), buyer_amount.clone())?;

    let mut alice = BMPProtocol::new(alice_context)?;
    let bob_context = BMPContext::new(bob_funds, ProtocolRole::Buyer, seller_amount.clone(), buyer_amount.clone())?;
    let mut bob = BMPProtocol::new(bob_context)?;

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
    let alice_r3 = alice.round3(bob_r2)?;
    let bob_r3 = bob.round3(alice_r2)?;

    assert_eq!(alice_r3.deposit_txid, bob_r3.deposit_txid);
    dbg!(&alice_r3.deposit_txid);
    Ok(())
}
pub struct Round1Parameter {
    p_a: Point,
    q_a: Point,
    dep_part_psbt: Psbt,
}
pub(crate) struct Round2Parameter {
    p_agg: Point,
    q_agg: Point,
    deposit_tx_signed: Psbt,
}
pub(crate) struct Round3Parameter {
    deposit_txid: Txid,
}
/**
this context is for the whole process and need to be persisted by the caller
*/
pub struct BMPContext {
    // first of all, everything which is general to the protcol itself
    funds: TestWallet,
    role: ProtocolRole,
    seller_amount: Amount,
    buyer_amount: Amount,
}
pub struct BMPProtocol {
    ctx: BMPContext,
    p_tik: AggKey, // Point securing Seller deposit and trade amount
    q_tik: AggKey, // Point securing Buyer deposit
    deposit_tx: DepositTx,
    round: u8, // which round are we in.
}

impl BMPContext {
    pub(crate) fn new(funds: TestWallet, role: ProtocolRole, seller_amount: Amount, buyer_amount: Amount) -> anyhow::Result<BMPContext> {
        Ok(BMPContext {
            funds,
            role,
            seller_amount,
            buyer_amount,
        })
    }
}
impl BMPProtocol {
    pub(crate) fn new(ctx: BMPContext) -> anyhow::Result<BMPProtocol> {
        Ok(BMPProtocol {
            ctx,
            p_tik: AggKey::new()?,
            q_tik: AggKey::new()?,
            deposit_tx: DepositTx::new(),
            round: 0,
        })
    }

    pub(crate) fn round1(&mut self) -> anyhow::Result<Round1Parameter> {
        self.check_round(1);

        let dep_part_psbt = self.deposit_tx.generate_part_tx(&mut self.ctx, &self.p_tik.pub_point, &self.q_tik.pub_point)?;
        Ok(Round1Parameter {
            p_a: self.p_tik.pub_point,
            q_a: self.q_tik.pub_point,
            dep_part_psbt,
        })
    }

    pub(crate) fn round2(&mut self, bob: Round1Parameter) -> anyhow::Result<Round2Parameter> {
        self.check_round(2);
        assert_ne!(bob.p_a, bob.q_a, "Bob is sending the same point for P' and Q'.");

        // save bobs parameters
        self.p_tik.other_point = Some(bob.p_a);
        self.q_tik.other_point = Some(bob.q_a);
        self.p_tik.aggregate_key(bob.p_a)?;
        self.q_tik.aggregate_key(bob.q_a)?;
        // now we have the aggregated key
        // so we can contruct the Deposit Tx
        let depopit_tx_signed = self.deposit_tx.build_and_merge_tx(&mut self.ctx, &bob.dep_part_psbt, &self.p_tik, &self.q_tik)?;

        Ok(Round2Parameter {
            p_agg: self.p_tik.agg_point.unwrap(),
            q_agg: self.q_tik.agg_point.unwrap(),
            deposit_tx_signed: depopit_tx_signed,
        })
    }
    pub(crate) fn round3(&mut self, bob: Round2Parameter) -> anyhow::Result<Round3Parameter> {
        self.check_round(3);
        // actually this next test is not necessary, but double-checking and fast fail is always good
        // TODO since we are sending this only to validate, we could use a hash of it as well, optimization
        assert_eq!(bob.p_agg, self.p_tik.agg_point.unwrap(), "Bob is sending the wrong P' for his aggregated key.");
        assert_eq!(bob.q_agg, self.q_tik.agg_point.unwrap(), "Bob is sending the wrong Q' for his aggregated key.");

        let txid = self.deposit_tx.transfer_sig_and_broadcast(&mut self.ctx, bob.deposit_tx_signed)?;
        Ok(Round3Parameter { deposit_txid: txid })
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

struct DepositTx {
    part_psbt: Option<Psbt>,
    signed_psbt: Option<Psbt>,
    tx: Option<Transaction>,
}


impl DepositTx {
    fn new() -> DepositTx {
        DepositTx {
            part_psbt: None,
            signed_psbt: None,
            tx: None,
        }
    }

    pub fn generate_part_tx(&mut self, ctx: &mut BMPContext, p_a: &Point, q_a: &Point) -> anyhow::Result<Psbt> {
        // we are using our point as receipient address, but it will be changed to the musig address later.
        let (funded_by_me, amount) = match ctx.role {
            ProtocolRole::Seller => (p_a, ctx.seller_amount), // pub
            ProtocolRole::Buyer => (q_a, ctx.buyer_amount),
        };
        // create and fund a (virtual) transaction which funds Alice part of the Deposit Tx
        let mut builder = ctx.funds.wallet.build_tx();
        builder.add_recipient(
            funded_by_me.key_spend_no_merkle_address()?.script_pubkey(), amount,
        );
        builder.fee_rate(FeeRate::from_sat_per_vb(20).unwrap()); // TODO feerates shall come from pricenodes
        let pbst = builder.finish()?;
        self.part_psbt = Some(pbst.clone());
        // dbg!(&pbst.unsigned_tx.output);
        Ok(pbst)
    }

    pub fn build_and_merge_tx(&mut self, ctx: &mut BMPContext, other_psbt: &Psbt, p_tik: &AggKey, q_tik: &AggKey) -> anyhow::Result<Psbt> {
        let my_psbt = self.part_psbt.as_ref().unwrap();
        //sanity check that Bob doesn't send UTXOs owned by alice.
        for pbst_input in other_psbt.inputs.iter() {
            let scriptbuf = pbst_input.witness_utxo.clone().unwrap().script_pubkey;
            if ctx.funds.wallet.is_mine(scriptbuf.clone()) {
                // bob is trying to trick me.
                panic!("Fraud detected. Bob send me my own scriptbuf {:?}", scriptbuf)
            }
        }
        // TODO sanity check if bobs transaction actually calculate the fee correctly, otherwise he could save on fess at the expense of alice

        // recreate combined ty from scratch
        let mut builder = ctx.funds.wallet.build_tx();
        builder.manually_selected_only(); // only use inputs we have already identified.
        builder.set_exact_sequence(Sequence::MAX); // no RBF, RBF disabled for foreign utxos anyway.
        // keep track of the fees, as total fees is not equal to sum of fess from alice and bob psbts.
        let mut total: i64 = 0; // measured in sats
        // add deposit outputs first.
        let p_tik_adr = p_tik.get_agg_adr()?;
        builder.add_recipient(p_tik_adr.script_pubkey(), ctx.seller_amount);
        total = total - ctx.seller_amount.to_sat() as i64;

        let q_tik_adr = q_tik.get_agg_adr()?;
        builder.add_recipient(q_tik_adr.script_pubkey(), ctx.buyer_amount);
        total = total - ctx.buyer_amount.to_sat() as i64;

        // in the following merge, we want to copy the funding inputs and the change outputs
        // so we disregard basically all known scripts.
        // technically, only the script created in the 'generate_part_tx()' should appear
        let disregard_scripts = &[
            p_tik.pub_point.key_spend_no_merkle_address()?.script_pubkey(),
            q_tik.pub_point.key_spend_no_merkle_address()?.script_pubkey(),
            p_tik.other_point.unwrap().key_spend_no_merkle_address()?.script_pubkey(),
            q_tik.other_point.unwrap().key_spend_no_merkle_address()?.script_pubkey(),
            p_tik.agg_point.unwrap().key_spend_no_merkle_address()?.script_pubkey(), // technically these 2 script should not appear
            q_tik.agg_point.unwrap().key_spend_no_merkle_address()?.script_pubkey(), // but don't let the other side do some fancy stuff
        ];
        total = builder.merge(my_psbt, false, total, disregard_scripts)?;
        total = builder.merge(other_psbt, true, total, disregard_scripts)?;

        builder.fee_absolute(Amount::from_sat(total as u64));
        builder.nlocktime(LockTime::ZERO); // TODO RBF disabled anyway, so this value can be disregarded.

        // Attempt to finish and return the merged PSBT
        let mut merged_psbt = builder.finish()?;

        // We need to sort the order of inputs and output to make the TXid of alice nd bod equal
        // TODO come up with a randomized sort to preserve privacy
        // TODO use builder.ordering() with custom alg.
        sort(&mut merged_psbt);

        // sign my psbt
        ctx.funds.wallet.sign(&mut merged_psbt, SignOptions::default())?;
        self.signed_psbt = Some(merged_psbt.clone());
        self.part_psbt = None; // make sure not reused.
        Ok(merged_psbt)
    }

    fn transfer_sig_and_broadcast(&mut self, ctx: &mut BMPContext,
                                  psbt_bob: Psbt,   // bobs psbt should be same as mine but have bob's sig
    ) -> anyhow::Result<Txid> {
        // I expect to find all sigs missing in psbt_alice to be in psbt_bob
        // also I expect that both psbts are the same exect for the sigs.
        let mut my_psbt = self.signed_psbt.as_ref().unwrap().clone();

        dbg!(&my_psbt.unsigned_tx);
        dbg!(&psbt_bob.unsigned_tx);
        assert!(my_psbt.unsigned_tx == psbt_bob.unsigned_tx);

        for (i, alice_input) in my_psbt.inputs.iter_mut().enumerate() {
            if alice_input.final_script_witness.is_none() {
                alice_input.final_script_witness =
                    Some(psbt_bob.inputs[i].final_script_witness.clone().unwrap()); //must exist
            }
        }
        let tx = my_psbt.extract_tx()?;
        self.tx = Some(tx.clone());
        self.signed_psbt = None; // remove used data
        // TODO alice and bob will broadcast, is that a bug or a feature?
        ctx.funds.client.transaction_broadcast(&tx)?;
        Ok(tx.compute_txid())
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
        self.agg_point.unwrap().key_spend_no_merkle_address()
    }
}
trait PointExt {
    fn key_spend_no_merkle_address(&self) -> anyhow::Result<Address>;
}
impl PointExt for Point {
    fn key_spend_no_merkle_address(&self) -> anyhow::Result<Address> {
        let pubkey = PublicKey::from_slice(&self.serialize())?.to_x_only_pubkey();
        let secp = Secp256k1::new(); // TODO make it static?
        let adr = Address::p2tr(&secp, pubkey, None, KnownHrp::Regtest);
        Ok(adr)
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
