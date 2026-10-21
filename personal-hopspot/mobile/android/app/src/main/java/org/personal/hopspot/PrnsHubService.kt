package org.personal.hopspot

import android.app.Service
import android.content.Intent
import android.os.Binder
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.os.RemoteCallbackList
import android.os.RemoteException
import android.util.Log
import java.io.IOException
import org.personal.hopspot.api.HubStatus
import org.personal.hopspot.api.IPrnsHub
import org.personal.hopspot.api.IPrnsHubListener
import org.personal.hopspot.api.PrnsHubContract
import org.personal.hopspot.api.SharedInstanceInfo

/**
 * The public hub interface any installed app may bind. It reads the process-wide engine that
 * [PrnsService] runs and never starts it: Android does not let a background bind start a
 * foreground service, so a client that finds the engine stopped asks the user to open the app.
 */
class PrnsHubService : Service() {
    private val listeners = RemoteCallbackList<IPrnsHubListener>()
    private val handler = Handler(Looper.getMainLooper())
    private var lastPushed: StatusFingerprint? = null

    private val pushTick = object : Runnable {
        override fun run() {
            pushStatusIfChanged()
            if (listeners.registeredCallbackCount > 0) {
                handler.postDelayed(this, PUSH_INTERVAL_MS)
            }
        }
    }

    private val binder = object : IPrnsHub.Stub() {
        override fun apiVersion(): Int = PrnsHubContract.API_VERSION

        override fun getStatus(): HubStatus = captureHubStatus()

        override fun getSharedInstanceInfo(): SharedInstanceInfo =
            SharedInstanceInfo().apply {
                localPort = PrnsService.LOCAL_RNS_PORT
                rpcPort = PrnsService.RPC_PORT
                rpcKeyHex = NativeBridge.nativeRpcKeyHex()
                reticulumConfig = NativeBridge.nativeSidebandJoinConfig()
            }

        override fun openLocalClient(clientLabel: String?): ParcelFileDescriptor? {
            if (NativeBridge.engineState() != PrnsEngineState.RUNNING) {
                return null
            }
            val uid = Binder.getCallingUid()
            val pair = try {
                ParcelFileDescriptor.createSocketPair()
            } catch (e: IOException) {
                Log.w(TAG, "socketpair for uid $uid failed", e)
                return null
            }
            val adopted = NativeBridge.nativeAdoptLocalClient(pair[0].detachFd(), uid)
            if (adopted != 0) {
                Log.w(TAG, "engine refused local client for uid $uid with code $adopted")
                pair[1].close()
                return null
            }
            Log.i(TAG, "local client opened for uid $uid (${clientLabel ?: "unlabelled"})")
            handler.post { pushStatusIfChanged() }
            return pair[1]
        }

        override fun registerListener(listener: IPrnsHubListener?) {
            if (listener == null || !listeners.register(listener)) {
                return
            }
            try {
                listener.onStatus(captureHubStatus())
            } catch (e: RemoteException) {
                listeners.unregister(listener)
                return
            }
            handler.post {
                handler.removeCallbacks(pushTick)
                handler.postDelayed(pushTick, PUSH_INTERVAL_MS)
            }
        }

        override fun unregisterListener(listener: IPrnsHubListener?) {
            if (listener != null) {
                listeners.unregister(listener)
            }
        }
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onDestroy() {
        handler.removeCallbacks(pushTick)
        listeners.kill()
        super.onDestroy()
    }

    private fun pushStatusIfChanged() {
        val status = captureHubStatus()
        val fingerprint = StatusFingerprint.of(status)
        if (fingerprint == lastPushed) {
            return
        }
        lastPushed = fingerprint
        val count = listeners.beginBroadcast()
        try {
            for (index in 0 until count) {
                try {
                    listeners.getBroadcastItem(index).onStatus(status)
                } catch (e: RemoteException) {
                    // RemoteCallbackList drops the dead listener on its own.
                }
            }
        } finally {
            listeners.finishBroadcast()
        }
    }

    /** The fields whose change is worth a push; byte counters move constantly and are excluded. */
    private data class StatusFingerprint(
        val state: String?,
        val lastFailureCode: Int,
        val interfaceCount: Int,
        val onlineInterfaceCount: Int,
        val localClientCount: Int,
        val routeCount: Int,
        val linkCount: Int,
    ) {
        companion object {
            fun of(status: HubStatus) = StatusFingerprint(
                status.state,
                status.lastFailureCode,
                status.interfaceCount,
                status.onlineInterfaceCount,
                status.localClientCount,
                status.routeCount,
                status.linkCount,
            )
        }
    }

    companion object {
        private const val TAG = "PrnsHubService"
        private const val PUSH_INTERVAL_MS = 1_000L
    }
}

internal fun captureHubStatus(): HubStatus {
    val engineState = NativeBridge.engineState()
    val engineFailure = NativeBridge.engineFailure()
    val health = NativeBridge.runtimeHealth()
    return HubStatus().apply {
        state = engineState.wireName
        running = engineState == PrnsEngineState.RUNNING
        lastFailureCode = engineFailure.code
        lastFailure = engineFailure.name
        nodeIdentityHashHex = NativeBridge.nativeNodeIdentityHashHex()
        deliveryDestinationHex = NativeBridge.nativeDeliveryDestinationHex()
        nodePageDestinationHex = NativeBridge.nativeNodePageDestinationHex()
        runtimeUptimeMs = health.runtimeUptimeMs
        interfaceCount = health.interfaceCount
        onlineInterfaceCount = health.onlineInterfaceCount
        localClientCount = health.localClientCount
        routeCount = health.routeCount
        linkCount = health.linkCount
        transportedLinkCount = health.transportedLinkCount
        rxBytes = health.rxBytes
        txBytes = health.txBytes
        rxBps = health.rxBps
        txBps = health.txBps
    }
}
