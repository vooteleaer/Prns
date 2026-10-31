# Prns on Haiku

The `haiku` branch is Prns with what it takes to build and run on Haiku
(x86_64): a short stack of commits on a Prns release tag, each one small and
behind `cfg(target_os = "haiku")` or a target-specific dependency, so that
they can go upstream one by one.

## What changes on Haiku

- **TCP keepalive:** socket2 has no keepalive interval or retry count on
  Haiku; only the idle time is set there.
- **browser-rendezvous without LAN discovery:** `mdns-sd` needs the receiving
  interface of each datagram (`IP_PKTINFO`, through `socket-pktinfo`), which
  Haiku's IPv4 stack does not report (it has `IPV6_PKTINFO` and
  `IP_RECVDSTADDR` only). Gateways are not advertised or found by mDNS there;
  `wifi-auto-mdns` is unavailable.
- **if-addrs:** `get_if_addrs()` is `getifaddrs()` (in Haiku's libnetwork);
  only the netlink change notifier is left out, as on the BSDs. Carried in
  `third_party/if-addrs` and patched into prnsd's workspace until upstream
  has it.
- **No netdev:** no Haiku support; its only user, `wifi-auto`, is unavailable.
- **mio:** at least 1.2.4 (non-blocking socket creation on Haiku), plus
  `O_NONBLOCK` set after `accept` on Haiku (as proposed to mio upstream): an
  accepted socket there does not inherit it from the listener, and mio
  assumed it does, so the first local
  client of the shared instance got a blocking socket and the runtime stopped
  (signals too). Carried in `third_party/mio` until upstream has it; every
  Rust program using tokio on Haiku needs it (retifsd's local API too).
- **Lifecycle locks:** Rust's std has no `File::try_lock` on Haiku ("try_lock()
  not supported"); prnsd's runtime and control locks use `flock(2)` there,
  through a Haiku-only `prns-ffi::file_lock`.

So far: `tokio-cloud-host` (TCP, UDP, backbone, I2P, WebSocket, shared
instance, …). Tested on Haiku nightly hrev60299 (x86_64, a QEMU VM): a TCP
connection to another Prns node, the shared instance with a local client
(retifsd) attached, status over RPC, a clean shutdown on SIGTERM. Serial and
RNode (`tokio-host`'s radio side) are next.

## Building on Haiku

```sh
pkgman install rust_bin gcc binutils haiku_devel
RUST_MIN_STACK=16777216 cargo build --release --locked --ignore-rust-version \
    --manifest-path prnsd/Cargo.toml --no-default-features \
    --features tokio-cloud-host,observability
```

- `RUST_MIN_STACK`: `.cargo/config.toml` sets 256 MiB for everything cargo
  runs, rustc included, and Haiku refuses a thread stack that large ("invalid
  stack size"); a value in the environment wins over the config.
- `--ignore-rust-version`: HaikuPorts has Rust 1.94; Prns asks for 1.96 but
  builds with it.

## Following Prns

```sh
git fetch origin --tags
git rebase --onto <new tag> <old tag> haiku
```

When Prns moves to another if-addrs version, `third_party/if-addrs` follows
(the Haiku change is the same few lines in `src/lib.rs`).
