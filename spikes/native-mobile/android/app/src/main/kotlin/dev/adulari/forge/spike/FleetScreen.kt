package dev.adulari.forge.spike

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.adulari.forge.core.FleetEntry
import dev.adulari.forge.core.SessionRow
import kotlinx.coroutines.delay

@Composable
fun FleetScreen(vm: AppViewModel, onOpen: (String) -> Unit) {
    LaunchedEffect(Unit) { while (true) { vm.refreshFleet(); delay(5000) } }
    Column(Modifier.fillMaxSize()) {
        Text("Fleet", color = T.ink, fontSize = 22.sp, fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(start = 16.dp, top = 12.dp, bottom = 8.dp))
        vm.fleetError?.let { Text(it, color = T.danger, fontSize = 12.sp, modifier = Modifier.padding(16.dp)) }
        LazyColumn(
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 4.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.fillMaxSize().semantics { contentDescription = "fleet-list" },
        ) {
            items(vm.fleet, key = { when (it) { is FleetEntry.Label -> "l:" + it.text; is FleetEntry.Session -> it.row.id } }) { e ->
                when (e) {
                    is FleetEntry.Label -> Text(e.text, color = T.ink3, fontSize = 11.sp, fontFamily = FontFamily.Monospace,
                        letterSpacing = 1.2.sp, modifier = Modifier.padding(start = 4.dp, top = 10.dp, bottom = 2.dp))
                    is FleetEntry.Session -> SessionCard(e.row) { onOpen(e.row.id) }
                }
            }
        }
    }
}

@Composable
private fun SessionCard(row: SessionRow, onClick: () -> Unit) {
    val state = if (row.waiting) "waiting" else if (row.busy) "busy" else "idle"
    val dot = when (state) { "waiting" -> T.danger; "busy" -> T.accent; else -> T.ink4 }
    val bg = if (row.waiting) T.selection else T.bg2
    val project = row.cwd.trimEnd('/').substringAfterLast('/')
    Column(
        Modifier.fillMaxWidth().background(bg, RoundedCornerShape(12.dp))
            .border(BorderStroke(1.dp, T.border), RoundedCornerShape(12.dp))
            .clickable(onClick = onClick)
            .semantics { contentDescription = "${row.title}, $state" }
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.size(if (state == "idle") 7.dp else 8.dp).background(dot, CircleShape))
            Text(row.title.ifEmpty { "session ${row.id.take(8)}" }, color = if (state == "idle") T.ink2 else T.ink,
                fontSize = if (state == "idle") 13.5.sp else 14.5.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f))
            if (row.lastTurnOutcome == "failed") Text("failed", color = T.danger, fontSize = 10.sp)
            Text(if (state == "waiting") "now" else if (state == "busy") "running" else "${row.costUsd.format()}",
                color = if (state == "waiting") T.danger else T.ink3, fontSize = 11.sp, fontFamily = FontFamily.Monospace)
        }
        Text(if (row.waiting) "needs a decision" else "local · ${row.model.substringAfter("::")}",
            color = if (state == "waiting") T.ink2 else T.ink4, fontSize = 11.sp, fontFamily = FontFamily.Monospace, maxLines = 1)
        if (!row.waiting) Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(project, color = T.ink3, fontSize = 11.sp, fontFamily = FontFamily.Monospace, maxLines = 1)
            row.branch?.let { Text("⑂ $it", color = T.ink3, fontSize = 11.sp, fontFamily = FontFamily.Monospace, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            if (row.diffAdded != null) Text("+${row.diffAdded} −${row.diffRemoved ?: 0}", color = T.success, fontSize = 11.sp, fontFamily = FontFamily.Monospace)
        }
    }
}

private fun Double.format() = "$" + String.format("%.2f", this)
