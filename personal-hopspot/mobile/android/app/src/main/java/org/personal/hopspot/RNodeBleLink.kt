package org.personal.hopspot

import android.Manifest
import android.annotation.SuppressLint
import android.annotation.TargetApi
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import android.util.Log
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit

/**
 * Carries `ble://` RNode interfaces over Android's GATT stack, which native code cannot reach.
 * The engine queues a connect request per RNode interface (see `rust/src/rnode_ble.rs`); this
 * link finds the RNode, subscribes to its Nordic UART TX characteristic, and shuttles bytes
 * until either side closes, after which the engine's RNode interface reconnects on its own.
 */
@TargetApi(Build.VERSION_CODES.M)
@SuppressLint("MissingPermission")
class RNodeBleLink(private val context: Context) {
    @Volatile
    private var running = false
    private var dispatcher: Thread? = null
    private val sessions = ConcurrentHashMap<Int, Session>()

    fun start() {
        if (running) return
        running = true
        dispatcher = Thread({ dispatch() }, "rnode-ble").also { it.start() }
    }

    fun stop() {
        running = false
        dispatcher?.interrupt()
        dispatcher = null
        sessions.values.forEach { it.finish("the app stopped the Bluetooth link") }
        sessions.clear()
    }

    private fun dispatch() {
        while (running) {
            val request = NativeBridge.nativeRNodeBleNextRequest(REQUEST_POLL_MILLIS) ?: continue
            val separator = request.indexOf('\t')
            val id = request.substring(0, separator).toIntOrNull() ?: continue
            val target = request.substring(separator + 1)
            Thread({ open(id, target) }, "rnode-ble-$id").start()
        }
    }

    private fun open(id: Int, target: String) {
        val failure = try {
            openOrExplain(id, target)
        } catch (error: SecurityException) {
            "Bluetooth permission was denied; allow Nearby devices for Personal RNS"
        } catch (error: RuntimeException) {
            Log.w(TAG, "RNode BLE session $id failed", error)
            error.message ?: error.javaClass.simpleName
        }
        if (failure != null) {
            Log.w(TAG, "RNode BLE session $id: $failure")
            NativeBridge.nativeRNodeBleOpened(id, failure)
        }
    }

    /** Starts the GATT session, or returns why it cannot start. */
    private fun openOrExplain(id: Int, target: String): String? {
        if (!hasPermission(CONNECT_PERMISSION)) {
            return "Bluetooth permission was denied; allow Nearby devices for Personal RNS"
        }
        val adapter = (context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager)
            ?.adapter
            ?: return "this device has no Bluetooth adapter"
        if (!adapter.isEnabled) {
            return "Bluetooth is off"
        }
        val device = findDevice(adapter, target)
            ?: return "could not find ${describe(target)}; pair it in Android Bluetooth settings or bring it into range"
        Log.i(TAG, "RNode BLE session $id connecting to ${device.address}")
        val session = Session(id, device)
        sessions[id] = session
        session.connect()
        return null
    }

    private fun describe(target: String): String =
        when {
            target.startsWith("addr:") -> "RNode ${target.removePrefix("addr:")}"
            target.startsWith("name:") -> "\"${target.removePrefix("name:")}\""
            else -> "a paired RNode"
        }

    private fun matches(target: String, device: BluetoothDevice, advertisedName: String?): Boolean {
        val name = advertisedName ?: device.name
        return when {
            target.startsWith("addr:") ->
                device.address.equals(target.removePrefix("addr:"), ignoreCase = true)
            target.startsWith("name:") -> name == target.removePrefix("name:")
            else -> name?.startsWith("RNode ") == true
        }
    }

    private fun findDevice(adapter: BluetoothAdapter, target: String): BluetoothDevice? {
        if (target.startsWith("addr:")) {
            val address = target.removePrefix("addr:").uppercase()
            if (BluetoothAdapter.checkBluetoothAddress(address)) {
                return adapter.getRemoteDevice(address)
            }
        }
        adapter.bondedDevices.orEmpty()
            .firstOrNull { matches(target, it, null) }
            ?.let { return it }
        return scan(adapter, target)
    }

    private fun scan(adapter: BluetoothAdapter, target: String): BluetoothDevice? {
        if (!hasPermission(SCAN_PERMISSION)) return null
        val scanner = adapter.bluetoothLeScanner ?: return null
        val found = CountDownLatch(1)
        var match: BluetoothDevice? = null
        val callback = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                if (match == null && matches(target, result.device, result.scanRecord?.deviceName)) {
                    match = result.device
                    found.countDown()
                }
            }
        }
        val filters = listOf(ScanFilter.Builder().setServiceUuid(ParcelUuid(NUS_SERVICE)).build())
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .build()
        scanner.startScan(filters, settings, callback)
        try {
            found.await(SCAN_TIMEOUT_SECONDS, TimeUnit.SECONDS)
        } finally {
            runCatching { scanner.stopScan(callback) }
        }
        return match
    }

    private fun hasPermission(permission: String?): Boolean =
        permission == null ||
            context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private inner class Session(
        private val id: Int,
        private val device: BluetoothDevice,
    ) : BluetoothGattCallback() {
        @Volatile private var gatt: BluetoothGatt? = null
        @Volatile private var opened = false
        @Volatile private var finished = false
        @Volatile private var mtu = DEFAULT_MTU
        private var rx: BluetoothGattCharacteristic? = null
        private val writeDone = Semaphore(0)

        fun connect() {
            gatt = device.connectGatt(context, false, this, BluetoothDevice.TRANSPORT_LE)
            Thread({
                Thread.sleep(OPEN_TIMEOUT_MILLIS)
                if (!opened) finish("the RNode did not finish connecting; pair it and keep it in range")
            }, "rnode-ble-$id-timeout").apply { isDaemon = true }.start()
        }

        override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
            when (newState) {
                BluetoothProfile.STATE_CONNECTED -> {
                    if (!gatt.requestMtu(REQUESTED_MTU)) gatt.discoverServices()
                }
                BluetoothProfile.STATE_DISCONNECTED -> finish("the RNode disconnected (status $status)")
            }
        }

        override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS) this.mtu = mtu
            gatt.discoverServices()
        }

        @Suppress("DEPRECATION")
        override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
            val service = gatt.getService(NUS_SERVICE)
            val rx = service?.getCharacteristic(NUS_RX)
            val tx = service?.getCharacteristic(NUS_TX)
            if (status != BluetoothGatt.GATT_SUCCESS || rx == null || tx == null) {
                finish("${device.address} has no Nordic UART service; is it an RNode with Bluetooth enabled?")
                return
            }
            this.rx = rx
            gatt.setCharacteristicNotification(tx, true)
            val descriptor = tx.getDescriptor(CCCD)
            if (descriptor == null) {
                markOpened()
                return
            }
            descriptor.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            if (!gatt.writeDescriptor(descriptor)) {
                finish("could not subscribe to the RNode's notifications")
            }
        }

        override fun onDescriptorWrite(
            gatt: BluetoothGatt,
            descriptor: BluetoothGattDescriptor,
            status: Int,
        ) {
            if (status == BluetoothGatt.GATT_SUCCESS) {
                markOpened()
            } else {
                finish("the RNode refused the subscription (status $status); pair it in Android Bluetooth settings")
            }
        }

        @Deprecated("Deprecated in API 33")
        @Suppress("DEPRECATION")
        override fun onCharacteristicChanged(
            gatt: BluetoothGatt,
            characteristic: BluetoothGattCharacteristic,
        ) {
            if (characteristic.uuid == NUS_TX) {
                characteristic.value?.let { NativeBridge.nativeRNodeBleInbound(id, it) }
            }
        }

        override fun onCharacteristicChanged(
            gatt: BluetoothGatt,
            characteristic: BluetoothGattCharacteristic,
            value: ByteArray,
        ) {
            if (characteristic.uuid == NUS_TX) {
                NativeBridge.nativeRNodeBleInbound(id, value)
            }
        }

        override fun onCharacteristicWrite(
            gatt: BluetoothGatt,
            characteristic: BluetoothGattCharacteristic,
            status: Int,
        ) {
            writeDone.release()
        }

        private fun markOpened() {
            if (opened || finished) return
            opened = true
            Log.i(TAG, "RNode BLE session $id open (MTU $mtu)")
            NativeBridge.nativeRNodeBleOpened(id, null)
            Thread({ pump() }, "rnode-ble-$id-tx").start()
        }

        /** Writes the engine's outbound bytes to the RNode's RX characteristic, one at a time. */
        @Suppress("DEPRECATION")
        private fun pump() {
            val chunk = (mtu - ATT_WRITE_OVERHEAD).coerceAtLeast(MINIMUM_CHUNK)
            while (!finished) {
                val bytes = NativeBridge.nativeRNodeBleTakeOutbound(id, OUTBOUND_POLL_MILLIS)
                    ?: break
                var offset = 0
                while (offset < bytes.size && !finished) {
                    val end = minOf(offset + chunk, bytes.size)
                    val characteristic = rx ?: break
                    val current = gatt ?: break
                    characteristic.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
                    characteristic.value = bytes.copyOfRange(offset, end)
                    if (current.writeCharacteristic(characteristic)) {
                        writeDone.tryAcquire(WRITE_TIMEOUT_MILLIS, TimeUnit.MILLISECONDS)
                        offset = end
                    } else {
                        Thread.sleep(WRITE_RETRY_MILLIS)
                    }
                }
            }
            finish("the RNode interface closed")
        }

        fun finish(reason: String) {
            if (finished) return
            finished = true
            sessions.remove(id)
            if (opened) {
                Log.i(TAG, "RNode BLE session $id closed: $reason")
                NativeBridge.nativeRNodeBleDisconnected(id)
            } else {
                Log.w(TAG, "RNode BLE session $id failed: $reason")
                NativeBridge.nativeRNodeBleOpened(id, reason)
            }
            runCatching { gatt?.disconnect() }
            runCatching { gatt?.close() }
            gatt = null
        }
    }

    private companion object {
        const val TAG = "RNodeBleLink"
        val NUS_SERVICE: UUID = UUID.fromString("6e400001-b5a3-f393-e0a9-e50e24dcca9e")
        val NUS_RX: UUID = UUID.fromString("6e400002-b5a3-f393-e0a9-e50e24dcca9e")
        val NUS_TX: UUID = UUID.fromString("6e400003-b5a3-f393-e0a9-e50e24dcca9e")
        val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
        const val REQUEST_POLL_MILLIS = 1_000L
        const val OUTBOUND_POLL_MILLIS = 500L
        const val OPEN_TIMEOUT_MILLIS = 25_000L
        const val SCAN_TIMEOUT_SECONDS = 10L
        const val WRITE_TIMEOUT_MILLIS = 2_000L
        const val WRITE_RETRY_MILLIS = 10L
        const val DEFAULT_MTU = 23
        const val REQUESTED_MTU = 512
        const val ATT_WRITE_OVERHEAD = 3
        const val MINIMUM_CHUNK = 20
        val CONNECT_PERMISSION: String? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) Manifest.permission.BLUETOOTH_CONNECT else null
        val SCAN_PERMISSION: String? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                Manifest.permission.BLUETOOTH_SCAN
            } else {
                Manifest.permission.ACCESS_FINE_LOCATION
            }
    }
}
