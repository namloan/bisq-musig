### Test "Hello World" gRPC server and client

The Rust gRPC server listens on localhost port 50051.

1. To successfully build the Rust server, make sure the `PROTOC` environment variable is set to the path of the
   `protoc` binary, which needs to be installed separately. It can be downloaded from:

> https://github.com/protocolbuffers/protobuf/releases

2. To build and run the Rust server, run:

```sh
cargo run --bin server
```

3. To build and run the Java gRPC client, run:

```sh
mvn install exec:java
```
