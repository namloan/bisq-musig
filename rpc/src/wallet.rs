use bdk_bitcoind_rpc::Emitter;
use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, Client, RpcApi as _};
use bdk_wallet::{AddressInfo, Balance, KeychainKind, LocalOutput, Wallet};
use bdk_wallet::bitcoin::{Network, Transaction, Txid};
use bdk_wallet::chain::{CheckPoint, ChainPosition, ConfirmationBlockTime};
use drop_stream::DropStream;
use futures::stream::{BoxStream, StreamExt as _};
use std::sync::{Arc, Mutex, RwLock};

use crate::observable::ObservableHashMap;

const COOKIE_FILE_PATH: &str = ".localnet/bitcoind/regtest/.cookie";
//noinspection SpellCheckingInspection
const EXTERNAL_DESCRIPTOR: &str = "tr(tprv8ZgxMBicQKsPdrjwWCyXqqJ4YqcyG4DmKtjjsRt29v1PtD3r3PuFJAj\
    WytzcvSTKnZAGAkPSmnrdnuHWxCAwy3i1iPhrtKAfXRH7dVCNGp6/86'/1'/0'/0/*)#g9xn7wf9";
//noinspection SpellCheckingInspection
const INTERNAL_DESCRIPTOR: &str = "tr(tprv8ZgxMBicQKsPdrjwWCyXqqJ4YqcyG4DmKtjjsRt29v1PtD3r3PuFJAj\
    WytzcvSTKnZAGAkPSmnrdnuHWxCAwy3i1iPhrtKAfXRH7dVCNGp6/86'/1'/0'/1/*)#e3rjrmea";

pub trait WalletService {
    fn connect(&self);
    fn balance(&self) -> Balance;
    fn reveal_next_address(&self) -> AddressInfo;
    fn list_unspent(&self) -> Vec<LocalOutput>;
    fn get_tx_confidence_stream(&self, txid: Txid) -> BoxStream<'static, Option<TxConfidence>>;
}

pub struct WalletServiceImpl {
    // NOTE: To avoid deadlocks, must be careful to acquire these locks in consistent order. At
    //  present, the lock on 'wallet' is acquired first, then the lock on 'tx_confidence_map'.
    wallet: RwLock<Wallet>,
    tx_confidence_map: Mutex<ObservableHashMap<Txid, TxConfidence>>,
}

impl WalletServiceImpl {
    pub fn new() -> Self {
        let wallet = Wallet::create(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
            .network(Network::Regtest)
            .create_wallet_no_persist()
            .unwrap();

        let mut tx_confidence_map = ObservableHashMap::new();
        tx_confidence_map.sync(tx_confidence_entries(&wallet));

        Self { wallet: RwLock::new(wallet), tx_confidence_map: Mutex::new(tx_confidence_map) }
    }

    fn sync_tx_confidence_map(&self) {
        let wallet = self.wallet.read().unwrap();
        self.tx_confidence_map.lock().unwrap().sync(tx_confidence_entries(&wallet));
    }
}

fn tx_confidence_entries(wallet: &Wallet) -> impl Iterator<Item=(Txid, TxConfidence)> + '_ {
    let next_height = wallet.latest_checkpoint().height() + 1;
    wallet.transactions()
        .map(move |wallet_tx| {
            let wallet_tx: WalletTx = wallet_tx.into();
            let conf_height = wallet_tx.chain_position.confirmation_height_upper_bound().unwrap_or(next_height);
            let num_confirmations = next_height - conf_height;
            (wallet_tx.txid, TxConfidence { wallet_tx, num_confirmations })
        })
}

impl WalletService for WalletServiceImpl {
    // FIXME: This currently panics in case of failure to sync. Make error handling more robust.
    fn connect(&self) {
        let rpc_client: Client = Client::new(
            "https://127.0.0.1:18443",
            Auth::CookieFile(COOKIE_FILE_PATH.into()),
        ).unwrap();

        let blockchain_info = rpc_client.get_blockchain_info().unwrap();
        println!("Connected to Bitcoin Core RPC.\n  Chain: {}\n  Latest block: {} at height {}",
            blockchain_info.chain, blockchain_info.best_block_hash, blockchain_info.blocks);

        let wallet_tip: CheckPoint = self.wallet.read().unwrap().latest_checkpoint();
        let start_height = wallet_tip.height();
        println!("Current wallet tip is: {} at height {}", wallet_tip.hash(), start_height);

        let mut emitter = Emitter::new(&rpc_client, wallet_tip, start_height);
        while let Some(block) = emitter.next_block().unwrap() {
            print!(" {}", block.block_height());
            self.wallet.write().unwrap()
                .apply_block_connected_to(&block.block, block.block_height(), block.connected_to())
                .unwrap();
        }
        println!();

        println!("Syncing mempool...");
        let mempool_emissions = emitter.mempool().unwrap();
        self.wallet.write().unwrap().apply_unconfirmed_txs(mempool_emissions);

        println!("Syncing tx confidence map with wallet.");
        self.sync_tx_confidence_map();

        println!("Wallet balance after syncing: {}", self.balance().total());
    }

    fn balance(&self) -> Balance {
        self.wallet.read().unwrap().balance()
    }

    fn reveal_next_address(&self) -> AddressInfo {
        self.wallet.write().unwrap().reveal_next_address(KeychainKind::External)
    }

    fn list_unspent(&self) -> Vec<LocalOutput> {
        self.wallet.read().unwrap().list_unspent().collect()
    }

    fn get_tx_confidence_stream(&self, txid: Txid) -> BoxStream<'static, Option<TxConfidence>> {
        DropStream::new(self.tx_confidence_map.lock().unwrap().observe(txid), move || {
            println!("Confidence stream has been dropped for txid: {txid}");
        }).boxed()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TxConfidence {
    pub wallet_tx: WalletTx,
    pub num_confirmations: u32,
}

#[derive(Clone, Eq, PartialEq)]
pub struct WalletTx {
    pub txid: Txid,
    pub tx: Arc<Transaction>,
    pub chain_position: ChainPosition<ConfirmationBlockTime>,
}

impl From<bdk_wallet::WalletTx<'_>> for WalletTx {
    fn from(value: bdk_wallet::WalletTx) -> Self {
        Self { txid: value.tx_node.txid, tx: value.tx_node.tx, chain_position: value.chain_position }
    }
}
