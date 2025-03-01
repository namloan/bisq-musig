### Test "Hello World" gRPC server and client

The Rust gRPC server listens on localhost port 50051.

1. To successfully build the Rust server, make sure the `PROTOC` environment variable is set to the path of the `protoc`
   binary, which needs to be installed separately. It can be downloaded from:

> https://github.com/protocolbuffers/protobuf/releases

2. To build and run the Rust server, run:

```sh
cargo run --bin server
```

3. To build and run the Java gRPC client, run:

```sh
mvn install exec:java
```

### Experimental gGRP interface for Bisq2 Musig2 trade protocol

There is a (highly) experimental gRPC interface being developed for the Musig2 trade protocol, currently bundled in the
same `helloworld.proto` file as the above. (TODO: Organise and move to a more appropriate place.) A Java client
conducting a dummy two-party trade can be invoked by running:

```sh
mvn exec:java -Pmusig
```

The Rust code uses the `musig2` crate to construct aggregated signatures for the traders' warning and redirect
transactions, with pubkey & nonce shares and partial signatures exchanged with the Java client, to pass them back in as
fields of the simulated peer's RPC requests, setting up the trade.

The adaptor logic, swap transaction signing and simulated steps for the rest of the trade are not yet implemented for
the mockup. Dummy messages to represent the txs to sign are currently being used in place of real txs built with the aid
of BDK or similar wallet dependency.

See [MuSig trade protocol messages](musig-trade-protocol-messages.txt) for my current (incomplete) picture of what the
trade messages between the peers would look like, and thus the necessary data to exchange in an RPC interface between
the Bisq2 client and the Rust server managing the wallet and key material.
