use crate::{ProtocolRole, TestWallet, AMOUNT_BUYER, AMOUNT_SELLER, P_A_STRING, Q_A_STRING};
use anyhow::anyhow;
use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::absolute::LockTime;
use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::{Address, Amount, FeeRate, Psbt, Sequence, Txid, Weight};
use bdk_wallet::coin_selection::BranchAndBoundCoinSelection;
use bdk_wallet::{SignOptions, TxBuilder};
use std::str::FromStr;

pub struct MusigProtocol {
    funds: TestWallet,
    role: ProtocolRole,
    // Alice pubkey for the seller multisig
    p_pub: Address,
    // Alice pubkey for the buyer multisig
    q_pub: Address,
}

impl MusigProtocol {
    pub(crate) fn new(funds: TestWallet, role: ProtocolRole) -> anyhow::Result<MusigProtocol> {
        let p_pub = Address::from_str(P_A_STRING)?.assume_checked(); // TODO replace by ephemeral key generating Point
        let q_pub = Address::from_str(Q_A_STRING)?.assume_checked();
        Ok(MusigProtocol { funds, role, p_pub, q_pub })
    }

    pub(crate) fn generate_part_tx(&mut self) -> anyhow::Result<Psbt> {
        let (funded_by_me, amount) = match self.role {
            ProtocolRole::Seller => (&self.p_pub, AMOUNT_SELLER),
            ProtocolRole::Buyer => (&self.q_pub, AMOUNT_BUYER),
        };
        // create and fund a (virtual) transaction which funds Alice part of the Deposit Tx
        let mut builder = self.funds.wallet.build_tx();
        builder.add_recipient(
            funded_by_me.script_pubkey(), Amount::from_btc(amount)?,
        );
        builder.fee_rate(FeeRate::from_sat_per_vb(20).unwrap()); // TODO calc real feerate
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
                panic!(
                    "Fraud detected. Bob send me my own scriptbuf {:?}",
                    scriptbuf
                )
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
        let seller_amount = Amount::from_btc(AMOUNT_SELLER)?;
        builder.add_recipient(self.p_pub.script_pubkey(), seller_amount);
        total = total - seller_amount.to_sat() as i64;

        let buyer_amount = Amount::from_btc(AMOUNT_BUYER)?;
        builder.add_recipient(self.q_pub.script_pubkey(), buyer_amount); // TODO make amounts variable
        total = total - buyer_amount.to_sat() as i64;

        total = builder.merge(my_psbt, false, total)?;
        total = builder.merge(other_psbt, true, total)?;

        builder.fee_absolute(Amount::from_sat(total as u64)); // TODO calc real feerate
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
    fn merge(&mut self, psbt: &Psbt, foreign: bool, total: i64) -> anyhow::Result<i64>;
}

impl Merge for TxBuilder<'_, BranchAndBoundCoinSelection> {
    fn merge(&mut self, psbt: &Psbt, foreign: bool, mut total: i64) -> anyhow::Result<i64> {
        let p_script = Address::from_str(P_A_STRING)?.assume_checked().script_pubkey();
        let q_script = Address::from_str(Q_A_STRING)?.assume_checked().script_pubkey();

        let disregard_scripts = vec![p_script, q_script];
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