mod nigiri;
mod musig_protocol;

use anyhow::anyhow;
use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::bip32::Xpriv;
use bdk_electrum::{electrum_client, BdkElectrumClient};
use bdk_wallet::bitcoin::absolute::LockTime;
use bdk_wallet::bitcoin::{Address, Amount, FeeRate, Network, Psbt, Sequence, Txid, Weight};
use bdk_wallet::coin_selection::BranchAndBoundCoinSelection;
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::template::{Bip86, DescriptorTemplate};
use bdk_wallet::{AddressInfo, KeychainKind, PersistedWallet, SignOptions, TxBuilder, Wallet};
use rand::RngCore;
use std::collections::HashSet;
use std::io::Write;
use std::str::FromStr;

const DESCRIPTOR_PRIVATE_EXTERNAL: &str = "tr(tprv8ZgxMBicQKsPejo7mjMzejAWDQYi1UtxzyxJfNbvtPqCsVFkZAEj7hnnrH938bXWMccgkj9BQmduhnmmjS41rAXE8atPLkLUadrXLUffpd8/86'/1'/0'/0/*)#w0y7v8y2";
const DESCRIPTOR_PRIVATE_INTERNAL: &str = "tr(tprv8ZgxMBicQKsPejo7mjMzejAWDQYi1UtxzyxJfNbvtPqCsVFkZAEj7hnnrH938bXWMccgkj9BQmduhnmmjS41rAXE8atPLkLUadrXLUffpd8/86'/1'/0'/1/*)";
// const DESCRIPTOR_PRIVATE_INTERNAL: &str = "tr([5dd79578/86'/1'/0']tpubDCkzmSCo2jKu2oTMdXjsbAHZN27RxtsgdyV1sKj1LoW4HBkMLd24zGQt1278xGPSggSqqHrfkUTdisyZ91cXkCzjwWQsmg5L5D3M8prVA7j/1/*)";
const STOP_GAP: usize = 50;
const BATCH_SIZE: usize = 5;

fn main() {}

const ELECTRUM_URL: &str =
// "ssl://electrum.blockstream.info:60002";
    "localhost:50000"; //TODO move to env
struct TestWallet {
    wallet: Wallet,
    client: BdkElectrumClient<electrum_client::Client>,
}

enum ProtocolRole {
    Seller,
    Buyer,
}
// TODO P_a and Q_a should be replaced by  public keys generated for MuSig2
const P_A_STRING: &'static str = "bcrt1pjvx4sh3w3n2qwwrn8fdswtpqwneelwm3nvqp8ys3wap6znvc5k9q9nen7q";
const Q_A_STRING: &'static str = "bcrt1pg0m5rem5f3jcqxzc9v93zykefvgg0z70shwgh4ap5tfv9rj0tglq039r50";

fn generate_part_tx(alice: &mut TestWallet, myrole: ProtocolRole) -> anyhow::Result<Psbt> {
    // Alice pubkey for the seller multisig
    let p_a = Address::from_str(P_A_STRING)?;
    // Alice pubkey for the buyer multisig
    let q_a = Address::from_str(Q_A_STRING)?;
    let (funded_by_alice, amount) = match myrole {
        ProtocolRole::Seller => (p_a, AMOUNT_SELLER),
        ProtocolRole::Buyer => (q_a, AMOUNT_BUYER),
    };
    // create and fund a (virtual) transaction which funds Alice part of the Deposit Tx
    let mut builder = alice.wallet.build_tx();
    builder.add_recipient(
        funded_by_alice.assume_checked().script_pubkey(),
        Amount::from_btc(amount)?,
    );
    builder.fee_rate(FeeRate::from_sat_per_vb(20).unwrap()); // TODO calc real feerate
    let pbst = builder.finish()?;
    // dbg!(&pbst.unsigned_tx.output);
    Ok(pbst)
}


const AMOUNT_SELLER: f64 = 0.2;
const AMOUNT_BUYER: f64 = 0.4;

fn build_and_merge_tx(
    alice: &mut TestWallet,
    alice_psbt: Psbt,
    bob_psbt: Psbt,
    _alice_role: ProtocolRole,
) -> anyhow::Result<Psbt> {
    //TODO this should come from keyAgg
    let p = Address::from_str(P_A_STRING)?; // should be calculated by keyAgg
    let q = Address::from_str(Q_A_STRING)?; // should be calculated by keyAgg

    //sanity check that Bob doesn't send UTXOs owned by alice.
    for pbst_input in bob_psbt.inputs.iter() {
        let scriptbuf = pbst_input.clone().witness_utxo.unwrap().script_pubkey;
        if alice.wallet.is_mine(scriptbuf.clone()) {
            // bob is trying to trick me.
            panic!(
                "Fraud detected. Bob send me my own scriptbuf {:?}",
                scriptbuf
            )
        }
    }
    // TODO sanity check if bobs transaction actually calculate the fee correctly, otherwise he could save on fess at the expense of alice

    // recreate combined ty from scratch
    let mut builder = alice.wallet.build_tx();
    builder.manually_selected_only(); // only use inputs we have already identified.
    builder.set_exact_sequence(Sequence::MAX); // no RBF, RBF disabled for foreign utxos anyway.
    // add deposit outputs first.
    builder.add_recipient(
        p.assume_checked().script_pubkey(),
        Amount::from_btc(AMOUNT_SELLER)?,
    );
    builder.add_recipient(
        q.assume_checked().script_pubkey(),
        Amount::from_btc(AMOUNT_BUYER)?,
    ); // TODO change amounts
    builder.merge(alice_psbt, false)?;
    builder.merge(bob_psbt, true)?;

    builder.fee_absolute(Amount::from_sat(6170)); // TODO calc real feerate
    builder.nlocktime(LockTime::ZERO); // TODO RBF disabled anyway, so this value can be disregarded.

    // Attempt to finish and return the merged PSBT
    let mut merged_psbt = builder
        .finish()
        .map_err(|e| anyhow!("Failed to build transaction: {:?}", e))?;

    // We need to sort the order of inputs and output to make the TXid of alice nd bod equal
    // TODO come up with a randomized sort to preserve privacy
    sort(&mut merged_psbt);

    // sign my psbt
    alice
        .wallet
        .sign(&mut merged_psbt, SignOptions::default())?;
    Ok(merged_psbt)
}

fn transfer_sig_and_broadcast(
    alice: &TestWallet,
    mut psbt_alice: Psbt, // my generate psbt
    psbt_bob: Psbt,   // bobs psbt should be same as mine but have bob's sig
    _alice_role: ProtocolRole,
) -> anyhow::Result<Txid> {
    // I expect to find all sigs missing in psbt_alice to be in psbt_bob
    // also I expect that both psbts are the same exect for the sigs.
    dbg!(&psbt_alice.unsigned_tx);
    dbg!(&psbt_bob.unsigned_tx);
    assert!(psbt_alice.unsigned_tx == psbt_bob.unsigned_tx);

    for (i, alice_input) in psbt_alice.inputs.iter_mut().enumerate() {
        if alice_input.final_script_witness.is_none() {
            alice_input.final_script_witness =
                Some(psbt_bob.inputs[i].final_script_witness.clone().unwrap()); //must exist
        }
    }
    let tx = psbt_alice.extract_tx()?;
    // TODO alice and bob will broadcast, is that a bug or a feature?
    alice.client.transaction_broadcast(&tx)?;
    Ok(tx.compute_txid())
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
    fn merge(&mut self, psbt: Psbt, foreign: bool) -> anyhow::Result<&mut Self>;
}

impl Merge for TxBuilder<'_, BranchAndBoundCoinSelection> {
    fn merge(&mut self, psbt: Psbt, foreign: bool) -> anyhow::Result<&mut Self> {
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
        }

        // find the change TxOut from Bob and add them
        for txout in psbt.unsigned_tx.output.iter() {
            let scriptbuf = txout.script_pubkey.clone();
            if !disregard_scripts.contains(&scriptbuf) {
                self.add_recipient(scriptbuf, txout.value);
            }
        }
        Ok(self)
    }
}
struct DepositTx {
    psbt: Psbt,
}

impl DepositTx {
    fn new(mut alice: TestWallet, mut bob: TestWallet) -> anyhow::Result<DepositTx> {
        // alice makes pbst which will get singed by bob

        // generate partial transaction ---------------------------
        let deposit_address =
            Address::from_str("bcrt1pjvx4sh3w3n2qwwrn8fdswtpqwneelwm3nvqp8ys3wap6znvc5k9q9nen7q")?;
        let deposit_script = deposit_address.assume_checked().script_pubkey();

        let mut txbob = bob.wallet.build_tx();
        txbob.add_recipient(deposit_script.clone(), Amount::from_btc(1.5)?);
        let psbtbob = txbob.finish()?;
        dbg!(&psbtbob);
        // ---------------------------------

        // add all inputs from Bob ------------------------
        let tx = psbtbob.clone().extract_tx()?;
        dbg!(&tx);
        // check bobs inputs, if they are not spoofed
        for pbst_input in psbtbob.inputs.iter() {
            let scriptbuf = pbst_input.clone().witness_utxo.unwrap().script_pubkey;
            if alice.wallet.is_mine(scriptbuf.clone()) {
                // bob is trying to trick me.
                panic!(
                    "Fraud detected. Bob send me my own scriptbuf {:?}",
                    scriptbuf
                )
            }
        }

        let mut builder = alice.wallet.build_tx();
        for (index, psbt_input) in psbtbob.inputs.iter().enumerate() {
            let op = tx.input[index].previous_output; // yes, you are seeing right, index in tx and psbt_input must match
            builder.add_foreign_utxo(op, psbt_input.clone(), Weight::from_wu(3))?;
            // TODO: how to calculate the satisfaction weight?
            // alicewallet.insert_txout(op, prev_utxo); // do we need this??
        }

        // find the change TxOut from Bob and add them
        for txout in tx.output.iter() {
            let scriptbuf = txout.script_pubkey.clone();
            if scriptbuf != deposit_script {
                builder.add_recipient(scriptbuf, txout.value);
            }
        }

        builder
            .add_recipient(deposit_script, Amount::from_btc(2.2)?) // to address should be calculated from musig
            .fee_rate(FeeRate::from_sat_per_vb(20).unwrap()); // TODO calc real feerate
        let mut psbt = builder.finish()?;

        // Alice signs her part
        let alice_signed = alice.wallet.sign(&mut psbt, SignOptions::default())?;
        assert!(!alice_signed);

        // Send the PSBT to Bob for signing
        let bob_signed = bob.wallet.sign(&mut psbt, SignOptions::default())?;
        assert!(bob_signed);

        // At this point, the PSBT should be fully signed; finalize the transaction
        let tx = psbt.clone().extract_tx()?;

        // broadcast it using either wallet
        alice.client.transaction_broadcast(&tx)?;
        println!("txid = {}", tx.compute_txid());
        Ok(DepositTx { psbt })
    }
}

impl TestWallet {
    fn new() -> anyhow::Result<TestWallet> {
        let mut seed: [u8; 32] = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);

        let network: Network = Network::Regtest;
        let xprv: Xpriv = Xpriv::new_master(network, &seed)?;
        println!("Generated Master Private Key:\n{}\nWarning: be very careful with private keys when using MainNet! We are logging these values for convenience only because this is an example on RegTest.\n", xprv);

        let (descriptor, external_map, _) = Bip86(xprv, KeychainKind::External)
            .build(network)
            .expect("Failed to build external descriptor");

        let (change_descriptor, internal_map, _) = Bip86(xprv, KeychainKind::Internal)
            .build(network)
            .expect("Failed to build internal descriptor");

        let wallet = Wallet::create(descriptor, change_descriptor)
            .network(network)
            .keymap(KeychainKind::External, external_map)
            .keymap(KeychainKind::Internal, internal_map)
            .create_wallet_no_persist()?;
        let client = BdkElectrumClient::new(electrum_client::Client::new(ELECTRUM_URL)?);

        Ok(TestWallet { wallet, client })
    }

    fn sync(&mut self) -> anyhow::Result<()> {
        // Populate the electrum client's transaction cache so it doesn't redownload transaction we
        // already have.
        self.client
            .populate_tx_cache(self.wallet.tx_graph().full_txs().map(|tx_node| tx_node.tx));

        let request = self.wallet.start_full_scan().inspect({
            let mut stdout = std::io::stdout();
            // let mut once = HashSet::<KeychainKind>::new();
            move |_k, _spk_i, _| {
                // if once.insert(k) {
                //     print!("\nScanning keychain [{:?}]", k);
                // }
                // print!(" {:<3}", spk_i);
                stdout.flush().expect("must flush");
            }
        });
        eprintln!("requesting update...");
        let update = self
            .client
            .full_scan(request, STOP_GAP, BATCH_SIZE, false)?;
        self.wallet.apply_update(update)?;
        Ok(())
    }

    fn balance(&self) -> Amount {
        self.wallet.balance().trusted_spendable()
    }

    fn next_unused_address(&mut self) -> AddressInfo {
        self.wallet.next_unused_address(KeychainKind::External)
    }

    fn transfer_to_address(
        &mut self,
        address: AddressInfo,
        amount: Amount,
    ) -> anyhow::Result<Txid> {
        let mut tx_builder = self.wallet.build_tx();
        tx_builder.add_recipient(address.script_pubkey(), amount);

        let mut psbt = tx_builder.finish()?;
        let finalized = self.wallet.sign(&mut psbt, SignOptions::default())?;
        assert!(finalized);

        let tx = psbt.extract_tx()?;
        self.client.transaction_broadcast(&tx)?;
        Ok(tx.compute_txid())
    }
}

struct ConnectedWallet {
    wallet: PersistedWallet<Connection>,
    db: Connection,
}

impl ConnectedWallet {
    fn load_or_create_wallet(database_path: &str) -> anyhow::Result<ConnectedWallet> {
        // set to regtest
        let network = Network::Regtest;
        //create or load wallet
        let mut db = Connection::open(database_path)?;

        let wallet_opt = Wallet::load()
            .descriptor(KeychainKind::External, Some(DESCRIPTOR_PRIVATE_EXTERNAL))
            .descriptor(KeychainKind::Internal, Some(DESCRIPTOR_PRIVATE_INTERNAL))
            // .extract_keys()
            // .keymap()
            .check_network(network)
            .load_wallet(&mut db)?;
        let mut wallet = match wallet_opt {
            Some(wallet) => wallet,
            None => Wallet::create(DESCRIPTOR_PRIVATE_EXTERNAL, DESCRIPTOR_PRIVATE_INTERNAL)
                .network(network)
                .create_wallet(&mut db)?,
        };

        //sync
        // use electrum as backend
        let client = BdkElectrumClient::new(electrum_client::Client::new(ELECTRUM_URL)?);

        // Populate the electrum client's transaction cache so it doesn't redownload transaction we
        // already have.
        client.populate_tx_cache(wallet.tx_graph().full_txs().map(|tx_node| tx_node.tx));

        let request = wallet.start_full_scan().inspect({
            let mut stdout = std::io::stdout();
            let mut once = HashSet::<KeychainKind>::new();
            move |k, spk_i, _| {
                if once.insert(k) {
                    print!("\nScanning keychain [{:?}]", k);
                }
                print!(" {:<3}", spk_i);
                stdout.flush().expect("must flush");
            }
        });
        eprintln!("requesting update...");
        let update = client.full_scan(request, STOP_GAP, BATCH_SIZE, false)?;

        println!();

        wallet.apply_update(update)?;
        wallet.persist(&mut db)?;

        Ok(ConnectedWallet { wallet, db })
    }

    fn balance(&self) -> Amount {
        self.wallet.balance().trusted_spendable()
    }

    fn next_unused_address(&mut self) -> AddressInfo {
        self.wallet.next_unused_address(KeychainKind::External)
    }

    fn transfer_to_address(
        &mut self,
        address: AddressInfo,
        amount: Amount,
    ) -> anyhow::Result<Txid> {
        let client = BdkElectrumClient::new(electrum_client::Client::new(ELECTRUM_URL)?);
        let mut tx_builder = self.wallet.build_tx();
        tx_builder.add_recipient(address.script_pubkey(), amount);

        let mut psbt = tx_builder.finish()?;
        let finalized = self.wallet.sign(&mut psbt, SignOptions::default())?;
        assert!(finalized);

        let tx = psbt.extract_tx()?;
        client.transaction_broadcast(&tx)?;
        self.wallet.persist(&mut self.db)?;
        Ok(tx.compute_txid())
    }
}
