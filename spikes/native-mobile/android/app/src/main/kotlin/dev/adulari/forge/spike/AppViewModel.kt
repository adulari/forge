package dev.adulari.forge.spike

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.adulari.forge.core.*
import kotlinx.coroutines.launch

class AppViewModel(base: String) : ViewModel() {
    private val client = DaemonClient(base)

    var fleet by mutableStateOf<List<FleetEntry>>(emptyList())
    var fleetError by mutableStateOf<String?>(null)

    val items = mutableStateListOf<ChatItem>()
    var update by mutableStateOf<SessionUpdate?>(null)
    var connection by mutableStateOf(ConnectionState.CONNECTING)
    var loadedMs by mutableStateOf(-1L)
    private var handle: SessionHandle? = null
    private var openId: String? = null

    fun refreshFleet() = viewModelScope.launch {
        try { fleet = client.fleet(); fleetError = null } catch (e: Exception) { fleetError = e.message }
    }

    fun openSession(id: String) {
        if (openId == id) return
        close()
        openId = id
        items.clear(); update = null
        val h = client.session(id)
        handle = h
        viewModelScope.launch {
            val t0 = System.nanoTime()
            val rows = h.loadHistory(1000u)
            items.addAll(rows)
            loadedMs = (System.nanoTime() - t0) / 1_000_000
            android.util.Log.i("ForgeSpike", "history loaded: ${rows.size} rows in ${loadedMs}ms")
            h.startStream(object : SessionListener {
                override fun onUpdate(update: SessionUpdate) { this@AppViewModel.update = update }
                override fun onItems(items: List<ChatItem>) {
                    this@AppViewModel.update = this@AppViewModel.update?.copy(streamingBlocks = emptyList(), streamingChars = 0u)
                    this@AppViewModel.items.removeAll { it.seq < 0 }
                    this@AppViewModel.items.addAll(items)
                }
                override fun onConnection(state: ConnectionState) { connection = state }
            })
        }
    }

    fun send(text: String) {
        // Optimistic bubble (negative seq) until the daemon's own row arrives via onItems.
        items.add(ChatItem(-(items.size.toLong() + 1), Role.USER, null, parseMarkdown(text)))
        handle?.sendPrompt(text)
    }

    fun close() {
        handle?.stopStream(); handle = null; openId = null
    }

    override fun onCleared() = close()

    companion object {
        fun factory(base: String) = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T = AppViewModel(base) as T
        }
    }
}
