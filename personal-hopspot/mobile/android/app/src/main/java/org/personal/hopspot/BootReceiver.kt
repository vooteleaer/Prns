package org.personal.hopspot

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Starts the node when the phone starts, and again after an update, so the phone's shared
 * instance is up before the apps that join it. The receiver's high priority puts it ahead of
 * other apps' boot receivers, so the hub (not a stock Reticulum app) takes the loopback port.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED -> try {
                PrnsService.start(context)
            } catch (e: Exception) {
                Log.w("PrnsBoot", "could not start the node", e)
            }
        }
    }
}
