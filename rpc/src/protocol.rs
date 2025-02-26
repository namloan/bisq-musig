use musig2::{AggNonce, KeyAggContext, NonceSeed, PartialSignature, PubNonce, SecNonce,
    SecNonceBuilder};
use musig2::adaptor::AdaptorSignature;
use secp::{MaybePoint, Point, Scalar};
use std::collections::BTreeMap;
use std::prelude::rust_2021::*;
use std::sync::{Arc, LazyLock, Mutex};
use thiserror::Error;

use crate::storage::{ByRef, ByVal, ByOptVal, Storage, ValStorage};

pub trait TradeModelStore {
    fn add_trade_model(&self, trade_model: TradeModel);
    fn get_trade_model(&self, trade_id: &str) -> Option<Arc<Mutex<TradeModel>>>;
}

type TradeModelMemoryStore = Mutex<BTreeMap<String, Arc<Mutex<TradeModel>>>>;

impl TradeModelStore for TradeModelMemoryStore {
    fn add_trade_model(&self, trade_model: TradeModel) {
        // TODO: Maybe use try_insert (or similar), to disallow overwriting a trade model with the same ID.
        self.lock().unwrap().insert(trade_model.trade_id.clone(), Arc::new(Mutex::new(trade_model)));
    }

    fn get_trade_model(&self, trade_id: &str) -> Option<Arc<Mutex<TradeModel>>> {
        self.lock().unwrap().get(trade_id).map(Arc::clone)
    }
}

pub static TRADE_MODELS: LazyLock<TradeModelMemoryStore> = LazyLock::new(|| Mutex::new(BTreeMap::new()));

#[derive(Default)]
pub struct TradeModel {
    trade_id: String,
    my_role: Role,
    pub trade_amount: Option<u64>,
    pub buyers_security_deposit: Option<u64>,
    pub sellers_security_deposit: Option<u64>,
    pub deposit_tx_fee_rate: Option<f64>,
    pub prepared_tx_fee_rate: Option<f64>,
    buyer_output_key_ctx: KeyCtx,
    seller_output_key_ctx: KeyCtx,
    swap_tx_input_sig_ctx: SigCtx,
    buyers_warning_tx_buyer_input_sig_ctx: SigCtx,
    buyers_warning_tx_seller_input_sig_ctx: SigCtx,
    sellers_warning_tx_buyer_input_sig_ctx: SigCtx,
    sellers_warning_tx_seller_input_sig_ctx: SigCtx,
    buyers_redirect_tx_input_sig_ctx: SigCtx,
    sellers_redirect_tx_input_sig_ctx: SigCtx,
}

#[derive(Default, Eq, PartialEq)]
pub enum Role {
    #[default] SellerAsMaker,
    SellerAsTaker,
    BuyerAsMaker,
    BuyerAsTaker,
}

#[expect(clippy::struct_field_names,
reason = "not sure removing common postfix would make things clearer")] // TODO: Consider further.
pub struct ExchangedNonces<'a, S: Storage> {
    pub swap_tx_input_nonce_share: S::Store<'a, PubNonce>,
    pub buyers_warning_tx_buyer_input_nonce_share: S::Store<'a, PubNonce>,
    pub buyers_warning_tx_seller_input_nonce_share: S::Store<'a, PubNonce>,
    pub sellers_warning_tx_buyer_input_nonce_share: S::Store<'a, PubNonce>,
    pub sellers_warning_tx_seller_input_nonce_share: S::Store<'a, PubNonce>,
    pub buyers_redirect_tx_input_nonce_share: S::Store<'a, PubNonce>,
    pub sellers_redirect_tx_input_nonce_share: S::Store<'a, PubNonce>,
}

#[expect(clippy::struct_field_names,
reason = "not sure removing common postfix would make things clearer")] // TODO: Consider further.
pub struct ExchangedSigs<'a, S: Storage> {
    pub peers_warning_tx_buyer_input_partial_signature: S::Store<'a, PartialSignature>,
    pub peers_warning_tx_seller_input_partial_signature: S::Store<'a, PartialSignature>,
    pub peers_redirect_tx_input_partial_signature: S::Store<'a, PartialSignature>,
    pub swap_tx_input_partial_signature: Option<S::Store<'a, PartialSignature>>,
}

pub struct KeyPair<PrvKey: ValStorage = ByVal> {
    pub pub_key: Point,
    pub prv_key: PrvKey::Store<Scalar>,
}

pub struct NoncePair {
    pub pub_nonce: PubNonce,
    pub sec_nonce: Option<SecNonce>,
}

#[derive(Default)]
struct KeyCtx {
    am_buyer: bool,
    my_key_share: Option<KeyPair>,
    peers_key_share: Option<KeyPair<ByOptVal>>,
    aggregated_key: Option<KeyPair<ByOptVal>>,
    key_agg_ctx: Option<KeyAggContext>,
}

// TODO: For safety, this should hold a reference to the KeyCtx our nonce & signature share (& final
//  aggregation) are built from, so that we don't have to pass it repeatedly as a method parameter.
#[derive(Default)]
struct SigCtx {
    am_buyer: bool,
    adaptor_point: MaybePoint,
    my_nonce_share: Option<NoncePair>,
    peers_nonce_share: Option<PubNonce>,
    aggregated_nonce: Option<AggNonce>,
    message: Option<Vec<u8>>,
    my_partial_sig: Option<PartialSignature>,
    peers_partial_sig: Option<PartialSignature>,
    aggregated_sig: Option<AdaptorSignature>,
}

impl TradeModel {
    pub fn new(trade_id: String, my_role: Role) -> Self {
        let mut trade_model = Self { trade_id, my_role, ..Default::default() };
        let am_buyer = trade_model.am_buyer();
        trade_model.buyer_output_key_ctx.am_buyer = am_buyer;
        trade_model.seller_output_key_ctx.am_buyer = am_buyer;
        trade_model.swap_tx_input_sig_ctx.am_buyer = am_buyer;
        trade_model.buyers_warning_tx_buyer_input_sig_ctx.am_buyer = am_buyer;
        trade_model.buyers_warning_tx_seller_input_sig_ctx.am_buyer = am_buyer;
        trade_model.sellers_warning_tx_buyer_input_sig_ctx.am_buyer = am_buyer;
        trade_model.sellers_warning_tx_seller_input_sig_ctx.am_buyer = am_buyer;
        trade_model.buyers_redirect_tx_input_sig_ctx.am_buyer = am_buyer;
        trade_model.sellers_redirect_tx_input_sig_ctx.am_buyer = am_buyer;
        trade_model
    }

    const fn am_buyer(&self) -> bool {
        matches!(self.my_role, Role::BuyerAsMaker | Role::BuyerAsTaker)
    }

    pub fn init_my_key_shares(&mut self) {
        let buyer_output_pub_key = self.buyer_output_key_ctx.init_my_key_share().pub_key;
        self.seller_output_key_ctx.init_my_key_share();
        if !self.am_buyer() {
            self.swap_tx_input_sig_ctx.adaptor_point = MaybePoint::Valid(buyer_output_pub_key);
        }
    }

    pub fn get_my_key_shares(&self) -> Option<[&KeyPair; 2]> {
        Some([
            self.buyer_output_key_ctx.my_key_share.as_ref()?,
            self.seller_output_key_ctx.my_key_share.as_ref()?
        ])
    }

    pub fn set_peer_key_shares(&mut self, buyer_output_pub_key: Point, seller_output_pub_key: Point) {
        self.buyer_output_key_ctx.peers_key_share = Some(KeyPair::from_public(buyer_output_pub_key));
        self.seller_output_key_ctx.peers_key_share = Some(KeyPair::from_public(seller_output_pub_key));
        if self.am_buyer() {
            // TODO: Should check that signing hasn't already begun before setting an adaptor point.
            self.swap_tx_input_sig_ctx.adaptor_point = MaybePoint::Valid(buyer_output_pub_key);
        }
    }

    pub fn aggregate_key_shares(&mut self) -> Result<()> {
        self.buyer_output_key_ctx.aggregate_key_shares()?;
        self.seller_output_key_ctx.aggregate_key_shares()?;
        Ok(())
    }

    pub fn init_my_nonce_shares(&mut self) -> Result<()> {
        for ctx in [
            &mut self.buyers_warning_tx_buyer_input_sig_ctx,
            &mut self.sellers_warning_tx_buyer_input_sig_ctx,
            &mut self.buyers_redirect_tx_input_sig_ctx
        ] {
            ctx.init_my_nonce_share(&self.buyer_output_key_ctx)?;
        }
        for ctx in [
            &mut self.swap_tx_input_sig_ctx,
            &mut self.buyers_warning_tx_seller_input_sig_ctx,
            &mut self.sellers_warning_tx_seller_input_sig_ctx,
            &mut self.sellers_redirect_tx_input_sig_ctx
        ] {
            ctx.init_my_nonce_share(&self.seller_output_key_ctx)?;
        }
        Ok(())
    }

    pub fn get_my_nonce_shares(&self) -> Option<ExchangedNonces<ByRef>> {
        Some(ExchangedNonces {
            swap_tx_input_nonce_share:
            &(self.swap_tx_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            buyers_warning_tx_buyer_input_nonce_share:
            &(self.buyers_warning_tx_buyer_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            buyers_warning_tx_seller_input_nonce_share:
            &(self.buyers_warning_tx_seller_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            sellers_warning_tx_buyer_input_nonce_share:
            &(self.sellers_warning_tx_buyer_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            sellers_warning_tx_seller_input_nonce_share:
            &(self.sellers_warning_tx_seller_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            buyers_redirect_tx_input_nonce_share:
            &(self.buyers_redirect_tx_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            sellers_redirect_tx_input_nonce_share:
            &(self.sellers_redirect_tx_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
        })
    }

    pub fn set_peer_nonce_shares(&mut self, peer_nonce_shares: ExchangedNonces<ByVal>) {
        self.swap_tx_input_sig_ctx.peers_nonce_share =
            Some(peer_nonce_shares.swap_tx_input_nonce_share);
        self.buyers_warning_tx_buyer_input_sig_ctx.peers_nonce_share =
            Some(peer_nonce_shares.buyers_warning_tx_buyer_input_nonce_share);
        self.buyers_warning_tx_seller_input_sig_ctx.peers_nonce_share =
            Some(peer_nonce_shares.buyers_warning_tx_seller_input_nonce_share);
        self.sellers_warning_tx_buyer_input_sig_ctx.peers_nonce_share =
            Some(peer_nonce_shares.sellers_warning_tx_buyer_input_nonce_share);
        self.sellers_warning_tx_seller_input_sig_ctx.peers_nonce_share =
            Some(peer_nonce_shares.sellers_warning_tx_seller_input_nonce_share);
        self.buyers_redirect_tx_input_sig_ctx.peers_nonce_share =
            Some(peer_nonce_shares.buyers_redirect_tx_input_nonce_share);
        self.sellers_redirect_tx_input_sig_ctx.peers_nonce_share =
            Some(peer_nonce_shares.sellers_redirect_tx_input_nonce_share);
    }

    pub fn aggregate_nonce_shares(&mut self) -> Result<()> {
        self.swap_tx_input_sig_ctx.aggregate_nonce_shares()?;
        self.buyers_warning_tx_buyer_input_sig_ctx.aggregate_nonce_shares()?;
        self.buyers_warning_tx_seller_input_sig_ctx.aggregate_nonce_shares()?;
        self.sellers_warning_tx_buyer_input_sig_ctx.aggregate_nonce_shares()?;
        self.sellers_warning_tx_seller_input_sig_ctx.aggregate_nonce_shares()?;
        self.buyers_redirect_tx_input_sig_ctx.aggregate_nonce_shares()?;
        self.sellers_redirect_tx_input_sig_ctx.aggregate_nonce_shares()?;
        Ok(())
    }

    pub fn sign_partial(&mut self) -> Result<()> {
        // TODO: Make these dummy messages (txs-to-sign) non-fixed, for greater realism:
        let [buyer_key_ctx, seller_key_ctx] = [&self.buyer_output_key_ctx, &self.seller_output_key_ctx];

        self.buyers_warning_tx_buyer_input_sig_ctx
            .sign_partial(buyer_key_ctx, b"buyer's warning tx buyer input".into())?;
        self.sellers_warning_tx_buyer_input_sig_ctx
            .sign_partial(buyer_key_ctx, b"seller's warning tx buyer input".into())?;
        self.buyers_redirect_tx_input_sig_ctx
            .sign_partial(buyer_key_ctx, b"buyer's redirect tx input".into())?;

        self.swap_tx_input_sig_ctx
            .sign_partial(seller_key_ctx, b"swap tx input".into())?;
        self.buyers_warning_tx_seller_input_sig_ctx
            .sign_partial(seller_key_ctx, b"buyer's warning tx seller input".into())?;
        self.sellers_warning_tx_seller_input_sig_ctx
            .sign_partial(seller_key_ctx, b"seller's warning tx seller input".into())?;
        self.sellers_redirect_tx_input_sig_ctx
            .sign_partial(seller_key_ctx, b"seller's redirect tx input".into())?;
        Ok(())
    }

    pub fn get_my_partial_signatures_on_peer_txs(&self) -> Option<ExchangedSigs<ByRef>> {
        Some(if self.am_buyer() {
            ExchangedSigs {
                peers_warning_tx_buyer_input_partial_signature: self.sellers_warning_tx_buyer_input_sig_ctx.my_partial_sig.as_ref()?,
                peers_warning_tx_seller_input_partial_signature: self.sellers_warning_tx_seller_input_sig_ctx.my_partial_sig.as_ref()?,
                peers_redirect_tx_input_partial_signature: self.sellers_redirect_tx_input_sig_ctx.my_partial_sig.as_ref()?,
                swap_tx_input_partial_signature: Some(self.swap_tx_input_sig_ctx.my_partial_sig.as_ref()?),
            }
        } else {
            ExchangedSigs {
                peers_warning_tx_buyer_input_partial_signature: self.buyers_warning_tx_buyer_input_sig_ctx.my_partial_sig.as_ref()?,
                peers_warning_tx_seller_input_partial_signature: self.buyers_warning_tx_seller_input_sig_ctx.my_partial_sig.as_ref()?,
                peers_redirect_tx_input_partial_signature: self.buyers_redirect_tx_input_sig_ctx.my_partial_sig.as_ref()?,
                swap_tx_input_partial_signature: Some(self.swap_tx_input_sig_ctx.my_partial_sig.as_ref()?),
            }
        })
    }

    pub fn set_peer_partial_signatures_on_my_txs(&mut self, sigs: &ExchangedSigs<ByVal>) {
        if self.am_buyer() {
            self.buyers_warning_tx_buyer_input_sig_ctx.peers_partial_sig = Some(sigs.peers_warning_tx_buyer_input_partial_signature);
            self.buyers_warning_tx_seller_input_sig_ctx.peers_partial_sig = Some(sigs.peers_warning_tx_seller_input_partial_signature);
            self.buyers_redirect_tx_input_sig_ctx.peers_partial_sig = Some(sigs.peers_redirect_tx_input_partial_signature);
            self.swap_tx_input_sig_ctx.peers_partial_sig = sigs.swap_tx_input_partial_signature;
        } else {
            self.sellers_warning_tx_buyer_input_sig_ctx.peers_partial_sig = Some(sigs.peers_warning_tx_buyer_input_partial_signature);
            self.sellers_warning_tx_seller_input_sig_ctx.peers_partial_sig = Some(sigs.peers_warning_tx_seller_input_partial_signature);
            self.sellers_redirect_tx_input_sig_ctx.peers_partial_sig = Some(sigs.peers_redirect_tx_input_partial_signature);

            // NOTE: The passed field here would normally be 'None'. The buyer should redact the field at the trade
            // start and reveal it later, after payment is started, to prevent premature trade closure by the seller:
            self.swap_tx_input_sig_ctx.peers_partial_sig = sigs.swap_tx_input_partial_signature;
        }
    }

    pub fn aggregate_partial_signatures(&mut self) -> Result<()> {
        if self.am_buyer() {
            self.buyers_warning_tx_buyer_input_sig_ctx.aggregate_partial_signatures(&self.buyer_output_key_ctx)?;
            self.buyers_warning_tx_seller_input_sig_ctx.aggregate_partial_signatures(&self.seller_output_key_ctx)?;
            self.buyers_redirect_tx_input_sig_ctx.aggregate_partial_signatures(&self.buyer_output_key_ctx)?;

            // This forms a validated adaptor signature on the swap tx for the buyer, ensuring that the seller's
            // private key share is revealed if the swap tx is published. The seller doesn't get the full adaptor
            // signature (or the ordinary signature) until later on in the trade, when the buyer confirms payment:
            self.swap_tx_input_sig_ctx.aggregate_partial_signatures(&self.buyer_output_key_ctx)?;
        } else {
            self.sellers_warning_tx_buyer_input_sig_ctx.aggregate_partial_signatures(&self.buyer_output_key_ctx)?;
            self.sellers_warning_tx_seller_input_sig_ctx.aggregate_partial_signatures(&self.seller_output_key_ctx)?;
            self.sellers_redirect_tx_input_sig_ctx.aggregate_partial_signatures(&self.seller_output_key_ctx)?;
        }
        Ok(())
    }

    pub fn set_swap_tx_input_peers_partial_signature(&mut self, sig: PartialSignature) {
        self.swap_tx_input_sig_ctx.peers_partial_sig = Some(sig);
    }

    pub fn aggregate_swap_tx_partial_signatures(&mut self) -> Result<()> {
        let my_key_ctx = if self.am_buyer() {
            &self.buyer_output_key_ctx
        } else {
            &self.seller_output_key_ctx
        };
        self.swap_tx_input_sig_ctx.aggregate_partial_signatures(my_key_ctx)?;
        Ok(())
    }

    pub fn get_my_private_key_share_for_peer_output(&self) -> Option<&Scalar> {
        // TODO: Check that it's actually safe to release the funds at this point.
        let peer_key_ctx = if self.am_buyer() {
            &self.seller_output_key_ctx
        } else {
            &self.buyer_output_key_ctx
        };
        Some(&peer_key_ctx.my_key_share.as_ref()?.prv_key)
    }

    //noinspection RsSelfConvention
    fn get_my_key_ctx_mut(&mut self) -> &mut KeyCtx {
        if self.am_buyer() {
            &mut self.buyer_output_key_ctx
        } else {
            &mut self.seller_output_key_ctx
        }
    }

    pub fn set_peer_private_key_share_for_my_output(&mut self, prv_key_share: Scalar) -> Result<()> {
        self.get_my_key_ctx_mut().peers_key_share.as_mut()
            .ok_or(ProtocolErrorKind::MissingKeyShare)?
            .set_prv_key(prv_key_share)?;
        Ok(())
    }

    pub fn aggregate_private_keys_for_my_output(&mut self) -> Result<&Scalar> {
        self.get_my_key_ctx_mut().aggregate_prv_key_shares()
    }
}

impl KeyPair {
    fn new() -> Self {
        Self::from_private(Scalar::one())
    }

    fn from_private(prv_key: Scalar) -> Self {
        Self { pub_key: prv_key.base_point_mul(), prv_key }
    }
}

impl KeyPair<ByOptVal> {
    const fn from_public(pub_key: Point) -> Self {
        Self { pub_key, prv_key: None }
    }

    fn set_prv_key(&mut self, prv_key: Scalar) -> Result<&Scalar> {
        if self.pub_key != prv_key.base_point_mul() {
            return Err(ProtocolErrorKind::MismatchedKeyPair);
        }
        Ok(self.prv_key.insert(prv_key))
    }
}

impl NoncePair {
    fn new(nonce_seed: impl Into<NonceSeed>, aggregated_pub_key: Point) -> Self {
        let sec_nonce = SecNonceBuilder::new(nonce_seed)
            .with_aggregated_pubkey(aggregated_pub_key)
            .build();
        Self { pub_nonce: sec_nonce.public_nonce(), sec_nonce: Some(sec_nonce) }
    }
}

impl KeyCtx {
    fn init_my_key_share(&mut self) -> &KeyPair {
        // FIXME: Obtains a dummy private key -- may need to pass a provider or RNG to the constructor.
        self.my_key_share.insert(KeyPair::new())
    }

    fn get_key_shares(&self) -> Option<[Point; 2]> {
        Some(if self.am_buyer {
            [self.my_key_share.as_ref()?.pub_key, self.peers_key_share.as_ref()?.pub_key]
        } else {
            [self.peers_key_share.as_ref()?.pub_key, self.my_key_share.as_ref()?.pub_key]
        })
    }

    fn aggregate_key_shares(&mut self) -> Result<()> {
        let agg_ctx = KeyAggContext::new(self.get_key_shares()
            .ok_or(ProtocolErrorKind::MissingKeyShare)?)?;
        self.aggregated_key = Some(KeyPair::from_public(agg_ctx.aggregated_pubkey()));
        self.key_agg_ctx = Some(agg_ctx);
        Ok(())
    }

    fn get_prv_key_shares(&self) -> Option<[Scalar; 2]> {
        Some(if self.am_buyer {
            [self.my_key_share.as_ref()?.prv_key, self.peers_key_share.as_ref()?.prv_key?]
        } else {
            [self.peers_key_share.as_ref()?.prv_key?, self.my_key_share.as_ref()?.prv_key]
        })
    }

    fn aggregate_prv_key_shares(&mut self) -> Result<&Scalar> {
        let prv_key_shares = self.get_prv_key_shares()
            .ok_or(ProtocolErrorKind::MissingKeyShare)?;
        let agg_ctx = self.key_agg_ctx.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggPubKey)?;
        let agg_key = self.aggregated_key.as_mut()
            .ok_or(ProtocolErrorKind::MissingAggPubKey)?;
        agg_key.set_prv_key(agg_ctx.aggregated_seckey(prv_key_shares)?)
    }
}

impl SigCtx {
    fn init_my_nonce_share(&mut self, key_ctx: &KeyCtx) -> Result<()> {
        // FIXME: Obtains a fixed nonce share -- must pass a _random_ seed data source to the constructor.
        let aggregated_pub_key = key_ctx.aggregated_key.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggPubKey)?.pub_key;
        self.my_nonce_share = Some(NoncePair::new([0; 32], aggregated_pub_key));
        Ok(())
    }

    fn get_nonce_shares(&self) -> Option<[&PubNonce; 2]> {
        Some(if self.am_buyer {
            [&self.my_nonce_share.as_ref()?.pub_nonce, self.peers_nonce_share.as_ref()?]
        } else {
            [self.peers_nonce_share.as_ref()?, &self.my_nonce_share.as_ref()?.pub_nonce]
        })
    }

    fn aggregate_nonce_shares(&mut self) -> Result<()> {
        // TODO: Should check that the aggregated nonce doesn't have a zero point & fail immediately
        //  otherwise. (No need to assign blame at the signing stage, as this is two-party.)
        self.aggregated_nonce = Some(AggNonce::sum(self.get_nonce_shares()
            .ok_or(ProtocolErrorKind::MissingKeyShare)?));
        Ok(())
    }

    fn sign_partial(&mut self, key_ctx: &KeyCtx, message: Vec<u8>) -> Result<&PartialSignature> {
        let key_agg_ctx = key_ctx.key_agg_ctx.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggPubKey)?;
        let seckey = key_ctx.my_key_share.as_ref()
            .ok_or(ProtocolErrorKind::MissingKeyShare)?.prv_key;
        let secnonce = self.my_nonce_share.as_mut()
            .ok_or(ProtocolErrorKind::MissingNonceShare)?.sec_nonce.take()
            .ok_or(ProtocolErrorKind::NonceReuse)?;
        let aggregated_nonce = &self.aggregated_nonce.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggNonce)?;

        let sig = musig2::adaptor::sign_partial(key_agg_ctx, seckey, secnonce, aggregated_nonce,
            self.adaptor_point, &message[..])?;
        self.message = Some(message);
        Ok(self.my_partial_sig.insert(sig))
    }

    fn get_partial_signatures(&self) -> Option<[PartialSignature; 2]> {
        Some(if self.am_buyer {
            [self.my_partial_sig?, self.peers_partial_sig?]
        } else {
            [self.peers_partial_sig?, self.my_partial_sig?]
        })
    }

    fn aggregate_partial_signatures(&mut self, key_ctx: &KeyCtx) -> Result<&AdaptorSignature> {
        let key_agg_ctx = key_ctx.key_agg_ctx.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggPubKey)?;
        let aggregated_nonce = &self.aggregated_nonce.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggNonce)?;
        let partial_signatures = self.get_partial_signatures()
            .ok_or(ProtocolErrorKind::MissingPartialSig)?;
        let message = &self.message.as_ref()
            .ok_or(ProtocolErrorKind::MissingPartialSig)?[..];

        let sig = musig2::adaptor::aggregate_partial_signatures(key_agg_ctx, aggregated_nonce,
            self.adaptor_point, partial_signatures, message)?;
        Ok(self.aggregated_sig.insert(sig))
    }
}

type Result<T> = std::result::Result<T, ProtocolErrorKind>;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ProtocolErrorKind {
    #[error("missing key share")]
    MissingKeyShare,
    #[error("missing nonce share")]
    MissingNonceShare,
    #[error("missing partial signature")]
    MissingPartialSig,
    #[error("missing aggregated pubkey")]
    MissingAggPubKey,
    #[error("missing aggregated nonce")]
    MissingAggNonce,
    #[error("nonce has already been used")]
    NonceReuse,
    #[error("public-private key mismatch")]
    MismatchedKeyPair,
    KeyAgg(#[from] musig2::errors::KeyAggError),
    Signing(#[from] musig2::errors::SigningError),
    Verify(#[from] musig2::errors::VerifyError),
    InvalidSecretKeys(#[from] musig2::errors::InvalidSecretKeysError),
    ZeroScalar(#[from] secp::errors::ZeroScalarError),
}
