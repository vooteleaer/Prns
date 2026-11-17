package org.personal.hopspot

import android.content.Context
import android.util.Log
import java.io.File
import java.io.RandomAccessFile
import java.util.Date

/**
 * Keeps this process's log (Kotlin and the native node, which logs as HopspotRust) in
 * files/hopspot.log, so a failure can be read and shared without adb. A `logcat` child
 * follows this process only; the file is rotated to hopspot.log.1 above [MAX_BYTES].
 */
object HopspotLog {
    private const val TAG = "HopspotLog"
    private const val MAX_BYTES = 2_000_000L
    private const val SHARE_CHARS = 150_000
    private var logcat: Process? = null

    private fun current(context: Context) = File(context.filesDir, "hopspot.log")
    private fun previous(context: Context) = File(context.filesDir, "hopspot.log.1")

    @Synchronized
    fun start(context: Context) {
        if (logcat?.let { running(it) } == true) return
        try {
            val file = current(context)
            if (file.length() > MAX_BYTES) {
                previous(context).delete()
                file.renameTo(previous(context))
            }
            file.appendText("=== ${Date()} pid ${android.os.Process.myPid()}\n")
            logcat = Runtime.getRuntime().exec(
                arrayOf(
                    "logcat", "-v", "time", "-f", file.absolutePath,
                    "--pid=${android.os.Process.myPid()}",
                ),
            )
        } catch (error: Exception) {
            Log.e(TAG, "cannot keep a log file", error)
        }
    }

    private fun running(process: Process): Boolean =
        try {
            process.exitValue()
            false
        } catch (_: IllegalThreadStateException) {
            true
        }

    /** The newest part of the log (both files), small enough for a share intent. */
    fun tail(context: Context): String {
        val text = StringBuilder()
        for (file in listOf(previous(context), current(context))) {
            if (!file.exists()) continue
            RandomAccessFile(file, "r").use { raf ->
                val start = maxOf(0L, raf.length() - SHARE_CHARS)
                raf.seek(start)
                val bytes = ByteArray((raf.length() - start).toInt())
                raf.readFully(bytes)
                text.append(String(bytes, Charsets.UTF_8))
            }
        }
        return if (text.length > SHARE_CHARS) text.substring(text.length - SHARE_CHARS) else text.toString()
    }
}
