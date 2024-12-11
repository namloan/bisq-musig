use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::{Amount, Network};
use bdk_bitcoind_rpc::bitcoincore_rpc::RpcApi;
use bdk_electrum::{electrum_client, BdkElectrumClient};
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{KeychainKind, Wallet};
use std::collections::HashSet;
use std::io::Write;
use std::process::Command;
use std::time::Instant;
use std::{thread, time};
/*
ENDPOINTS
chopsticks localhost:3000
bitcoin localhost:18443
bitcoin localhost:18444
bitcoin localhost:28332
bitcoin localhost:28333
lnd localhost:9735
lnd localhost:10009
lnd localhost:18080
tap localhost:10029
tap localhost:8089
cln localhost:9935
cln localhost:9835
esplora localhost:5000
electrs localhost:50000
electrs localhost:30000

 */
const DESCRIPTOR_PRIVATE_EXTERNAL: &str = "tr(tprv8ZgxMBicQKsPejo7mjMzejAWDQYi1UtxzyxJfNbvtPqCsVFkZAEj7hnnrH938bXWMccgkj9BQmduhnmmjS41rAXE8atPLkLUadrXLUffpd8/86'/1'/0'/0/*)#w0y7v8y2";
const DESCRIPTOR_PRIVATE_INTERNAL: &str = "tr(tprv8ZgxMBicQKsPejo7mjMzejAWDQYi1UtxzyxJfNbvtPqCsVFkZAEj7hnnrH938bXWMccgkj9BQmduhnmmjS41rAXE8atPLkLUadrXLUffpd8/86'/1'/0'/1/*)";
// const DESCRIPTOR_PRIVATE_INTERNAL: &str = "tr([5dd79578/86'/1'/0']tpubDCkzmSCo2jKu2oTMdXjsbAHZN27RxtsgdyV1sKj1LoW4HBkMLd24zGQt1278xGPSggSqqHrfkUTdisyZ91cXkCzjwWQsmg5L5D3M8prVA7j/1/*)";
const STOP_GAP: usize = 50;
const BATCH_SIZE: usize = 5;
const SEND_AMOUNT: Amount = Amount::from_sat(5000);
const FUND_ADDRESS: &str = "bcrt1plrmcqc9pwf4zjcej5n7ynre5k8lkn0xcz0c7y3dw37e8nqew2utq5l06jv";
#[test]
fn test_tx() -> anyhow::Result<()> {
    check_start();
    make_tx()
}

const ELECTRUM_URL: &str =
    // "ssl://electrum.blockstream.info:60002";
    "localhost:50000"; //TODO move to env

fn make_tx() -> anyhow::Result<()> {
    eprintln!("Starting...");

    // set to regtest
    let network = Network::Regtest;
    //create or load wallet
    let start_load_wallet = Instant::now();
    let mut db = Connection::open("bdk-electrum-example.db")?;

    let wallet_opt = Wallet::load()
        .descriptor(KeychainKind::External, Some(DESCRIPTOR_PRIVATE_EXTERNAL))
        .descriptor(KeychainKind::Internal, Some(DESCRIPTOR_PRIVATE_INTERNAL))
        .extract_keys()
        .check_network(network)
        .load_wallet(&mut db)?;
    let mut wallet = match wallet_opt {
        Some(wallet) => wallet,
        None => Wallet::create(DESCRIPTOR_PRIVATE_EXTERNAL, DESCRIPTOR_PRIVATE_INTERNAL)
            .network(network)
            .create_wallet(&mut db)?,
    };
    eprintln!(
        "Loaded wallet in {}s",
        start_load_wallet.elapsed().as_secs_f32()
    );

    // check balance

    let balance = wallet.balance();
    eprintln!("Wallet balance before syncing: {}", balance.total());
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

    let balance = wallet.balance();
    println!("Wallet balance after syncing: {}", balance.total());

    // if below 1 btc fund wallet

    if balance.total() < SEND_AMOUNT {
        println!(
            "Please send at least {} to the address {}",
            SEND_AMOUNT,
            wallet
                .next_unused_address(KeychainKind::External)
                .to_string()
        );
    } else {
        // start protocol
    }

    Ok(())
}

#[test]
fn test_fund() {
    check_start();
    //tb1pez0yfvf9gqpnn4nwpjh6n7fza96nzgf88pgdgmkms0stzcrj7wqsxdkv32
    //
    // let wallet = update_balance();
    // let adress = wallet.unwrap().next_unused_address(KeychainKind::External);
    // // fund_address("bcrt1qfm40e76x6ywg6e2y9xratcfzppcnywvqn444v5");
    // fund_address(adress.to_string().as_str());
    fund_address(FUND_ADDRESS);
}
fn fund_address(address: &str) {
    // let faucet_response = Command::new("nigiri")
    //     .args(["faucet", address])
    //     .output()
    //     .expect("Failed to fund Alice's wallet");
    // eprintln!("{}", String::from_utf8_lossy(&faucet_response.stdout));
    // thread::sleep(time::Duration::from_secs(2)); // Add delays between steps

    eprintln!("Mining mining to {}", address);
    let resp = Command::new("nigiri")
        .args(["rpc", "generatetoaddress", "101", address])
        .output()
        .expect("Failed to mine block");
    eprintln!("reponse {}", String::from_utf8_lossy(&resp.stdout));
}
#[test]
fn check_start() {
    // Step 2: Run 'nigiri start' to start Nigiri
    let nigiri_output = Command::new("nigiri").arg("start").output().unwrap();

    if nigiri_output.status.success() {
        eprintln!("Nigiri starting...");
        thread::sleep(time::Duration::from_secs(3));
        eprintln!("Nigiri started successfully.");
    } else {
        let msg = String::from_utf8_lossy(&nigiri_output.stderr);
        if !msg.contains("already running") {
            panic!(
                "Failed to start Nigiri. Please install Docker and Nigiri manually. Error: {}",
                String::from_utf8_lossy(&nigiri_output.stderr)
            );
        }
    }
}
