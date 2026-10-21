package org.personal.hopspot

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build

/** The runtime permissions the node's radios need, shared by every activity that starts it. */
internal object PrnsPermissions {
    const val REQUEST_CODE = 1

    fun runtimePermissions(): List<String> {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) {
            return emptyList()
        }
        val permissions = mutableListOf<String>()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            permissions += Manifest.permission.POST_NOTIFICATIONS
            permissions += Manifest.permission.NEARBY_WIFI_DEVICES
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            permissions += listOf(
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_ADVERTISE,
                Manifest.permission.BLUETOOTH_CONNECT,
            )
        }
        if (Build.VERSION.SDK_INT <= Build.VERSION_CODES.S_V2) {
            permissions += listOf(
                Manifest.permission.ACCESS_COARSE_LOCATION,
                Manifest.permission.ACCESS_FINE_LOCATION,
            )
        }
        return permissions
    }

    fun requestMissing(activity: Activity) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) {
            return
        }
        val needed = runtimePermissions().filter { permission ->
            activity.checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED
        }
        if (needed.isNotEmpty()) {
            activity.requestPermissions(needed.toTypedArray(), REQUEST_CODE)
        }
    }

    fun state(activity: Activity): List<Boolean> {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) {
            return emptyList()
        }
        return runtimePermissions().map {
            activity.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED
        }
    }
}
