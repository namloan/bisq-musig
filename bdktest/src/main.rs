mod nigiri;

use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::bip32::Xpriv;
use bdk_electrum::{electrum_client, BdkElectrumClient};
use bdk_wallet::bitcoin::psbt::Input;
use bdk_wallet::bitcoin::{Amount, FeeRate, Network, Psbt, TxOut, Txid, Weight};
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::template::{Bip86, DescriptorTemplate};
use bdk_wallet::{AddressInfo, KeychainKind, PersistedWallet, SignOptions, Wallet};
use rand::RngCore;
use std::collections::HashSet;
use std::io::Write;

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

struct DepositTx {
    psbt: Psbt,
}

impl DepositTx {
    fn new(alice: &mut TestWallet, bob: &mut TestWallet) -> anyhow::Result<DepositTx> {
        // alice makes pbst which will get singed by bob
        // let w = &mut alice.wallet;

        // let alice_out = alice.wallet.list_output().next().unwrap().outpoint;
        // TODO next line is a quirks, make the wallet pick the right utxo(s) instead of just using the first one.
        let output = bob.wallet.list_output().next().unwrap(); // first utxo may not have enough funds on it.
        let bob_out = output.outpoint;
        dbg!(bob_out);

        // from psbt only field witness_utxo is being used. (for segwit transactions)
        let tout: TxOut = output.txout;
        let mut input = Input::default();
        input.witness_utxo = Some(tout.clone());

        // TODO: how to calculate the satisfaction weight?
        let mut builder = alice.wallet.build_tx();
        builder
            .add_foreign_utxo(bob_out, input, Weight::from_wu(3))?
            .add_recipient(
                bob.next_unused_address().script_pubkey(),
                Amount::from_btc(1.5)?,
            ) // to address should be calculated from musig
            .fee_rate(FeeRate::from_sat_per_vb(20).unwrap()); // TODO calc real feerate
        let mut psbt = builder.finish()?;

        alice.wallet.insert_txout(bob_out, tout);
        let sign = SignOptions {
            trust_witness_utxo: true, // TODO why not send the non-witness_utxo as well??
            ..SignOptions::default()
        };

        // Alice signs her part
        let alice_signed = alice.wallet.sign(&mut psbt, sign.clone())?;
        assert!(!alice_signed);

        // Send the PSBT to Bob for signing
        let bob_signed = bob.wallet.sign(&mut psbt, sign)?;
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
