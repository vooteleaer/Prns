package org.personal.hopspot

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PersistableBundle
import android.text.InputType
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.Switch
import android.widget.TextView
import android.widget.Toast
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import org.json.JSONArray
import org.json.JSONObject

/**
 * The app's home screen: node and shared-instance status, the built-in radios, and the
 * interfaces configured in the node's Reticulum-style config file. Every native call runs on a
 * single background thread because applying an edit waits for the node to reload.
 */
class DashboardActivity : Activity() {
    private val main = Handler(Looper.getMainLooper())
    private val worker: ExecutorService = Executors.newSingleThreadExecutor()
    private var bound = false
    private var service: PrnsService? = null
    private var lastPermissionState: List<Boolean>? = null
    private var refreshQueued = false

    private lateinit var nodeStatus: TextView
    private lateinit var sharedInstanceStatus: TextView
    private lateinit var builtInList: LinearLayout
    private lateinit var configuredList: LinearLayout
    private lateinit var configuredMessage: TextView
    private var types: List<InterfaceType> = emptyList()

    private val serviceConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            service = (binder as? PrnsService.LocalBinder)?.service
            refresh()
        }

        override fun onServiceDisconnected(name: ComponentName) {
            service = null
        }
    }

    private val ticker = object : Runnable {
        override fun run() {
            refresh()
            main.postDelayed(this, REFRESH_INTERVAL_MILLIS)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(buildLayout())
        PrnsService.start(this)
        bound = bindService(
            Intent(this, PrnsService::class.java),
            serviceConnection,
            Context.BIND_AUTO_CREATE,
        )
        PrnsPermissions.requestMissing(this)
    }

    override fun onResume() {
        super.onResume()
        val current = PrnsPermissions.state(this)
        val changed = lastPermissionState?.let { it != current } == true
        lastPermissionState = current
        if (changed) {
            service?.refreshPlatformLinks()
        }
        main.post(ticker)
    }

    override fun onPause() {
        super.onPause()
        main.removeCallbacks(ticker)
    }

    override fun onDestroy() {
        main.removeCallbacks(ticker)
        if (bound) {
            unbindService(serviceConnection)
            bound = false
        }
        worker.shutdown()
        super.onDestroy()
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == PrnsPermissions.REQUEST_CODE) {
            lastPermissionState = PrnsPermissions.state(this)
            service?.refreshPlatformLinks()
        }
    }

    // region layout

    private fun buildLayout(): View {
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(16), dp(16), dp(24))
        }
        column.addView(text("Personal RNS", 24f, bold = true))
        nodeStatus = text("Starting…", 14f).also { column.addView(it) }

        column.addView(sectionHeader("Shared instance"))
        sharedInstanceStatus = text("", 14f).also { column.addView(it) }
        column.addView(
            row(
                button("Copy RNS config") { copySharedInstanceConfig() },
                button("Hopspot view") {
                    startActivity(Intent(this, MainActivity::class.java))
                },
            ),
        )

        column.addView(sectionHeader("Built-in interfaces"))
        builtInList = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        column.addView(builtInList)

        column.addView(
            row(
                sectionHeader("Configured interfaces").apply {
                    layoutParams = LinearLayout.LayoutParams(0, wrap, 1f)
                },
                button("Add") { showEditor(null) },
            ),
        )
        configuredMessage = text("", 14f).also { column.addView(it) }
        configuredList = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        column.addView(configuredList)

        return ScrollView(this).apply { addView(column) }
    }

    private val wrap = ViewGroup.LayoutParams.WRAP_CONTENT

    private fun dp(value: Int): Int =
        TypedValue.applyDimension(
            TypedValue.COMPLEX_UNIT_DIP,
            value.toFloat(),
            resources.displayMetrics,
        ).toInt()

    private fun text(value: String, size: Float, bold: Boolean = false): TextView =
        TextView(this).apply {
            text = value
            setTextSize(TypedValue.COMPLEX_UNIT_SP, size)
            if (bold) setTypeface(typeface, Typeface.BOLD)
        }

    private fun sectionHeader(value: String): TextView =
        text(value, 18f, bold = true).apply { setPadding(0, dp(20), 0, dp(6)) }

    private fun button(label: String, onClick: () -> Unit): Button =
        Button(this).apply {
            text = label
            setOnClickListener { onClick() }
        }

    private fun row(vararg views: View): LinearLayout =
        LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            views.forEach { addView(it) }
        }

    private fun listRow(
        title: String,
        subtitle: String,
        checked: Boolean?,
        onToggle: ((Boolean) -> Unit)?,
        onClick: (() -> Unit)? = null,
    ): View {
        val labels = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            layoutParams = LinearLayout.LayoutParams(0, wrap, 1f)
            addView(text(title, 16f))
            addView(text(subtitle, 13f).apply { alpha = 0.7f })
        }
        val container = row(labels).apply {
            setPadding(0, dp(8), 0, dp(8))
            minimumHeight = dp(56)
        }
        if (checked != null) {
            container.addView(
                Switch(this).apply {
                    isChecked = checked
                    isEnabled = onToggle != null
                    setOnCheckedChangeListener { _, value -> onToggle?.invoke(value) }
                },
            )
        }
        if (onClick != null) {
            container.isClickable = true
            container.setOnClickListener { onClick() }
        }
        return container
    }

    // endregion

    // region refresh

    private fun refresh() {
        if (refreshQueued) return
        refreshQueued = true
        worker.execute {
            val snapshot = runCatching {
                DashboardSnapshot(
                    state = NativeBridge.engineState(),
                    health = NativeBridge.runtimeHealth(),
                    builtIn = NativeBridge.nativeBuiltInInterfaces(),
                    configured = NativeBridge.nativeConfiguredInterfaces(),
                )
            }.getOrNull()
            main.post {
                refreshQueued = false
                if (snapshot != null && !isFinishing) render(snapshot)
            }
        }
    }

    private class DashboardSnapshot(
        val state: PrnsEngineState,
        val health: PrnsRuntimeHealth,
        val builtIn: String?,
        val configured: String?,
    )

    private fun render(snapshot: DashboardSnapshot) {
        val health = snapshot.health
        nodeStatus.text = when (snapshot.state) {
            PrnsEngineState.RUNNING ->
                "Running · ${health.onlineInterfaceCount}/${health.interfaceCount} interfaces online · " +
                    "${health.routeCount} routes · ${health.linkCount} links\n" +
                    "↓ ${formatBytes(health.rxBytes)} · ↑ ${formatBytes(health.txBytes)}"
            PrnsEngineState.STARTING -> "Starting…"
            PrnsEngineState.STOPPED -> "Stopped"
            PrnsEngineState.FAILED -> "Failed: ${NativeBridge.engineFailure().name}"
        }
        sharedInstanceStatus.text =
            "Other apps reach this node at 127.0.0.1:${PrnsService.LOCAL_RNS_PORT} " +
            "(RPC ${PrnsService.RPC_PORT}) or through the hub service.\n" +
            "${health.localClientCount} app connection(s)"

        renderBuiltIn(snapshot.builtIn)
        renderConfigured(snapshot.configured)
    }

    private fun renderBuiltIn(json: String?) {
        builtInList.removeAllViews()
        val interfaces = json?.let { runCatching { JSONArray(it) }.getOrNull() } ?: JSONArray()
        if (interfaces.length() == 0) {
            builtInList.addView(text("The node is not running.", 14f))
            return
        }
        for (index in 0 until interfaces.length()) {
            val entry = interfaces.getJSONObject(index)
            val id = entry.getString("id")
            val toggleable = entry.optBoolean("toggleable")
            val title = entry.optStringOrNull("name") ?: entry.optString("label")
            val subtitle = listOf(
                if (entry.optBoolean("online")) "online" else entry.optString("connection"),
                "↓ ${formatBytes(entry.optLong("rx_bytes"))}",
                "↑ ${formatBytes(entry.optLong("tx_bytes"))}",
            ).joinToString(" · ")
            builtInList.addView(
                listRow(
                    title = title,
                    subtitle = subtitle,
                    checked = if (toggleable) entry.optBoolean("enabled") else null,
                    onToggle = if (toggleable) {
                        { _ ->
                            worker.execute { NativeBridge.nativeToggleBuiltInInterface(id) }
                            main.postDelayed({ refresh() }, 300)
                        }
                    } else {
                        null
                    },
                ),
            )
        }
    }

    private fun renderConfigured(json: String?) {
        configuredList.removeAllViews()
        val document = json?.let { runCatching { JSONObject(it) }.getOrNull() }
        if (document == null) {
            configuredMessage.text = "Configured interfaces are unavailable."
            return
        }
        document.optStringOrNull("error")?.let {
            configuredMessage.text = it
            return
        }
        types = parseTypes(document.optJSONArray("types") ?: JSONArray())
        val interfaces = document.optJSONArray("interfaces") ?: JSONArray()
        configuredMessage.text = if (interfaces.length() == 0) {
            "None yet. Add a TCP, UDP, Backbone, I2P or WebSocket interface to reach " +
                "networks beyond the built-in radios."
        } else {
            "Stored in ${document.optString("path")}"
        }
        for (index in 0 until interfaces.length()) {
            val entry = ConfiguredInterface.from(interfaces.getJSONObject(index))
            configuredList.addView(
                listRow(
                    title = entry.name,
                    subtitle = "${entry.type.removeSuffix("Interface")} · ${entry.statusLine}",
                    checked = entry.enabled,
                    onToggle = { enabled ->
                        submit(
                            JSONObject()
                                .put("op", "set_enabled")
                                .put("name", entry.name)
                                .put("enabled", enabled),
                        )
                    },
                    onClick = { showEditor(entry) },
                ),
            )
        }
    }

    // endregion

    // region editing

    private fun submit(request: JSONObject, onSuccess: () -> Unit = {}) {
        worker.execute {
            val error = NativeBridge.nativeApplyInterfaceEdit(request.toString())
            main.post {
                if (error == null) {
                    onSuccess()
                } else {
                    Toast.makeText(this, error, Toast.LENGTH_LONG).show()
                }
                refresh()
            }
        }
    }

    private fun showEditor(existing: ConfiguredInterface?) {
        if (types.isEmpty()) {
            Toast.makeText(this, "The node is still starting", Toast.LENGTH_SHORT).show()
            return
        }
        val form = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(8), dp(20), dp(8))
        }
        val nameField = EditText(this).apply {
            hint = "Name"
            setSingleLine()
            setText(existing?.name ?: "")
        }
        form.addView(nameField)
        val typeSpinner = Spinner(this).apply {
            adapter = ArrayAdapter(
                this@DashboardActivity,
                android.R.layout.simple_spinner_dropdown_item,
                types.map { it.name.removeSuffix("Interface") },
            )
            types.indexOfFirst { it.name == existing?.type }.takeIf { it >= 0 }?.let(::setSelection)
        }
        form.addView(typeSpinner)
        val enabledBox = CheckBox(this).apply {
            text = "Enabled"
            isChecked = existing?.enabled ?: true
        }
        form.addView(enabledBox)
        val fieldsContainer = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        form.addView(fieldsContainer)
        val fields = mutableMapOf<String, EditText>()

        fun showFields(type: InterfaceType) {
            fieldsContainer.removeAllViews()
            fields.clear()
            val values = if (existing?.type == type.name) existing.settings else emptyMap()
            for (setting in type.settings) {
                fieldsContainer.addView(
                    text(setting.label + if (setting.required) " *" else "", 14f).apply {
                        setPadding(0, dp(10), 0, 0)
                    },
                )
                val field = EditText(this).apply {
                    setSingleLine()
                    hint = setting.hint
                    inputType = when {
                        setting.secret ->
                            InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
                        setting.input == "port" || setting.input == "unsigned" ->
                            InputType.TYPE_CLASS_NUMBER
                        else -> InputType.TYPE_CLASS_TEXT
                    }
                    setText(values[setting.key] ?: "")
                }
                fieldsContainer.addView(field)
                fieldsContainer.addView(text(setting.description, 12f).apply { alpha = 0.6f })
                fields[setting.key] = field
            }
        }

        typeSpinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                showFields(types[position])
            }

            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
        showFields(types[typeSpinner.selectedItemPosition.coerceAtLeast(0)])

        val builder = AlertDialog.Builder(this)
            .setTitle(if (existing == null) "Add interface" else "Edit ${existing.name}")
            .setView(ScrollView(this).apply { addView(form) })
            .setPositiveButton("Save", null)
            .setNegativeButton("Cancel", null)
        if (existing != null) {
            builder.setNeutralButton("Remove", null)
        }
        val dialog = builder.show()
        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
            val type = types[typeSpinner.selectedItemPosition]
            // Settings the form does not show (advanced ones set by hand in the file) survive
            // an edit as long as the type stays the same.
            val settings = JSONObject()
            if (existing?.type == type.name) {
                existing.settings.forEach { (key, value) -> settings.put(key, value) }
            }
            fields.forEach { (key, field) -> settings.put(key, field.text.toString()) }
            val request = JSONObject()
                .put("op", "upsert")
                .put("original", existing?.name ?: JSONObject.NULL)
                .put("name", nameField.text.toString())
                .put("type", type.name)
                .put("enabled", enabledBox.isChecked)
                .put("settings", settings)
            submit(request) { dialog.dismiss() }
        }
        if (existing != null) {
            dialog.getButton(AlertDialog.BUTTON_NEUTRAL).setOnClickListener {
                AlertDialog.Builder(this)
                    .setMessage("Remove ${existing.name}?")
                    .setPositiveButton("Remove") { _, _ ->
                        submit(JSONObject().put("op", "remove").put("name", existing.name)) {
                            dialog.dismiss()
                        }
                    }
                    .setNegativeButton("Cancel", null)
                    .show()
            }
        }
    }

    private fun copySharedInstanceConfig() {
        val config = NativeBridge.nativeSidebandJoinConfig()
        if (config.isNullOrBlank()) {
            Toast.makeText(this, "The node is not running", Toast.LENGTH_SHORT).show()
            return
        }
        val clip = ClipData.newPlainText("RNS config", config)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            clip.description.extras = PersistableBundle().apply {
                putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
            }
        }
        (getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(clip)
        Toast.makeText(this, "RNS config copied", Toast.LENGTH_SHORT).show()
    }

    // endregion

    private class InterfaceType(val name: String, val settings: List<SettingField>)

    private class SettingField(
        val key: String,
        val label: String,
        val description: String,
        val hint: String,
        val input: String,
        val required: Boolean,
        val secret: Boolean,
    )

    private class ConfiguredInterface(
        val name: String,
        val type: String,
        val enabled: Boolean,
        val settings: Map<String, String>,
        val statusLine: String,
    ) {
        companion object {
            fun from(json: JSONObject): ConfiguredInterface {
                val settingsJson = json.optJSONObject("settings") ?: JSONObject()
                val settings = settingsJson.keys().asSequence()
                    .associateWith { settingsJson.getString(it) }
                val status = json.optJSONObject("status") ?: JSONObject()
                val statusLine = when (status.optString("state")) {
                    "up" -> buildString {
                        append(if (status.optBoolean("online")) "online" else status.optString("connection", "connecting"))
                        val peers = status.optInt("peers")
                        if (peers > 0) append(" · $peers peer(s)")
                        append(" · ↓ ${formatBytes(status.optLong("rx_bytes"))}")
                        append(" · ↑ ${formatBytes(status.optLong("tx_bytes"))}")
                        status.optStringOrNull("detail")?.let { append(" · $it") }
                    }
                    "failed" -> "failed: ${status.optString("error")}"
                    "disabled" -> "disabled"
                    "pending" -> "starting"
                    else -> "node stopped"
                }
                return ConfiguredInterface(
                    name = json.getString("name"),
                    type = json.optString("type"),
                    enabled = json.optBoolean("enabled", true),
                    settings = settings,
                    statusLine = statusLine,
                )
            }
        }
    }

    private fun parseTypes(json: JSONArray): List<InterfaceType> =
        (0 until json.length()).map { index ->
            val type = json.getJSONObject(index)
            val settings = type.optJSONArray("settings") ?: JSONArray()
            InterfaceType(
                name = type.getString("name"),
                settings = (0 until settings.length()).map { settingIndex ->
                    val setting = settings.getJSONObject(settingIndex)
                    SettingField(
                        key = setting.getString("key"),
                        label = setting.optString("label"),
                        description = setting.optString("description"),
                        hint = setting.optString("hint"),
                        input = setting.optString("input"),
                        required = setting.optBoolean("required"),
                        secret = setting.optBoolean("secret"),
                    )
                },
            )
        }

    private companion object {
        const val REFRESH_INTERVAL_MILLIS = 2_000L
    }
}

private fun JSONObject.optStringOrNull(key: String): String? =
    if (isNull(key)) null else optString(key).takeIf { it.isNotEmpty() }

private fun formatBytes(bytes: Long): String =
    when {
        bytes >= 1_000_000_000 -> String.format("%.1f GB", bytes / 1e9)
        bytes >= 1_000_000 -> String.format("%.1f MB", bytes / 1e6)
        bytes >= 1_000 -> String.format("%.1f kB", bytes / 1e3)
        else -> "$bytes B"
    }
