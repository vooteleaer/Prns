# Personal Hopspot hub client

A small Android library for apps that want to use the Personal Hopspot app as their
Reticulum hub: one engine and one set of radios, shared across the phone. It wraps the
AIDL contract in [`../api`](../api), has no dependencies beyond the Android
framework, and supports `minSdk 19`.

> **Preview.** The library builds from this repository and has no published package
> yet. Include it as a Gradle project or copy the built AAR.

The hub is open to every installed app, the same way a desktop Reticulum shared
instance is open to every local process. Any app that binds it can read its status,
receive its RPC key, and route traffic through it.

## Binding

```kotlin
val hub = PrnsHubClient(context)
val installed = hub.bind(object : PrnsHubClient.ConnectionListener {
    override fun onConnected(client: PrnsHubClient) {
        client.addStatusListener { status -> showState(status.state, status.localClientCount) }
    }
    override fun onDisconnected(client: PrnsHubClient) = Unit // Android rebinds by itself
})
if (!installed) {
    // Personal Hopspot is not installed.
}
// ...
hub.close()
```

`status()`, `sharedInstanceInfo()`, `openLocalClient()` and `apiVersion()` are
synchronous Binder calls, so call them off the main thread. Callbacks arrive on the
main thread. When the hub is gone, every query throws `PrnsHubUnavailableException`.

The hub does not start its engine for you, because Android does not let a background
bind start a foreground service. If `status().running` is false, send the user to
the hub app:

```kotlin
PrnsHubClient.launchHubIntent(context)?.let(context::startActivity)
```

The library's manifest declares the `<queries>` entry that Android 11+ needs to see
the hub package. It is merged into your app automatically.

## Carrying traffic

`openLocalClient(label)` returns a `ParcelFileDescriptor` for one end of a
socketpair. The hub attaches the other end as a shared-instance local client. The
stream carries HDLC-framed Reticulum packets, the same wire as the loopback TCP
listener and as a Reticulum `PipeInterface`. Closing the descriptor disconnects the
client.

With the Prns Kotlin/Java SDK (`prns-host/bindings/jvm`), attach it as a supplied
Pipe. The SDK reopens it after the respawn delay whenever the hub restarts:

```kotlin
host.beginSuppliedPipe(
    name = "personal-hopspot-hub",
    respawnDelayMillis = 2_000,
    bitrate = BitrateAuto,
).use { pipe ->
    pipe.serve {
        withContext(Dispatchers.IO) {
            runCatching { hub.openLocalClient("my-app") }.getOrNull()?.detachFd()
                ?: SUPPLIED_PIPE_DECLINED
        }
    }
}
```

The SDK does not ship an Android package yet, so this snippet is not compiled in this
repository. See [the JVM SDK README](../../../../prns-host/bindings/jvm/README.md)
for its Android setup.

## Stock Reticulum stacks

A Python RNS or Sideband instance can join over loopback TCP instead.
`sharedInstanceInfo()` returns the ports, the RPC key, and a complete `reticulumConfig`
to write as that instance's config.

## Contract evolution

Methods in `IPrnsHub` and fields in `HubStatus` and `SharedInstanceInfo` are only
ever appended. Check `apiVersion()` against the version that introduced a method
before calling it. `PrnsHubContract.API_VERSION` is the version this library was
built against.
