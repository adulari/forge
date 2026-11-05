package dev.adulari.forge.spike

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.adulari.forge.core.ChatItem
import dev.adulari.forge.core.ConnectionState
import dev.adulari.forge.core.Role

@Composable
fun SessionScreen(vm: AppViewModel, id: String, onBack: () -> Unit) {
    LaunchedEffect(id) { vm.openSession(id) }
    DisposableEffect(id) { onDispose { vm.close() } }
    val upd = vm.update
    Column(Modifier.fillMaxSize().imePadding()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("‹", color = T.ink, fontSize = 28.sp, modifier = Modifier.clickable(onClick = onBack).padding(horizontal = 12.dp)
                .semantics { contentDescription = "Back" })
            Column(Modifier.weight(1f)) {
                Text(upd?.title?.ifEmpty { null } ?: "session", color = T.ink, fontSize = 16.sp, maxLines = 1)
                Text(when {
                    vm.connection != ConnectionState.OPEN -> "connecting…"
                    upd?.busy == true -> upd.activity.ifEmpty { "forging" }
                    else -> "idle"
                }, color = T.ink3, fontSize = 11.sp, fontFamily = FontFamily.Monospace)
            }
            Box(Modifier.size(8.dp).background(if (upd?.busy == true) T.accent else T.ink4, CircleShape))
            Spacer(Modifier.width(12.dp))
        }
        val rows = remember(vm.items.size) { vm.items.toList() }
        // Inverted list (newest at the bottom), same shape as the RN transcript FlatList.
        LazyColumn(
            reverseLayout = true,
            modifier = Modifier.weight(1f).fillMaxWidth().semantics { contentDescription = "transcript" },
            contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
        ) {
            val live = upd?.streamingBlocks.orEmpty()
            // Newest first (reverseLayout): the live reply's blocks, then settled messages.
            itemsIndexed(live.asReversed(), key = { i, _ -> "live:${live.size - 1 - i}" }) { _, b ->
                Box(Modifier.padding(top = 8.dp)) { BlockItem(b) }
            }
            for (m in rows.asReversed()) {
                if (m.role == Role.ASSISTANT) {
                    itemsIndexed(m.blocks.asReversed(), key = { i, _ -> "${m.seq}:${m.blocks.size - 1 - i}" }) { i, b ->
                        Box(Modifier.padding(top = if (i == m.blocks.size - 1) 14.dp else 8.dp)) { BlockItem(b) }
                    }
                } else {
                    item(key = m.seq) { Box(Modifier.padding(top = 14.dp)) { Message(m) } }
                }
            }
        }
        Composer(onSend = vm::send)
    }
}

@Composable
private fun Message(m: ChatItem) {
    when (m.role) {
        Role.USER -> Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
            Column(Modifier.widthIn(max = 300.dp).background(T.bg3, RoundedCornerShape(14.dp)).padding(horizontal = 12.dp, vertical = 8.dp)) {
                MarkdownBlocks(m.blocks)
            }
        }
        Role.ASSISTANT -> MarkdownBlocks(m.blocks)
        else -> Column(Modifier.fillMaxWidth().background(T.bg2, RoundedCornerShape(8.dp)).padding(8.dp)) {
            Text(m.tool ?: "tool", color = T.ink3, fontSize = 11.sp, fontFamily = FontFamily.Monospace)
        }
    }
}

@Composable
private fun Composer(onSend: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    Row(Modifier.fillMaxWidth().background(T.bg1).padding(10.dp).navigationBarsPadding(), verticalAlignment = Alignment.CenterVertically) {
        BasicTextField(
            value = text, onValueChange = { text = it },
            textStyle = TextStyle(color = T.ink, fontSize = 15.sp), cursorBrush = SolidColor(T.accent),
            modifier = Modifier.weight(1f).background(T.bg3, RoundedCornerShape(20.dp)).padding(horizontal = 14.dp, vertical = 10.dp)
                .semantics { contentDescription = "composer" },
            decorationBox = { inner -> if (text.isEmpty()) Text("Message", color = T.ink3, fontSize = 15.sp); inner() },
        )
        Spacer(Modifier.width(8.dp))
        Box(Modifier.size(40.dp).background(if (text.isBlank()) T.bg3 else T.accent, CircleShape)
            .clickable(enabled = text.isNotBlank()) { onSend(text.trim()); text = "" }
            .semantics { contentDescription = "send" }, contentAlignment = Alignment.Center) {
            Text("↑", color = if (text.isBlank()) T.ink3 else T.onAccent, fontSize = 18.sp)
        }
    }
}
