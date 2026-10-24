package org.tetherust.companion

import android.app.*
import android.content.*
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.*
import android.view.*
import android.widget.*
import java.util.Locale

class MainActivity : Activity() {
    private val handler = Handler(Looper.getMainLooper())
    private val ink = Color.rgb(231, 239, 236)
    private val muted = Color.rgb(144, 163, 157)
    private val mint = Color.rgb(156, 243, 203)
    private val canvas = Color.rgb(15, 20, 20)
    private val surface = Color.rgb(24, 32, 31)
    private val border = Color.rgb(46, 61, 57)
    private lateinit var state: TextView
    private lateinit var badge: TextView
    private lateinit var vpn: TextView
    private lateinit var download: TextView
    private lateinit var upload: TextView
    private lateinit var streams: TextView
    private lateinit var action: Button
    private lateinit var method: Spinner
    private lateinit var wifi: Switch
    private lateinit var proxyPort: EditText
    private val refresh = object : Runnable {
        override fun run() {
            updateState()
            handler.postDelayed(this, 1000)
        }
    }
    private fun dp(value: Int) = (value * resources.displayMetrics.density + .5f).toInt()
    private fun shape(color: Int, radius: Int = 20, stroke: Int? = null) = GradientDrawable().apply {
        setColor(color); cornerRadius = dp(radius).toFloat()
        if (stroke != null) setStroke(dp(1), stroke)
    }
    private fun text(value: String, size: Float, color: Int = ink, bold: Boolean = false) = TextView(this).apply {
        text = value; textSize = size; setTextColor(color)
        if (bold) setTypeface(null, Typeface.BOLD)
        setLineSpacing(dp(3).toFloat(), 1f)
    }
    private fun column() = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
    private fun LinearLayout.item(view: View, top: Int = 0) {
        addView(view, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(top) })
    }
    private fun card(color: Int = surface) = column().apply {
        background = shape(color, 24, border); setPadding(dp(20), dp(20), dp(20), dp(20))
    }
    private fun button(label: String, primary: Boolean = false) = Button(this).apply {
        text = label; isAllCaps = false; textSize = 16f
        setTypeface(null, Typeface.BOLD); setTextColor(if (primary) canvas else ink)
        minHeight = dp(56); minimumHeight = dp(56); minWidth = 0; minimumWidth = 0
        setPadding(dp(16), dp(12), dp(16), dp(12))
        background = RippleDrawable(ColorStateList.valueOf(Color.argb(35, 255, 255, 255)),
            shape(if (primary) mint else surface, 16, if (primary) null else border), null)
    }
    private fun bytes(value: Long): String {
        val units = arrayOf("B", "KB", "MB", "GB", "TB")
        var amount = value.coerceAtLeast(0).toDouble(); var unit = 0
        while (amount >= 1024 && unit < units.lastIndex) { amount /= 1024; unit++ }
        return if (unit == 0) "${amount.toLong()} B" else String.format(Locale.getDefault(), "%.1f %s", amount, units[unit])
    }
    private fun updateState() {
        val running = SharingState.running
        badge.text = if (running) "●  SHARING IS ON" else "○  READY WHEN YOU ARE"
        badge.setTextColor(if (running) mint else muted)
        state.text = SharingState.status
        vpn.text = SharingState.vpn
        download.text = bytes(SharingState.received.get())
        upload.text = bytes(SharingState.sent.get())
        streams.text = if (SharingState.connections > 0) "${SharingState.connections} authenticated streams" else if (running) "Waiting for your laptop" else "USB · encrypted companion link"
        action.text = if (running) "Stop sharing" else "Start sharing"
        method.isEnabled = !running; wifi.isEnabled = !running; proxyPort.isEnabled = !running
        method.alpha = if (running) .55f else 1f
        wifi.alpha = if (running) .55f else 1f
        proxyPort.alpha = if (running) .55f else 1f
    }
    override fun onCreate(saved: Bundle?) {
        super.onCreate(saved)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        window.statusBarColor = canvas; window.navigationBarColor = canvas
        val layout = column().apply {
            setPadding(dp(20), dp(24), dp(20), dp(24))
        }
        val scroll = ScrollView(this).apply {
            setBackgroundColor(canvas); isFillViewport = true; clipToPadding = false
            addView(layout); isVerticalScrollBarEnabled = false
        }
        // Android 15 enforces edge-to-edge: keep controls clear of bars and cutouts.
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false)
            scroll.setOnApplyWindowInsetsListener { view, insets ->
                val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
                insets
            }
        }
        val brand = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL }
        brand.addView(text("T", 24f, canvas, true).apply {
            gravity = Gravity.CENTER; background = shape(mint, 13)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }, LinearLayout.LayoutParams(dp(44), dp(44)))
        brand.addView(column().apply {
            item(text("tetherust", 23f, ink, true))
            item(text("PHONE TO DESKTOP", 10f, muted).apply { letterSpacing = .12f }, 2)
        }, LinearLayout.LayoutParams(-1, -2).apply { marginStart = dp(12) })
        layout.item(brand)
        layout.item(text("Your internet.\nA little further.", 34f, ink, true).apply { setLineSpacing(0f, 1.03f) }, 28)
        layout.item(text("Bring your phone’s connection to your laptop.", 15f, muted), 8)

        val hero = card(Color.rgb(27, 44, 37))
        badge = text("", 11f, mint, true).apply { letterSpacing = .08f }; hero.item(badge)
        state = text("Stopped", 21f, ink, true).apply { accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE }; hero.item(state, 16)
        vpn = text("", 13f, muted); hero.item(vpn, 8)
        action = button("Start sharing", true); hero.item(action, 24)
        streams = text("", 12f, muted).apply { gravity = Gravity.CENTER }; hero.item(streams, 12)
        layout.item(hero, 24)

        val metrics = LinearLayout(this)
        fun metric(label: String, arrow: String): Pair<LinearLayout, TextView> {
            val card = card()
            card.item(text(arrow, 20f, mint))
            val number = text("0 B", 25f, ink, true); card.item(number, 12)
            card.item(text(label, 12f, muted), 4)
            return card to number
        }
        val (downCard, downValue) = metric("To laptop", "↓"); download = downValue
        val (upCard, upValue) = metric("From laptop", "↑"); upload = upValue
        metrics.addView(downCard, LinearLayout.LayoutParams(0, -1, 1f).apply { marginEnd = dp(6) })
        metrics.addView(upCard, LinearLayout.LayoutParams(0, -1, 1f).apply { marginStart = dp(6) })
        layout.item(metrics, 12)

        val preferences = getSharedPreferences("routing", MODE_PRIVATE)
        val settings = card()
        settings.item(text("Connection", 19f, ink, true))
        settings.item(text("Settings unlock when sharing is stopped.", 12f, muted), 4)
        settings.item(text("INTERNET SOURCE", 10f, muted, true).apply { letterSpacing = .08f }, 24)
        val modes = arrayOf("Phone’s current connection", "Local SOCKS5 proxy", "Local HTTP CONNECT proxy")
        val keys = arrayOf("default", "socks5", "http")
        method = Spinner(this).apply {
            adapter = object : ArrayAdapter<String>(this@MainActivity, android.R.layout.simple_spinner_item, modes) {
                override fun getView(position: Int, convertView: View?, parent: ViewGroup): View =
                    (super.getView(position, convertView, parent) as TextView).apply { setTextColor(ink); textSize = 15f; setPadding(dp(12), dp(16), dp(32), dp(16)); isSingleLine = false }
                override fun getDropDownView(position: Int, convertView: View?, parent: ViewGroup): View =
                    (super.getDropDownView(position, convertView, parent) as TextView).apply { setTextColor(ink); setBackgroundColor(surface); textSize = 15f; setPadding(dp(16), dp(18), dp(16), dp(18)); isSingleLine = false }
            }
            backgroundTintList = ColorStateList.valueOf(muted)
            contentDescription = "Internet source"
            setSelection(keys.indexOf(preferences.getString("mode", "default")).coerceAtLeast(0))
        }
        settings.item(method, 8)
        val proxyFields = column()
        proxyFields.item(text("PROXY PORT · 127.0.0.1", 10f, muted, true), 16)
        proxyPort = EditText(this).apply {
            inputType = android.text.InputType.TYPE_CLASS_NUMBER
            setText(preferences.getInt("proxy_port", 1080).toString()); setTextColor(ink); textSize = 16f
            contentDescription = "Local proxy port"; setSingleLine(true)
            backgroundTintList = ColorStateList.valueOf(mint)
        }; proxyFields.item(proxyPort, 4)
        proxyFields.item(text("TCP only. Your proxy must accept this app on loopback without credentials. Strict VPN sharing is unavailable in proxy mode.", 12f, muted), 8)
        settings.item(proxyFields)
        method.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) { proxyFields.visibility = if (position == 0) View.GONE else View.VISIBLE }
            override fun onNothingSelected(parent: AdapterView<*>?) {}
        }
        settings.addView(View(this).apply { setBackgroundColor(border) }, LinearLayout.LayoutParams(-1, dp(1)).apply { topMargin = dp(20) })
        wifi = Switch(this).apply {
            text = "Wi-Fi sharing"; textSize = 15f; setTextColor(ink)
            setPadding(0, dp(16), 0, dp(12)); minHeight = dp(52)
            isChecked = preferences.getBoolean("wifi", false)
            val states = arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf())
            thumbTintList = ColorStateList(states, intArrayOf(mint, muted))
            trackTintList = ColorStateList(states, intArrayOf(Color.rgb(63, 110, 88), border))
        }; settings.item(wifi)
        settings.item(text("Optional encrypted access on the same local network. USB sharing is always available while the service runs.", 12f, muted), 2)
        layout.item(settings, 20)

        val help = card()
        val toggle = button("How to connect  +")
        val steps = column().apply { visibility = View.GONE }
        val instructions = arrayOf("Plug your phone into the laptop with a USB data cable.", "Enable USB debugging and approve your laptop.", "Start sharing here, then select this phone in Tetherust on Linux. Pair once and connect.")
        instructions.forEachIndexed { index, value ->
            val row = LinearLayout(this).apply { gravity = Gravity.TOP }
            row.addView(text("${index + 1}", 14f, mint, true).apply { gravity = Gravity.CENTER; background = shape(Color.rgb(35, 54, 45), 10) }, LinearLayout.LayoutParams(dp(32), dp(32)))
            row.addView(text(value, 13f, muted), LinearLayout.LayoutParams(0, -2, 1f).apply { marginStart = dp(12) })
            steps.item(row, 16)
        }
        steps.item(text("Using a VPN? Include Tetherust in its app list. VPN routing depends on that app’s rules; check the Linux verification status.", 13f, muted), 20)
        toggle.setOnClickListener { val show = steps.visibility != View.VISIBLE; steps.visibility = if (show) View.VISIBLE else View.GONE; toggle.text = if (show) "How to connect  −" else "How to connect  +" }
        help.item(toggle); help.item(steps); layout.item(help, 12)
        val reset = button("Forget paired laptops")
        reset.setOnClickListener {
            AlertDialog.Builder(this).setTitle("Forget paired laptops?")
                .setMessage("Stops sharing and changes your phone’s identity. You’ll need to pair again on each laptop.")
                .setNegativeButton("Cancel", null).setPositiveButton("Forget") { _, _ ->
                    stopService(Intent(this, SharingService::class.java)); Identity.reset(this)
                    SharingState.status = "Pairing reset. Start sharing, then pair again on your laptop."
                    updateState()
                }.show()
        }
        layout.item(reset, 20)
        layout.item(text("Your VPN stays on your phone.\nOnly approve USB debugging for laptops you trust.", 12f, muted).apply { gravity = Gravity.CENTER }, 20)
        action.setOnClickListener {
            if (SharingState.running) stopService(Intent(this, SharingService::class.java))
            else {
                val port = proxyPort.text.toString().toIntOrNull()
                if (method.selectedItemPosition != 0 && (port == null || port !in 1..65535)) {
                    proxyPort.error = "Use a port from 1 to 65535"; proxyPort.requestFocus(); return@setOnClickListener
                }
                preferences.edit().putBoolean("wifi", wifi.isChecked).putString("mode", keys[method.selectedItemPosition]).putInt("proxy_port", port ?: 1080).apply()
                if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED)
                    requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), 1)
                val intent = Intent(this, SharingService::class.java)
                try { if (Build.VERSION.SDK_INT >= 26) startForegroundService(intent) else startService(intent) }
                catch (_: Exception) { SharingState.status = "Android did not allow sharing to start. Keep this app open and retry." }
            }
            handler.post { updateState() }
        }
        setContentView(scroll)
        updateState()
    }
    override fun onResume() { super.onResume(); handler.removeCallbacks(refresh); handler.post(refresh) }
    override fun onPause() { handler.removeCallbacks(refresh); super.onPause() }
}
