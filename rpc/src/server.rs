mod protocol;
mod storage;

use futures::stream;
use helloworld::{ClockRequest, CloseTradeRequest, CloseTradeResponse, DepositPsbt,
    DepositTxSignatureRequest, HelloReply, HelloRequest, NonceSharesMessage, NonceSharesRequest,
    PartialSignaturesMessage, PartialSignaturesRequest, PubKeySharesRequest, PubKeySharesResponse,
    PublishDepositTxRequest, SwapTxSignatureRequest, SwapTxSignatureResponse, TickEvent,
    TxConfirmationStatus};
use helloworld::greeter_server::{Greeter, GreeterServer};
use helloworld::mu_sig_server::{MuSig, MuSigServer};
use helloworld::partial_signatures_message::SwapTxInputScalar::{SwapTxInputBuyersPartialSignature,
    SwapTxInputSellersAdaptor};
use musig2::PubNonce;
use prost::UnknownEnumValue;
use secp::{Point, MaybeScalar, Scalar};
use std::pin::Pin;
use std::prelude::rust_2021::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};
use tonic::transport::Server;

use crate::protocol::{ExchangedNonces, ExchangedSigs, ProtocolErrorKind, Role, SwapTxInputScalar,
    TradeModel, TradeModelStore, TRADE_MODELS};
use crate::storage::{ByRef, ByVal};

pub mod helloworld {
    tonic::include_proto!("helloworld");
}

#[derive(Default, Debug)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(&self, request: Request<HelloRequest>) -> Result<Response<HelloReply>, Status> {
        println!("Got a request: {:?}", request);

        let reply = HelloReply {
            message: format!("Hello, {}!", request.into_inner().name)
        };

        Ok(Response::new(reply))
    }

    type SubscribeClockStream = Pin<Box<dyn stream::Stream<Item=Result<TickEvent, Status>> + Send>>;

    async fn subscribe_clock(&self, request: Request<ClockRequest>) -> Result<Response<Self::SubscribeClockStream>, Status> {
        println!("Got a request: {:?}", request);

        let period = Duration::from_millis(request.into_inner().tick_period_millis as u64);

        Ok(Response::new(Box::pin(stream::repeat(())
            .throttle(period)
            .map(|()| Ok(TickEvent { current_time_millis: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64 })))))
    }
}

#[derive(Default, Debug)]
pub struct MyMuSig {}

// FIXME: At present, the MuSig service passes some fields to the Java client that should be kept
//  secret for a time before passing them to the peer, namely the buyer's partial signature on the
//  swap tx and the seller's private key share for the buyer payout. Premature revelation of those
//  secrets would allow the seller to close the trade before the buyer starts payment, or the buyer
//  to close the trade before the seller had a chance to confirm receipt of payment (but after the
//  buyer starts payment), respectively. This should probably be changed, as the Java client should
//  never hold secrets which directly control funds (but doing so makes the RPC interface a little
//  bigger and less symmetrical.)
#[tonic::async_trait]
impl MuSig for MyMuSig {
    async fn init_trade(&self, request: Request<PubKeySharesRequest>) -> Result<Response<PubKeySharesResponse>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let mut trade_model = TradeModel::new(request.trade_id, request.my_role.my_try_into()?);
        trade_model.init_my_key_shares();
        let my_key_shares = trade_model.get_my_key_shares()
            .ok_or_else(|| Status::internal("missing key shares"))?;
        let response = PubKeySharesResponse {
            buyer_output_pub_key_share: my_key_shares[0].pub_key.serialize().into(),
            seller_output_pub_key_share: my_key_shares[1].pub_key.serialize().into(),
            current_block_height: 900000,
        };
        TRADE_MODELS.add_trade_model(trade_model);

        Ok(Response::new(response))
    }

    async fn get_nonce_shares(&self, request: Request<NonceSharesRequest>) -> Result<Response<NonceSharesMessage>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let trade_model = TRADE_MODELS.get_trade_model(&request.trade_id)
            .ok_or_else(|| Status::not_found(format!("missing trade with id: {}", request.trade_id)))?;
        let mut trade_model = trade_model.lock().unwrap();
        trade_model.set_peer_key_shares(
            request.buyer_output_peers_pub_key_share.my_try_into()?,
            request.seller_output_peers_pub_key_share.my_try_into()?);
        trade_model.aggregate_key_shares()?;
        trade_model.init_my_nonce_shares()?;
        trade_model.trade_amount = Some(request.trade_amount);
        trade_model.buyers_security_deposit = Some(request.buyers_security_deposit);
        trade_model.sellers_security_deposit = Some(request.sellers_security_deposit);
        trade_model.deposit_tx_fee_rate = Some(request.deposit_tx_fee_rate);
        trade_model.prepared_tx_fee_rate = Some(request.prepared_tx_fee_rate);
        let my_nonce_shares = trade_model.get_my_nonce_shares()
            .ok_or_else(|| Status::internal("missing nonce shares"))?;
        let response = NonceSharesMessage {
            warning_tx_fee_bump_address: "address1".to_string(),
            redirect_tx_fee_bump_address: "address2".to_string(),
            half_deposit_psbt: vec![],
            swap_tx_input_nonce_share:
            my_nonce_shares.swap_tx_input_nonce_share.serialize().into(),
            buyers_warning_tx_buyer_input_nonce_share:
            my_nonce_shares.buyers_warning_tx_buyer_input_nonce_share.serialize().into(),
            buyers_warning_tx_seller_input_nonce_share:
            my_nonce_shares.buyers_warning_tx_seller_input_nonce_share.serialize().into(),
            sellers_warning_tx_buyer_input_nonce_share:
            my_nonce_shares.sellers_warning_tx_buyer_input_nonce_share.serialize().into(),
            sellers_warning_tx_seller_input_nonce_share:
            my_nonce_shares.sellers_warning_tx_seller_input_nonce_share.serialize().into(),
            buyers_redirect_tx_input_nonce_share:
            my_nonce_shares.buyers_redirect_tx_input_nonce_share.serialize().into(),
            sellers_redirect_tx_input_nonce_share:
            my_nonce_shares.sellers_redirect_tx_input_nonce_share.serialize().into(),
        };

        Ok(Response::new(response))
    }

    async fn get_partial_signatures(&self, request: Request<PartialSignaturesRequest>) -> Result<Response<PartialSignaturesMessage>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let trade_model = TRADE_MODELS.get_trade_model(&request.trade_id)
            .ok_or_else(|| Status::not_found(format!("missing trade with id: {}", request.trade_id)))?;
        let mut trade_model = trade_model.lock().unwrap();
        let peer_nonce_shares = request.peers_nonce_shares
            .ok_or_else(|| Status::not_found("missing request.peers_nonce_shares"))?;
        trade_model.set_peer_nonce_shares(ExchangedNonces {
            swap_tx_input_nonce_share:
            peer_nonce_shares.swap_tx_input_nonce_share.my_try_into()?,
            buyers_warning_tx_buyer_input_nonce_share:
            peer_nonce_shares.buyers_warning_tx_buyer_input_nonce_share.my_try_into()?,
            buyers_warning_tx_seller_input_nonce_share:
            peer_nonce_shares.buyers_warning_tx_seller_input_nonce_share.my_try_into()?,
            sellers_warning_tx_buyer_input_nonce_share:
            peer_nonce_shares.sellers_warning_tx_buyer_input_nonce_share.my_try_into()?,
            sellers_warning_tx_seller_input_nonce_share:
            peer_nonce_shares.sellers_warning_tx_seller_input_nonce_share.my_try_into()?,
            buyers_redirect_tx_input_nonce_share:
            peer_nonce_shares.buyers_redirect_tx_input_nonce_share.my_try_into()?,
            sellers_redirect_tx_input_nonce_share:
            peer_nonce_shares.sellers_redirect_tx_input_nonce_share.my_try_into()?,
        });
        trade_model.aggregate_nonce_shares()?;
        trade_model.sign_partial()?;
        let my_partial_signatures = trade_model.get_my_partial_signatures_on_peer_txs()
            .ok_or_else(|| Status::internal("missing partial signatures"))?;
        let response = PartialSignaturesMessage {
            peers_warning_tx_buyer_input_partial_signature:
            my_partial_signatures.peers_warning_tx_buyer_input_partial_signature.serialize().into(),
            peers_warning_tx_seller_input_partial_signature:
            my_partial_signatures.peers_warning_tx_seller_input_partial_signature.serialize().into(),
            peers_redirect_tx_input_partial_signature:
            my_partial_signatures.peers_redirect_tx_input_partial_signature.serialize().into(),
            swap_tx_input_scalar:
            my_partial_signatures.swap_tx_input_scalar.map(Into::into),
        };

        Ok(Response::new(response))
    }

    async fn sign_deposit_tx(&self, request: Request<DepositTxSignatureRequest>) -> Result<Response<DepositPsbt>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let trade_model = TRADE_MODELS.get_trade_model(&request.trade_id)
            .ok_or_else(|| Status::not_found(format!("missing trade with id: {}", request.trade_id)))?;
        let mut trade_model = trade_model.lock().unwrap();
        let peers_partial_signatures = request.peers_partial_signatures
            .ok_or_else(|| Status::not_found("missing request.peers_partial_signatures"))?;
        trade_model.set_peer_partial_signatures_on_my_txs(ExchangedSigs {
            peers_warning_tx_buyer_input_partial_signature:
            peers_partial_signatures.peers_warning_tx_buyer_input_partial_signature.my_try_into()?,
            peers_warning_tx_seller_input_partial_signature:
            peers_partial_signatures.peers_warning_tx_seller_input_partial_signature.my_try_into()?,
            peers_redirect_tx_input_partial_signature:
            peers_partial_signatures.peers_redirect_tx_input_partial_signature.my_try_into()?,
            swap_tx_input_scalar:
            peers_partial_signatures.swap_tx_input_scalar.as_ref().my_try_into()?,
        });
        trade_model.aggregate_partial_signatures()?;
        let response = DepositPsbt {
            deposit_psbt: b"deposit_psbt".into()
        };

        Ok(Response::new(response))
    }

    type PublishDepositTxStream = Pin<Box<dyn stream::Stream<Item=Result<TxConfirmationStatus, Status>> + Send>>;

    async fn publish_deposit_tx(&self, request: Request<PublishDepositTxRequest>) -> Result<Response<Self::PublishDepositTxStream>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let trade_model = TRADE_MODELS.get_trade_model(&request.trade_id)
            .ok_or_else(|| Status::not_found(format!("missing trade with id: {}", request.trade_id)))?;
        let mut _trade_model = trade_model.lock().unwrap();

        // TODO: *** BROADCAST DEPOSIT TX ***

        let confirmation_event = TxConfirmationStatus {
            tx: b"signed_deposit_tx".into(),
            current_block_height: 900001,
            num_confirmations: 1,
        };

        Ok(Response::new(Box::pin(stream::iter([Ok(confirmation_event)].into_iter()))))
    }

    async fn sign_swap_tx(&self, request: Request<SwapTxSignatureRequest>) -> Result<Response<SwapTxSignatureResponse>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let trade_model = TRADE_MODELS.get_trade_model(&request.trade_id)
            .ok_or_else(|| Status::not_found(format!("missing trade with id: {}", request.trade_id)))?;
        let mut trade_model = trade_model.lock().unwrap();
        trade_model.set_swap_tx_input_peers_partial_signature(request.swap_tx_input_peers_partial_signature.my_try_into()?)?;
        trade_model.aggregate_swap_tx_partial_signatures()?;
        let prv_key_share = trade_model.get_my_private_key_share_for_peer_output()
            .ok_or_else(|| Status::internal("missing private key share"))?;
        let response = SwapTxSignatureResponse {
            swap_tx: b"signed_swap_tx".into(),
            peer_output_prv_key_share: prv_key_share.serialize().into(),
        };

        Ok(Response::new(response))
    }

    async fn close_trade(&self, request: Request<CloseTradeRequest>) -> Result<Response<CloseTradeResponse>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let trade_model = TRADE_MODELS.get_trade_model(&request.trade_id)
            .ok_or_else(|| Status::not_found(format!("missing trade with id: {}", request.trade_id)))?;
        let mut trade_model = trade_model.lock().unwrap();
        if let Some(peer_prv_key_share) = request.my_output_peers_prv_key_share.my_try_into()? {
            trade_model.set_peer_private_key_share_for_my_output(peer_prv_key_share)?;
        } else {
            // TODO: *** BROADCAST SWAP TX ***
        }
        let my_prv_key_share = trade_model.get_my_private_key_share_for_peer_output()
            .ok_or_else(|| Status::internal("missing private key share"))?;
        let response = CloseTradeResponse {
            peer_output_prv_key_share: my_prv_key_share.serialize().into(),
        };

        Ok(Response::new(response))
    }
}

impl From<helloworld::Role> for Role {
    fn from(value: helloworld::Role) -> Self {
        match value {
            helloworld::Role::SellerAsMaker => Role::SellerAsMaker,
            helloworld::Role::SellerAsTaker => Role::SellerAsTaker,
            helloworld::Role::BuyerAsMaker => Role::BuyerAsMaker,
            helloworld::Role::BuyerAsTaker => Role::BuyerAsTaker
        }
    }
}

impl From<ProtocolErrorKind> for Status {
    fn from(value: ProtocolErrorKind) -> Self {
        Status::internal(value.to_string())
    }
}

trait MyTryInto<T> {
    fn my_try_into(self) -> Result<T, Status>;
}

impl MyTryInto<Point> for &[u8] {
    fn my_try_into(self) -> Result<Point, Status> {
        self.try_into().map_err(|_| Status::invalid_argument("could not decode point"))
    }
}

impl MyTryInto<PubNonce> for &[u8] {
    fn my_try_into(self) -> Result<PubNonce, Status> {
        self.try_into().map_err(|_| Status::invalid_argument("could not decode pub nonce"))
    }
}

impl MyTryInto<Scalar> for &[u8] {
    fn my_try_into(self) -> Result<Scalar, Status> {
        self.try_into().map_err(|_| Status::invalid_argument("could not decode scalar"))
    }
}

impl MyTryInto<MaybeScalar> for &[u8] {
    fn my_try_into(self) -> Result<MaybeScalar, Status> {
        self.try_into().map_err(|_| Status::invalid_argument("could not decode scalar"))
    }
}

impl MyTryInto<Role> for i32 {
    fn my_try_into(self) -> Result<Role, Status> {
        TryInto::<helloworld::Role>::try_into(self)
            .map_err(|UnknownEnumValue(i)| Status::out_of_range(format!("unknown enum value: {}", i)))
            .map(Into::into)
    }
}

impl<'a> From<SwapTxInputScalar<'a, ByRef>> for helloworld::partial_signatures_message::SwapTxInputScalar {
    fn from(value: SwapTxInputScalar<'a, ByRef>) -> Self {
        match value {
            SwapTxInputScalar::BuyersPartialSignature(s) => SwapTxInputBuyersPartialSignature(s.serialize().into()),
            SwapTxInputScalar::SellersAdaptor(a) => SwapTxInputSellersAdaptor(a.serialize().into()),
        }
    }
}

impl<'a> MyTryInto<SwapTxInputScalar<'a, ByVal>> for &'a helloworld::partial_signatures_message::SwapTxInputScalar {
    fn my_try_into(self) -> Result<SwapTxInputScalar<'a, ByVal>, Status> {
        Ok(match self {
            SwapTxInputBuyersPartialSignature(s) => SwapTxInputScalar::BuyersPartialSignature((&s[..]).my_try_into()?),
            SwapTxInputSellersAdaptor(a) => SwapTxInputScalar::SellersAdaptor((&a[..]).my_try_into()?),
        })
    }
}

impl<T> MyTryInto<T> for Vec<u8> where for<'a> &'a [u8]: MyTryInto<T> {
    fn my_try_into(self) -> Result<T, Status> { (&self[..]).my_try_into() }
}

impl<T, S: MyTryInto<T>> MyTryInto<Option<T>> for Option<S> {
    fn my_try_into(self) -> Result<Option<T>, Status> {
        Ok(match self {
            None => None,
            Some(x) => Some(x.my_try_into()?)
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:50051".parse()?;
    let greeter = MyGreeter::default();
    let musig = MyMuSig::default();

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .add_service(MuSigServer::new(musig))
        .serve(addr)
        .await?;

    Ok(())
}
