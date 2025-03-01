mod protocol;

use futures::stream;
use helloworld::{ClockRequest, DepositPsbt, DepositTxSignatureRequest, HelloReply, HelloRequest,
    NonceShareMessage, NonceShareRequest, PartialSignatureMessage, PartialSignatureRequest,
    PubKeyShareRequest, PubKeyShareResponse, PublishDepositTxRequest, TickEvent, TxConfirmationStatus};
use helloworld::greeter_server::{Greeter, GreeterServer};
use helloworld::mu_sig_server::{MuSig, MuSigServer};
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

use crate::protocol::{ProtocolErrorKind, Role, TradeModel, TradeModelStore, TxInputParamVector, TRADE_MODELS};

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

#[tonic::async_trait]
impl MuSig for MyMuSig {
    async fn init_trade(&self, request: Request<PubKeyShareRequest>) -> Result<Response<PubKeyShareResponse>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let mut trade_model = TradeModel::new(request.trade_id, request.my_role.my_try_into()?);
        trade_model.init_my_key_shares();
        let my_key_shares = trade_model.get_my_key_shares()
            .ok_or_else(|| Status::internal("missing key shares"))?;
        let response = PubKeyShareResponse {
            buyer_output_pub_key_share: my_key_shares[0].pub_key.serialize().into(),
            seller_output_pub_key_share: my_key_shares[1].pub_key.serialize().into(),
            current_block_height: 80000,
        };
        TRADE_MODELS.add_trade_model(trade_model);

        Ok(Response::new(response))
    }

    async fn get_nonce_shares(&self, request: Request<NonceShareRequest>) -> Result<Response<NonceShareMessage>, Status> {
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
        let response = NonceShareMessage {
            warning_tx_fee_bump_address: "address1".to_string(),
            redirect_tx_fee_bump_address: "address2".to_string(),
            half_deposit_psbt: vec![],
            swap_tx_input_nonce_share: my_nonce_shares.swap_tx_input_param.serialize().into(),
            buyers_warning_tx_buyer_input_nonce_share: my_nonce_shares.buyers_warning_tx_buyer_input_param.serialize().into(),
            buyers_warning_tx_seller_input_nonce_share: my_nonce_shares.buyers_warning_tx_seller_input_param.serialize().into(),
            sellers_warning_tx_buyer_input_nonce_share: my_nonce_shares.sellers_warning_tx_buyer_input_param.serialize().into(),
            sellers_warning_tx_seller_input_nonce_share: my_nonce_shares.sellers_warning_tx_seller_input_param.serialize().into(),
            buyers_redirect_tx_input_nonce_share: my_nonce_shares.buyers_redirect_tx_input_param.serialize().into(),
            sellers_redirect_tx_input_nonce_share: my_nonce_shares.sellers_redirect_tx_input_param.serialize().into(),
        };

        Ok(Response::new(response))
    }

    async fn get_partial_signatures(&self, request: Request<PartialSignatureRequest>) -> Result<Response<PartialSignatureMessage>, Status> {
        println!("Got a request: {:?}", request);

        let request = request.into_inner();
        let trade_model = TRADE_MODELS.get_trade_model(&request.trade_id)
            .ok_or_else(|| Status::not_found(format!("missing trade with id: {}", request.trade_id)))?;
        let mut trade_model = trade_model.lock().unwrap();
        let peer_nonce_shares = request.peers_nonce_shares
            .ok_or_else(|| Status::not_found("missing request.peers_nonce_shares"))?;
        trade_model.set_peer_nonce_shares(TxInputParamVector {
            swap_tx_input_param: peer_nonce_shares.swap_tx_input_nonce_share.my_try_into()?,
            buyers_warning_tx_buyer_input_param: peer_nonce_shares.buyers_warning_tx_buyer_input_nonce_share.my_try_into()?,
            buyers_warning_tx_seller_input_param: peer_nonce_shares.buyers_warning_tx_seller_input_nonce_share.my_try_into()?,
            sellers_warning_tx_buyer_input_param: peer_nonce_shares.sellers_warning_tx_buyer_input_nonce_share.my_try_into()?,
            sellers_warning_tx_seller_input_param: peer_nonce_shares.sellers_warning_tx_seller_input_nonce_share.my_try_into()?,
            buyers_redirect_tx_input_param: peer_nonce_shares.buyers_redirect_tx_input_nonce_share.my_try_into()?,
            sellers_redirect_tx_input_param: peer_nonce_shares.sellers_redirect_tx_input_nonce_share.my_try_into()?,
        });
        trade_model.aggregate_nonce_shares()?;
        trade_model.sign_partial()?;
        let my_partial_signatures = trade_model.get_my_partial_signatures_on_peer_txs()
            .ok_or_else(|| Status::internal("missing partial signatures"))?;
        let response = PartialSignatureMessage {
            peers_warning_tx_buyer_input_partial_signature: my_partial_signatures[0].serialize().into(),
            peers_warning_tx_seller_input_partial_signature: my_partial_signatures[1].serialize().into(),
            peers_redirect_tx_input_partial_signature: my_partial_signatures[2].serialize().into(),
            swap_tx_input_adaptor_signature: None,
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
        trade_model.set_peer_partial_signatures_on_my_txs([
            peers_partial_signatures.peers_warning_tx_buyer_input_partial_signature.my_try_into()?,
            peers_partial_signatures.peers_warning_tx_seller_input_partial_signature.my_try_into()?,
            peers_partial_signatures.peers_redirect_tx_input_partial_signature.my_try_into()?
        ]);
        trade_model.aggregate_partial_signatures()?;
        let response = DepositPsbt {
            deposit_psbt: b"deposit_psbt".into()
        };

        Ok(Response::new(response))
    }

    type PublishDepositTxStream = Pin<Box<dyn stream::Stream<Item=Result<TxConfirmationStatus, Status>> + Send>>;

    async fn publish_deposit_tx(&self, _: Request<PublishDepositTxRequest>) -> Result<Response<Self::PublishDepositTxStream>, Status> {
        Err(Status::unimplemented("not implemented"))
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

impl MyTryInto<Point> for Vec<u8> {
    fn my_try_into(self) -> Result<Point, Status> {
        (&self[..]).try_into().map_err(|_| Status::invalid_argument("could not decode point"))
    }
}

impl MyTryInto<PubNonce> for Vec<u8> {
    fn my_try_into(self) -> Result<PubNonce, Status> {
        (&self[..]).try_into().map_err(|_| Status::invalid_argument("could not decode pub nonce"))
    }
}

impl MyTryInto<Scalar> for Vec<u8> {
    fn my_try_into(self) -> Result<Scalar, Status> {
        (&self[..]).try_into().map_err(|_| Status::invalid_argument("could not decode scalar"))
    }
}

impl MyTryInto<MaybeScalar> for Vec<u8> {
    fn my_try_into(self) -> Result<MaybeScalar, Status> {
        (&self[..]).try_into().map_err(|_| Status::invalid_argument("could not decode scalar"))
    }
}

impl MyTryInto<Role> for i32 {
    fn my_try_into(self) -> Result<Role, Status> {
        TryInto::<helloworld::Role>::try_into(self)
            .map_err(|UnknownEnumValue(i)| Status::out_of_range(format!("unknown enum value: {}", i)))
            .map(Into::into)
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
