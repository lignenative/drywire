# drywire-wasm

`drywire-proto` as a WebAssembly module, for the web client
([`apps/web/`](../../apps/web/)). It builds the `hello`, checks the
`hello_ack`, builds the sender announcement from a signature the page
made with WebCrypto, seals our audio, and verifies and opens everyone
else's, with the replay window: the same code the native clients run,
checked by the same vectors (ADR 0040).

The module has no wasm-bindgen and no JavaScript glue: it exports plain
functions (`dw_*`, in [`src/exports.rs`](src/exports.rs)) and one shared
buffer, so it loads with `WebAssembly.instantiate` in a page, a worker or
an `AudioWorklet`.

```sh
rustup target add wasm32-unknown-unknown   # once
cargo build -p drywire-wasm --release --target wasm32-unknown-unknown
# → target/wasm32-unknown-unknown/release/drywire_wasm.wasm
cargo test -p drywire-wasm                 # the logic, natively
```

The web client's own tests load the built module and run it against
`drywire-proto/vectors/`.
