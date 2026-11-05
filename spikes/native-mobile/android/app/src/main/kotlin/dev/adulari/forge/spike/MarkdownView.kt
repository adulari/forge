package dev.adulari.forge.spike

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.adulari.forge.core.MdBlock
import dev.adulari.forge.core.MdSpan

private fun spans(list: List<MdSpan>, base: Color): AnnotatedString = buildAnnotatedString {
    for (s in list) {
        val st = SpanStyle(
            color = if (s.link != null) T.accent else base,
            fontWeight = if (s.bold) FontWeight.SemiBold else null,
            fontStyle = if (s.italic) FontStyle.Italic else null,
            fontFamily = if (s.code) FontFamily.Monospace else null,
            background = if (s.code) T.bg3 else Color.Unspecified,
            textDecoration = if (s.link != null) TextDecoration.Underline else null,
        )
        withStyle(st) { append(s.text) }
    }
}

/** uniffi records live in another module, so Compose treats them as unstable; this wrapper lets
 * unchanged blocks of a streaming reply skip recomposition (equality is structural). */
@androidx.compose.runtime.Immutable
private class Blk(val b: MdBlock) {
    override fun equals(other: Any?) = other is Blk && other.b == b
    override fun hashCode() = b.hashCode()
}

@Composable
fun MarkdownBlocks(blocks: List<MdBlock>, color: Color = T.ink) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        blocks.forEachIndexed { i, b -> key(i) { BlockView(Blk(b), color) } }
    }
}

/** One markdown block as its own lazy-list item: keeps per-item compose/measure cost small. */
@Composable
fun BlockItem(b: MdBlock, color: Color = T.ink) = BlockView(Blk(b), color)

@Composable
private fun BlockView(blk: Blk, color: Color) {
    when (val b = blk.b) {
        is MdBlock.Paragraph -> Text(remember(blk) { spans(b.spans, color) }, fontSize = 15.sp, lineHeight = 22.sp)
        is MdBlock.Heading -> Text(remember(blk) { spans(b.spans, T.ink) },
            fontSize = when (b.level.toInt()) { 1 -> 20.sp; 2 -> 17.sp; else -> 15.sp }, fontWeight = FontWeight.SemiBold)
        is MdBlock.ListItem -> Row(Modifier.padding(start = (b.depth.toInt() * 14).dp)) {
            Text(b.marker + " ", color = T.ink3, fontSize = 15.sp, lineHeight = 22.sp)
            Text(remember(blk) { spans(b.spans, color) }, fontSize = 15.sp, lineHeight = 22.sp)
        }
        is MdBlock.Quote -> Row(Modifier.height(IntrinsicSize.Min)) {
            Box(Modifier.width(2.dp).fillMaxHeight().background(T.borderStrong))
            Text(remember(blk) { spans(b.spans, T.ink2) }, fontSize = 15.sp, lineHeight = 22.sp, modifier = Modifier.padding(start = 10.dp))
        }
        is MdBlock.Code -> Column(Modifier.fillMaxWidth().background(T.bg3, RoundedCornerShape(8.dp)).padding(10.dp)) {
            if (b.lang.isNotEmpty()) Text(b.lang, color = T.ink3, fontSize = 10.sp, fontFamily = FontFamily.Monospace)
            Text(b.code, color = T.ink, fontSize = 12.sp, lineHeight = 17.sp, fontFamily = FontFamily.Monospace,
                softWrap = false, modifier = Modifier.horizontalScroll(rememberScrollState()))
        }
        MdBlock.Rule -> Box(Modifier.fillMaxWidth().height(1.dp).background(T.border))
    }
}
