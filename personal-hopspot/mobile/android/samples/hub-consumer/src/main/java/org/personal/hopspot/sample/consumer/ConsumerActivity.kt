package org.personal.hopspot.sample.consumer

import android.app.Activity
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import org.personal.hopspot.api.HubStatus
import org.personal.hopspot.client.PrnsHubClient
import org.personal.hopspot.client.PrnsHubUnavailableException

/**
 * A third-party app's view of the hub: bind, watch status, and hold a Binder-carried
 * shared-instance connection open. A real app hands the descriptor to its Reticulum stack.
 */
class ConsumerActivity : Activity() {
    private val worker: ExecutorService = Executors.newSingleThreadExecutor()
    private lateinit var hub: PrnsHubClient
    private lateinit var statusView: TextView
    private lateinit var connectionView: TextView
    private var localClient: ParcelFileDescriptor? = null

    private val statusListener = PrnsHubClient.StatusListener { render(it) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        statusView = TextView(this)
        connectionView = TextView(this)
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 48, 48, 48)
            addView(statusView)
            addView(connectionView)
            addView(button("Open local client") { openLocalClient() })
            addView(button("Close local client") { closeLocalClient() })
            addView(button("Open hub app") { openHubApp() })
        }
        setContentView(layout)

        hub = PrnsHubClient(this)
        if (!hub.bind(object : PrnsHubClient.ConnectionListener {
                override fun onConnected(client: PrnsHubClient) {
                    client.addStatusListener(statusListener)
                }

                override fun onDisconnected(client: PrnsHubClient) {
                    statusView.text = "Hub disconnected; waiting for it to come back"
                }
            })
        ) {
            statusView.text = "Personal Hopspot is not installed"
        }
    }

    override fun onDestroy() {
        closeLocalClient()
        hub.close()
        worker.shutdown()
        super.onDestroy()
    }

    private fun render(status: HubStatus) {
        statusView.text = buildString {
            appendLine("Engine: ${status.state}")
            appendLine("Node: ${status.nodeIdentityHashHex ?: "-"}")
            appendLine("Interfaces online: ${status.onlineInterfaceCount}/${status.interfaceCount}")
            appendLine("Local clients: ${status.localClientCount}")
            appendLine("Routes: ${status.routeCount}")
        }
    }

    private fun openLocalClient() {
        if (localClient != null) {
            return
        }
        worker.execute {
            val message = try {
                val descriptor = hub.openLocalClient("hub-consumer sample")
                runOnUiThread { localClient = descriptor }
                if (descriptor == null) {
                    "Hub engine is not running; open the hub app first"
                } else {
                    val info = hub.sharedInstanceInfo()
                    "Local client open (TCP shared instance also on 127.0.0.1:${info.localPort})"
                }
            } catch (e: PrnsHubUnavailableException) {
                "Hub unavailable: ${e.message}"
            }
            runOnUiThread { connectionView.text = message }
        }
    }

    private fun closeLocalClient() {
        localClient?.close()
        localClient = null
        connectionView.text = "Local client closed"
    }

    private fun openHubApp() {
        PrnsHubClient.launchHubIntent(this)?.let(::startActivity)
    }

    private fun button(label: String, onClick: () -> Unit) =
        Button(this).apply {
            text = label
            setOnClickListener { onClick() }
        }
}
