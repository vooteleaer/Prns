package org.personal.hopspot.client

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.os.RemoteException
import java.util.concurrent.CopyOnWriteArrayList
import org.personal.hopspot.api.HubStatus
import org.personal.hopspot.api.IPrnsHub
import org.personal.hopspot.api.IPrnsHubListener
import org.personal.hopspot.api.PrnsHubContract
import org.personal.hopspot.api.SharedInstanceInfo

/** The hub is not installed, not bound yet, or its process died mid-call. */
class PrnsHubUnavailableException(message: String, cause: Throwable? = null) :
    IllegalStateException(message, cause)

/**
 * A connection to the Personal Hopspot hub. Bind once, then read status, fetch shared-instance
 * credentials for a stock Reticulum stack, or open a Binder-carried local-client socket.
 *
 * Query methods are synchronous Binder calls: call them off the main thread. Every callback is
 * delivered on the main thread. Android rebinds automatically after the hub process restarts,
 * and status listeners are re-registered when it does.
 */
class PrnsHubClient(context: Context) : AutoCloseable {
    /** Connection lifecycle. [onDisconnected] fires when the hub process dies; a rebind follows. */
    interface ConnectionListener {
        fun onConnected(client: PrnsHubClient)

        fun onDisconnected(client: PrnsHubClient)
    }

    fun interface StatusListener {
        fun onStatus(status: HubStatus)
    }

    private val appContext = context.applicationContext
    private val mainHandler = Handler(Looper.getMainLooper())
    private val statusListeners = CopyOnWriteArrayList<StatusListener>()
    private var connectionListener: ConnectionListener? = null
    private var bound = false

    @Volatile
    private var hub: IPrnsHub? = null

    private val hubListener = object : IPrnsHubListener.Stub() {
        override fun onStatus(status: HubStatus) {
            mainHandler.post { statusListeners.forEach { it.onStatus(status) } }
        }
    }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            val connected = IPrnsHub.Stub.asInterface(service) ?: return
            hub = connected
            if (statusListeners.isNotEmpty()) {
                registerHubListener(connected)
            }
            connectionListener?.onConnected(this@PrnsHubClient)
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            hub = null
            connectionListener?.onDisconnected(this@PrnsHubClient)
        }
    }

    val isConnected: Boolean
        get() = hub != null

    /**
     * Bind the hub. Returns false when it is not installed (or not visible to this app). The
     * listener is called on the main thread once the connection is up.
     */
    fun bind(listener: ConnectionListener? = null): Boolean {
        check(!bound) { "PrnsHubClient is already bound" }
        connectionListener = listener
        bound = appContext.bindService(hubIntent(), connection, Context.BIND_AUTO_CREATE)
        if (!bound) {
            connectionListener = null
            appContext.unbindService(connection)
        }
        return bound
    }

    /** The hub's contract version; methods newer than this must not be called. */
    fun apiVersion(): Int = call { it.apiVersion() }

    fun status(): HubStatus = call { it.status }

    fun sharedInstanceInfo(): SharedInstanceInfo = call { it.sharedInstanceInfo }

    /**
     * Open a shared-instance connection carrying HDLC-framed Reticulum packets. Returns null when
     * the hub's engine is not running. The caller owns the descriptor; closing it disconnects.
     */
    fun openLocalClient(clientLabel: String): ParcelFileDescriptor? =
        call { it.openLocalClient(clientLabel) }

    /** Receive status pushes (on the main thread) whenever the hub's engine state changes. */
    fun addStatusListener(listener: StatusListener) {
        val first = statusListeners.isEmpty()
        statusListeners += listener
        if (first) {
            hub?.let { registerHubListener(it) }
        }
    }

    fun removeStatusListener(listener: StatusListener) {
        statusListeners -= listener
        if (statusListeners.isEmpty()) {
            hub?.let { unregisterHubListener(it) }
        }
    }

    override fun close() {
        hub?.let { unregisterHubListener(it) }
        statusListeners.clear()
        connectionListener = null
        hub = null
        if (bound) {
            bound = false
            appContext.unbindService(connection)
        }
    }

    private fun registerHubListener(connected: IPrnsHub) {
        try {
            connected.registerListener(hubListener)
        } catch (e: RemoteException) {
            // The hub died; onServiceDisconnected follows and the rebind registers again.
        }
    }

    private fun unregisterHubListener(connected: IPrnsHub) {
        try {
            connected.unregisterListener(hubListener)
        } catch (e: RemoteException) {
            // Nothing to undo in a dead hub.
        }
    }

    private inline fun <T> call(block: (IPrnsHub) -> T): T {
        val connected = hub ?: throw PrnsHubUnavailableException("the hub is not connected")
        return try {
            block(connected)
        } catch (e: RemoteException) {
            throw PrnsHubUnavailableException("the hub process went away", e)
        }
    }

    companion object {
        /** The explicit intent that binds the hub service. */
        @JvmStatic
        fun hubIntent(): Intent =
            Intent(PrnsHubContract.ACTION_BIND_HUB).setPackage(PrnsHubContract.HUB_PACKAGE)

        @JvmStatic
        fun isHubInstalled(context: Context): Boolean =
            context.packageManager.resolveService(hubIntent(), 0) != null

        /**
         * Opens the hub app so the user can start its engine. Android does not let a bind from
         * another app start the hub's foreground service, so this is the way to bring it up.
         */
        @JvmStatic
        fun launchHubIntent(context: Context): Intent? =
            context.packageManager.getLaunchIntentForPackage(PrnsHubContract.HUB_PACKAGE)
    }
}
