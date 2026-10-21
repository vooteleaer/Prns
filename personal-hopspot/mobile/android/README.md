# Personal Hopspot — Android

The Android face of Personal Hopspot. `personal-hopspot-core` renders the same
canonical 64×128 monochrome frame used by embedded and desktop Hopspots, then
the Android adapter expands that frame into the app's native RGBA surface. This
crate adds the other platform adapters Android needs:

- canonical-frame expansion into a flat RGBA framebuffer (`rust/src/face.rs`)
- a single-button input source: every tap is a `ShortPress`, every hold a
  `LongPress` (`rust/src/face.rs` + the `nativePostInput` entry point)
- Android-hosted USB Auto (`app/src/main/java/org/personal/hopspot/UsbLink.kt`)
- Wi-Fi Auto/mDNS and Bluetooth LE Auto bridges into the shared Rust engine

`rust/` is the JNI `cdylib`. The Kotlin app shell in `app/` hosts it with a
plain Android `View` so the same APK can run on old Android devices as well as
modern phones.

## Native ABI — `org.personal.hopspot.NativeBridge`

```
nativeInit(storageDir) -> long handle
nativePostInput(handle, code) -> int action   // code: 0 = tap, 1 = hold; action: 0 = none, 1 = announce, 2 = copy shared-instance config
nativeRender(handle, directByteBuffer)         // fills PANEL_WIDTH * PANEL_HEIGHT * 4 RGBA bytes
nativeFree(handle)
```

The render path is pull-model and zero-copy: Kotlin owns a direct `ByteBuffer`,
Rust draws the current `UiState` into it, Kotlin blits it nearest-neighbor into an
`ARGB_8888` `Bitmap`. The panel is 64x128; bytes are `[R, G, B, A]` per pixel.

## Dashboard

The launcher screen (`DashboardActivity`) shows the node's state, how other apps reach
the shared instance, and every interface:

- **Built-in interfaces** are the radios the app drives itself (Wi-Fi Auto, Bluetooth
  LE Auto, USB Auto, Wi-Fi Direct/Aware), plus the shared-instance listener and the apps
  attached to it. The radios can be switched on and off.
- **Configured interfaces** live in a Reticulum-style `config` file in the app's files
  directory, the same format `prnsd` reads. The dashboard adds, edits, enables, disables and
  removes TCP client/server, UDP, Backbone, I2P and WebSocket interfaces. Every edit is
  validated against the whole file before it is written, then the node detaches and
  re-attaches the configured interfaces without a restart. Settings the form does not show
  survive an edit, so hand-tuned advanced keys are not lost.

The Hopspot face (`MainActivity`) is still one tap away and still receives USB attach intents.

## Hub interface for other apps

The app doubles as a phone-wide Reticulum hub: one engine, one set of radios, and
any installed app can route through it. There are two doors in, and both are
open to every app, just as a desktop shared instance is open to every local
process:

- **Stock Reticulum apps** (Sideband, Python RNS) join the loopback shared
  instance on `127.0.0.1:37428`, with RPC on `37429`. The long-press "copy
  shared-instance config" action and the hub's `getSharedInstanceInfo()` both
  hand out a complete client config, including the RPC key.
- **Android apps** bind `PrnsHubService` (action
  `org.personal.hopspot.action.BIND_PRNS_HUB`) through the typed AIDL contract
  in [`api/`](api/), most easily via the [`client/`](client/README.md) library.
  `openLocalClient()` returns one end of a socketpair that the engine attaches
  as a shared-instance local client. It carries the same HDLC-framed packets as
  the loopback listener, but without a TCP port and tagged with the caller's uid.

The hub never starts the engine on a client's behalf, because Android does not
let a background bind start a foreground service. A client that finds the
engine stopped sends the user to the app with `PrnsHubClient.launchHubIntent()`.

The older signature-protected Messenger interface (`BIND_PRNS_CLIENT` on
`PrnsService`) is kept for same-signer apps and the instrumentation probes.

[`samples/hub-consumer/`](samples/hub-consumer/) is a minimal third-party app.
It binds the hub, shows its status, and holds a local client open:

```
./gradlew :samples:hub-consumer:installDebug
```

## USB role model

Android Hopspot supports both Android-side USB roles where the OS permits them.
A phone with OTG support can act as the host for an attached embedded Hopspot or
RNode-style device. A desktop host can also negotiate Android Open Accessory
(AOA), after which the phone exposes an app-owned accessory stream and the same
Prns USB Auto wire runs over that stream.

`UsbLink.kt` deliberately keeps the consumer-facing idea as "USB Auto" while the
platform adapter chooses the Android transport:

- direct vendor bulk endpoints for the unified Prns USB Auto VID/PID
  (`1209:0001`)
- CDC ACM serial as a fallback for older development-board and legacy firmware
  shapes
- Android Open Accessory streams for the phone-as-device direction

Stock Android apps are not arbitrary USB gadgets: a Pixel can expose system USB
functions such as `adb`, MTP, or tethering, but an ordinary app cannot simply
publish the raw Prns `1209:0001` gadget endpoint. The phone-as-device direction
therefore uses a distinct physical lane: Android Open Accessory negotiation by
the external host, followed by the same Prns USB Auto wire over the accessory
bulk stream. That gives phones the "take either role when possible" behavior
without pretending every platform exposes the same low-level USB function.

## Toolchain (installed and verified on this machine)

Installed via Homebrew (no sudo) + sdkmanager; env is persisted in `~/.zshrc`:

- JDK 17 (`openjdk@17`) — `JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home`
- Android SDK at `ANDROID_HOME=/opt/homebrew/share/android-commandlinetools` (platform-tools, platform android-34, build-tools 34.0.0)
- NDK r27c — `ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.2.12479018`
- Rust targets `aarch64-linux-android` and `armv7-linux-androideabi` + `cargo-ndk`
- Gradle 8.7 via the committed wrapper (`./gradlew`); the system Gradle is 9.x, too new for AGP 8.5.2, so always use the wrapper

## Build the `.so`

From `rust/`:

```
cargo ndk -t arm64-v8a -o ../app/src/main/jniLibs build --release
cargo ndk -t armeabi-v7a -P 21 -o ../app/src/main/jniLibs build --release
```

Produces:

- `app/src/main/jniLibs/arm64-v8a/libpersonal_hopspot_android.so`
- `app/src/main/jniLibs/armeabi-v7a/libpersonal_hopspot_android.so`

The `armeabi-v7a` build uses API 21 because NDK r27 no longer ships API 19
native platform bits. The Rust bridge provides the one missing pre-21 loader
symbol needed by the MF97B Android 4.4.4 projector path.

## Build, install, and launch on a device

From this directory (`personal-hopspot/mobile/android`), with the phone plugged in and
USB debugging authorized (`adb devices` should list it):

```
./gradlew :app:installDebug
adb shell am start -n org.personal.hopspot/.DashboardActivity
```

`:app:installDebug` packages whatever `.so` is in `jniLibs/`, so rebuild the `.so`
first whenever the Rust changes. To install the prebuilt APK directly:

```
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

## Host-side checks (no NDK or device required)

From `rust/`:

```
cargo test
```
